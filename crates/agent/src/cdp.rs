//! Steam CEF DevTools client (127.0.0.1:8080): attaches to `SharedJSContext`, injects `shim.js` on every `executionContextCreated` (UI reload)
//! Receives `{topic, data}` through `__framemateEmit`. Reconnects when Steam restarts. Carries `/api/notify` requests to shim's hook (notify.rs).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::devices::DeviceMemory;
use crate::hub::{Hub, now_ms};
use crate::notify::{self, Outcome};

const TARGET_TITLE: &str = "SharedJSContext";
const BINDING: &str = "__framemateEmit";
const SHIM: &str = include_str!("shim.js");
/// Fallback icon for toasts in the headset
const LOGO_SVG: &str = include_str!("../../../assets/framemate-black.svg");
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// after this much silence, ping once, then reconnect
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn run(hub: Arc<Hub>, cdp_url: String, mut toasts: mpsc::Receiver<notify::Request>) {
    let authority = cdp_url
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned();
    let mut backoff = Duration::from_secs(1);
    let mut devices = DeviceMemory::load();
    loop {
        let started = Instant::now();
        let error = match session(&hub, &authority, &mut devices, &mut toasts).await {
            Ok(()) => "connection closed".to_owned(),
            Err(e) => format!("{e:#}"),
        };
        tracing::warn!("steam CDP: {error}");
        hub.update(|s| {
            s.steam.connected = false;
            s.steam.error = Some(error);
        });
        if started.elapsed() > MAX_BACKOFF {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn session(
    hub: &Hub,
    authority: &str,
    devices: &mut DeviceMemory,
    toasts: &mut mpsc::Receiver<notify::Request>,
) -> anyhow::Result<()> {
    let ws_url = find_target(authority).await?;
    let (ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .with_context(|| format!("connecting to {ws_url}"))?;
    let (mut tx, mut rx) = ws.split();
    let mut next_id = 0u64;
    let mut call = |method: &str, params: Value| {
        next_id += 1;
        let msg = json!({"id": next_id, "method": method, "params": params});
        (next_id, Message::Text(msg.to_string().into()))
    };
    // Toast requests waiting for Steam's answer
    let mut pending: HashMap<u64, oneshot::Sender<Result<Outcome, String>>> = HashMap::new();

    tx.send(call("Runtime.addBinding", json!({"name": BINDING})).1).await?;
    // Emits executionContextCreated for the existing context, which triggers injection.
    tx.send(call("Runtime.enable", json!({})).1).await?;
    tracing::info!("steam CDP: attached to {TARGET_TITLE}");
    hub.update(|s| {
        s.steam.connected = true;
        s.steam.error = None;
    });

    let mut pinged = false;
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(IDLE_TIMEOUT, rx.next()) => next,
            Some(request) = toasts.recv() => {
                // Queued while Steam was away; showing it now would be confusingly late.
                if Instant::now() >= request.deadline {
                    continue;
                }
                let expression = format!(
                    "(() => {{ const notify = window.__framemate?.notify; return notify ? notify({}) \
                     : {{ unavailable: \"the toast hook isn't installed (state.steam.topic_errors.toasts)\" }}; }})()",
                    request.args
                );
                let (id, msg) = call("Runtime.evaluate", json!({"expression": expression, "returnByValue": true}));
                tx.send(msg).await?;
                pending.insert(id, request.reply);
                continue;
            }
        };
        let msg = match next {
            Ok(Some(msg)) => msg,
            Ok(None) => break,
            Err(_) if pinged => bail!("Steam stopped responding"),
            Err(_) => {
                pinged = true;
                tx.send(call("Runtime.evaluate", json!({"expression": "1"})).1).await?;
                continue;
            }
        };
        pinged = false;
        let text = match msg? {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
        let msg: Value = serde_json::from_str(&text)?;
        let params = &msg["params"];
        match msg["method"].as_str() {
            Some("Runtime.executionContextCreated") => {
                let context = &params["context"];
                if context["auxData"]["isDefault"].as_bool() == Some(true) {
                    tracing::info!("steam CDP: injecting shim into context {}", context["id"]);
                    let shim = SHIM.replace("\"__FRAMEMATE_LOGO_SVG__\"", &serde_json::to_string(LOGO_SVG)?);
                    let params = json!({"expression": shim, "contextId": context["id"]});
                    tx.send(call("Runtime.evaluate", params).1).await?;
                }
            }
            Some("Runtime.executionContextsCleared") => {
                hub.update(|s| s.steam.topics.clear());
            }
            Some("Runtime.bindingCalled") if params["name"] == BINDING => {
                if let Some(payload) = params["payload"].as_str() {
                    handle_emit(hub, payload, devices);
                }
            }
            Some(_) => {}
            None => {
                if let Some(reply) = msg["id"].as_u64().and_then(|id| pending.remove(&id)) {
                    // Only the outcome is logged, never the content.
                    let result = if msg["result"].get("exceptionDetails").is_some() || msg.get("error").is_some() {
                        Err("the toast hook threw inside Steam".to_owned())
                    } else {
                        notify::outcome(&msg["result"]["result"]["value"])
                    };
                    tracing::debug!("toast: {result:?}");
                    let _ = reply.send(result);
                } else if let Some(exception) = msg["result"].get("exceptionDetails") {
                    tracing::warn!("steam CDP: shim threw: {exception}");
                } else if let Some(error) = msg.get("error") {
                    tracing::warn!("steam CDP: call {} failed: {error}", msg["id"]);
                }
            }
        }
    }
    Ok(())
}

fn handle_emit(hub: &Hub, payload: &str, devices: &mut DeviceMemory) {
    let Ok(mut msg) = serde_json::from_str::<Value>(payload) else {
        tracing::warn!("steam CDP: malformed emit payload");
        return;
    };
    let Some(topic) = msg["topic"].as_str().map(str::to_owned) else {
        return;
    };
    let mut data = msg["data"].take();
    if topic == "vr_devices" {
        data = devices.merge(data);
    }
    hub.update(|s| {
        s.steam.last_event_ms = Some(now_ms());
        if topic == "error" {
            let (Some(t), Some(m)) = (data["topic"].as_str(), data["message"].as_str()) else {
                return;
            };
            tracing::debug!("shim error in {t}: {m}");
            s.steam.topic_errors.insert(t.to_owned(), m.to_owned());
        } else {
            s.steam.topic_errors.remove(&topic);
            s.steam.topics.insert(topic, data);
        }
    });
}

async fn find_target(authority: &str) -> anyhow::Result<String> {
    let body = http_get(authority, "/json").await?;
    let targets: Vec<Value> = serde_json::from_slice(&body).context("parsing /json")?;
    let target = targets
        .iter()
        .find(|t| t["title"] == TARGET_TITLE)
        .with_context(|| format!("no {TARGET_TITLE} target (is Steam running?)"))?;
    let url = target["webSocketDebuggerUrl"]
        .as_str()
        .context("target has no webSocketDebuggerUrl")?;
    // CEF reports its own bind address; keep the configured one so a remote
    // FRAMEMATE_CDP (e.g. the devkit forward on :8081) works during development.
    let path = &url[url.find("/devtools/").context("unexpected debugger URL")?..];
    Ok(format!("ws://{authority}{path}"))
}

/// Minimal HTTP/1.1 GET (no HTTP client dependency). CEF ignores HTTP/1.0 requests
pub(crate) async fn http_get(authority: &str, path: &str) -> anyhow::Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = TcpStream::connect(authority)
            .await
            .with_context(|| format!("connecting to {authority}"))?;
        let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        let header_end = loop {
            if let Some(pos) = response.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                bail!("connection closed before HTTP headers");
            }
            response.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&response[..header_end]).into_owned();
        if !head.starts_with("HTTP/1.1 200") {
            // Drop the query: it may carry the token.
            let path = path.split('?').next().unwrap_or_default();
            bail!("GET {path}: {}", head.lines().next().unwrap_or_default());
        }
        let length: usize = head
            .lines()
            .find_map(|l| {
                let (name, value) = l.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().ok())?
            })
            .context("response has no Content-Length")?;
        while response.len() < header_end + length {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                bail!("connection closed mid-body");
            }
            response.extend_from_slice(&chunk[..n]);
        }
        Ok(response[header_end..header_end + length].to_vec())
    })
    .await
    .context("timeout")?
}
