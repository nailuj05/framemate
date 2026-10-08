// Needs the GTK/WebKit stack, since the crate under test is the Tauri lib. Verified
// out-of-tree against the same sources where that stack isn't installed.
use std::sync::{Arc, Mutex};

use framemate_app_lib::pairing::Pairing;
use framemate_app_lib::proxy;
use rcgen::PublicKeyData;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BODY: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok";

fn base32(bytes: &[u8]) -> String {
    const A: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let (mut acc, mut bits, mut out) = (0u32, 0u32, String::new());
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(A[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(A[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// A stand-in for the agent: TLS, replies to one request, records what it received.
async fn agent() -> (u16, String, Arc<Mutex<Vec<String>>>) {
    let issued = rcgen::generate_simple_self_signed(vec!["frame.local".to_string()]).unwrap();
    let digest = ring::digest::digest(&ring::digest::SHA256, &issued.signing_key.subject_public_key_info());
    let pin = base32(&digest.as_ref()[..16]);

    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(issued.signing_key.serialize_der()));
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![issued.cert.der().clone()], key)
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (acceptor, recorded) = (acceptor.clone(), recorded.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(stream).await else { return };
                let mut buf = [0u8; 512];
                if let Ok(n) = tls.read(&mut buf).await {
                    recorded.lock().unwrap().push(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                let _ = tls.write_all(BODY).await;
                let _ = tls.flush().await;
            });
        }
    });
    (port, pin, seen)
}

async fn request(port: u16, line: &str) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket.write_all(format!("{line}\r\n\r\n").as_bytes()).await.unwrap();
    let mut out = String::new();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), socket.read_to_string(&mut out)).await;
    out
}

fn pairing(port: u16, pin: &str) -> Pairing {
    Pairing { host: Some("127.0.0.1".into()), ip: None, port, token: "ABCDE-FGHJK".into(), pin: pin.into() }
}

#[tokio::test]
async fn relays_when_the_pin_and_secret_match() {
    let (agent_port, pin, seen) = agent().await;
    let current = Arc::new(Mutex::new(Some(proxy::Target::new(pairing(agent_port, &pin)).unwrap())));
    let status = Arc::new(proxy::Status::default());
    let p = proxy::spawn(current, status.clone()).await.unwrap();

    let got = request(p.port, &format!("GET /api/ws?token=ABCDE-FGHJK&s={} HTTP/1.1", p.secret)).await;
    assert!(got.contains("200 OK"), "expected the agent's reply, got {got:?}");
    assert_eq!(status.take(), None, "a good connection must not record an error");

    // The request line reaches the agent verbatim, secret and all, the agent ignores `s`.
    let forwarded = seen.lock().unwrap().clone();
    assert!(forwarded[0].contains("token=ABCDE-FGHJK"), "token must survive: {forwarded:?}");
    assert!(forwarded[0].contains(&format!("s={}", p.secret)));
}

#[tokio::test]
async fn refuses_a_request_without_the_secret() {
    let (agent_port, pin, seen) = agent().await;
    let current = Arc::new(Mutex::new(Some(proxy::Target::new(pairing(agent_port, &pin)).unwrap())));
    let status = Arc::new(proxy::Status::default());
    let p = proxy::spawn(current, status.clone()).await.unwrap();

    let got = request(p.port, "GET /api/ws?token=ABCDE-FGHJK HTTP/1.1").await;
    assert_eq!(got, "", "a request without the secret must get nothing");
    assert!(seen.lock().unwrap().is_empty(), "it must never reach the agent");
    // Not the user's problem, so it stays out of the UI.
    assert_eq!(status.take(), None);
}

#[tokio::test]
async fn refuses_a_frame_with_the_wrong_pin() {
    let (agent_port, _pin, seen) = agent().await;
    let wrong = "0000000000000000000000000A";
    let current = Arc::new(Mutex::new(Some(proxy::Target::new(pairing(agent_port, wrong)).unwrap())));
    let status = Arc::new(proxy::Status::default());
    let p = proxy::spawn(current, status.clone()).await.unwrap();

    let got = request(p.port, &format!("GET /api/ws?token=ABCDE-FGHJK&s={} HTTP/1.1", p.secret)).await;
    assert_eq!(got, "", "a pin mismatch must not relay anything");
    assert!(seen.lock().unwrap().is_empty(), "the request must not reach the agent");
    let error = status.take().expect("a pin mismatch must be surfaced to the user");
    assert!(error.contains("paired with"), "unhelpful message: {error}");
}
