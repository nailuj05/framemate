//! HTTP API for the app. Served twice: plaintext on `listen`, and pinned TLS on `listen_tls`
//! (see tls.rs). Same router both times, so every route and the token check are identical.
//!
//! - `GET /api/state`   full state as JSON
//! - `GET /api/ws`      full state as JSON on connect and after every change (throttled)
//! - `GET /api/stream/ws` headset view: JSON `{codec}`, fMP4 init segment, then one
//!   moof+mdat per frame (see stream.rs, fmp4.rs)
//! - `GET /healthz`     liveness, the only route without auth
//!
//! `/api/*` requires the token via `?token=` or `Authorization: Bearer`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::extract::connect_info::Connected;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::serve::IncomingStream;
use socket2::{Domain, Protocol, Socket, Type};
use tokio_rustls::TlsAcceptor;

use crate::config::Config;
use crate::fmp4;
use crate::hub::Hub;
use crate::stream::LiveStream;
use crate::tls::Identity;

/// Coalesces bursts (download progress fires every second) into one push.
const PUSH_THROTTLE: Duration = Duration::from_millis(250);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Completed handshakes waiting for `axum::serve` to pick them up.
const HANDSHAKE_QUEUE: usize = 64;

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
    token: Arc<str>,
    stream: Arc<LiveStream>,
}

pub async fn serve(hub: Arc<Hub>, stream: Arc<LiveStream>, config: &Config) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/state", get(state))
        .route("/api/ws", get(ws))
        .route("/api/stream/ws", get(stream_ws))
        .route("/healthz", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(config.allow_remote, crate::access::local_only))
        .with_state(AppState {
            hub,
            token: config.token.as_str().into(),
            stream,
        });

    let plain = bind(config.listen)?;
    // Fatal rather than degrading to plaintext only: an agent the app silently can't reach is a
    // worse support case than one that fails loudly with the address in the message.
    let tls = TlsListener::spawn(
        bind(config.listen_tls)?,
        TlsAcceptor::from(Identity::load_or_create()?.server_config()?),
    )?;
    tracing::info!("listening on {} and {} (TLS)", config.listen, config.listen_tls);
    // A prompt for whoever is sitting at the terminal; pointless in journald.
    // SAFETY: isatty only inspects the descriptor.
    if unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1 {
        println!("Run `framemate-agent pair` for the code to scan in the app.");
    }
    // Two signal registrations of the same kind; tokio delivers to all of them.
    tokio::try_join!(
        axum::serve(plain, app.clone().into_make_service_with_connect_info::<Peer>())
            .with_graceful_shutdown(shutdown_signal()),
        axum::serve(tls, app.into_make_service_with_connect_info::<Peer>())
            .with_graceful_shutdown(shutdown_signal()),
    )?;
    Ok(())
}

/// Terminates TLS so `axum::serve` keeps handling graceful shutdown, `ConnectInfo` and the
///
/// Handshakes deliberately do *not* happen in `accept`: awaiting one there is serial, so a
/// client that connects and then sends nothing would block every later connection and take the
/// whole TLS port down. They run on their own tasks and queue up here instead.
struct TlsListener {
    local: SocketAddr,
    ready: tokio::sync::mpsc::Receiver<(tokio_rustls::server::TlsStream<tokio::net::TcpStream>, SocketAddr)>,
}

impl TlsListener {
    fn spawn(mut tcp: tokio::net::TcpListener, acceptor: TlsAcceptor) -> anyhow::Result<Self> {
        let local = tcp.local_addr()?;
        let (tx, ready) = tokio::sync::mpsc::channel(HANDSHAKE_QUEUE);
        tokio::spawn(async move {
            loop {
                // Delegating keeps axum's own policy for accept errors (it backs off on EMFILE).
                let (stream, peer) = axum::serve::Listener::accept(&mut tcp).await;
                let (acceptor, tx) = (acceptor.clone(), tx.clone());
                // One task each, with no cap on how many run at once: capping them would mean
                // queueing, and half-open connections would starve real ones all over again.
                // What bounds this is the timeout above and the process's file descriptor limit.
                tokio::spawn(async move {
                    match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                        Ok(Ok(tls)) => {
                            let _ = tx.send((tls, peer)).await;
                        }
                        // A scanner, or plain HTTP to the TLS port. Drop it and keep serving.
                        Ok(Err(e)) => tracing::debug!("{peer}: TLS handshake failed: {e}"),
                        Err(_) => tracing::debug!("{peer}: TLS handshake timed out"),
                    }
                });
            }
        });
        Ok(Self { local, ready })
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(ready) => ready,
            // The accept task runs for the life of the process; only reachable if it panicked,
            // and `accept` has no way to report that, so stop handing out connections.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// The peer address access.rs checks. Crate-local because axum ships `Connected` only for its
/// own `TcpListener`, and the orphan rule rejects an impl for `SocketAddr`: `TlsListener`
/// appears only as a nested parameter, which doesn't make the impl local.
#[derive(Clone, Copy)]
pub struct Peer(pub SocketAddr);

impl Connected<IncomingStream<'_, tokio::net::TcpListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, tokio::net::TcpListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

impl Connected<IncomingStream<'_, TlsListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

fn bind(addr: SocketAddr) -> anyhow::Result<tokio::net::TcpListener> {
    match listen(addr) {
        // IPv6 can be disabled (ipv6.disable=1); keep serving IPv4 then.
        Err(e) if addr.is_ipv6() && addr.ip().is_unspecified() => {
            tracing::warn!("{e:#}; falling back to IPv4 only");
            listen(SocketAddr::from(([0, 0, 0, 0], addr.port())))
        }
        result => result,
    }
}

/// Dual-stack for an IPv6 wildcard (`frame.local` often resolves to IPv6). `IPV6_V6ONLY` has to be cleared before `bind`, which `TcpListener::bind` can't do.
fn listen(addr: SocketAddr) -> anyhow::Result<tokio::net::TcpListener> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))
        .context("creating the listening socket")?;
    if addr.is_ipv6() {
        socket.set_only_v6(false).context("clearing IPV6_V6ONLY")?;
    }
    // A restart must not fail while the previous socket lingers.
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into()).with_context(|| format!("binding {addr}"))?;
    socket.listen(1024)?;
    Ok(tokio::net::TcpListener::from_std(socket.into())?)
}

fn authorized(app: &AppState, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    bearer
        .or(query.get("token").map(String::as_str))
        .is_some_and(|token| crate::config::normalize_token(token) == *app.token)
}

async fn state(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    // CORS so the app can read the 401 (wrong token vs. unreachable).
    let cors = [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")];
    if !authorized(&app, &headers, &query) {
        return (StatusCode::UNAUTHORIZED, cors).into_response();
    }
    (cors, [(header::CONTENT_TYPE, "application/json")], app.hub.snapshot_json()).into_response()
}

async fn ws(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&app, &headers, &query) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    upgrade.on_upgrade(move |socket| push_state(socket, app.hub))
}

async fn push_state(mut socket: WebSocket, hub: Arc<Hub>) {
    let mut changes = hub.subscribe();
    changes.mark_unchanged();
    loop {
        if socket.send(Message::Text(hub.snapshot_json().into())).await.is_err() {
            return;
        }
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                tokio::time::sleep(PUSH_THROTTLE).await;
                changes.mark_unchanged();
            }
            incoming = socket.recv() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn stream_ws(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&app, &headers, &query) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    upgrade.on_upgrade(move |socket| push_stream(socket, app.stream))
}

/// Remuxes the shared H.264 stream into fMP4 for one viewer. Starts at a keyframe;
/// if the viewer falls behind it skips ahead to the next keyframe.
async fn push_stream(mut socket: WebSocket, stream: Arc<LiveStream>) {
    use tokio::sync::broadcast::error::RecvError;

    let frame_duration = fmp4::TIMESCALE / stream.fps();
    let mut packets = stream.subscribe();
    let (mut initialized, mut synced) = (false, false);
    let (mut sequence, mut decode_time) = (1u32, 0u64);
    loop {
        let packet = tokio::select! {
            packet = packets.recv() => match packet {
                Ok(packet) => packet,
                Err(RecvError::Lagged(_)) => {
                    synced = false;
                    stream.request_keyframe();
                    continue;
                }
                Err(RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => continue,
            },
        };
        if !synced {
            if !packet.keyframe {
                continue;
            }
            synced = true;
        }
        if !initialized {
            let Some((sps, pps)) = fmp4::parameter_sets(&packet.data) else {
                tracing::warn!("stream: keyframe without SPS/PPS");
                continue;
            };
            let header = serde_json::json!({ "codec": fmp4::codec_string(sps) }).to_string();
            let init = fmp4::init_segment(packet.width, packet.height, sps, pps);
            if socket.send(Message::Text(header.into())).await.is_err()
                || socket.send(Message::Binary(init.into())).await.is_err()
            {
                return;
            }
            initialized = true;
        }
        let segment = fmp4::media_segment(sequence, decode_time, frame_duration, packet.keyframe, &fmp4::to_sample(&packet.data));
        sequence += 1;
        decode_time += u64::from(frame_duration);
        if socket.send(Message::Binary(segment.into())).await.is_err() {
            return;
        }
    }
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutting down");
}
