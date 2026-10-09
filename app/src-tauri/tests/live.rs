//! The app's real proxy against a real agent: the pin the agent printed must be the pin the
//! verifier accepts, within the time budget the UI allows.
//!
//! Skipped unless FRAMEMATE_PAIRING holds a `framemate-agent pair --text` payload for an agent
//! that is actually running:
//!
//!   FRAMEMATE_PAIRING="$(framemate-agent pair --text | head -1)" cargo test --test live
use std::sync::{Arc, Mutex};
use std::time::Instant;

use framemate_app_lib::pairing::Pairing;
use framemate_app_lib::proxy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// Ignored by default so a plain `cargo test` reports it as ignored rather than as passing:
// it needs a running agent, and a skip that says `ok` would hide the only check that ties the
// agent's pin to the proxy's verifier. Run it with:
//   FRAMEMATE_PAIRING="$(framemate-agent pair --text | head -1)" cargo test --test live -- --ignored
#[tokio::test]
#[ignore = "needs a running agent and FRAMEMATE_PAIRING"]
async fn reaches_the_real_agent_through_the_proxy() {
    let payload = std::env::var("FRAMEMATE_PAIRING")
        .expect("set FRAMEMATE_PAIRING to a `pair --text` payload for a running agent");
    let pairing = Pairing::parse(payload.trim()).expect("payload must parse");
    let token = pairing.token.clone();

    let current = Arc::new(Mutex::new(Some(proxy::Target::new(pairing).unwrap())));
    let status = Arc::new(proxy::Status::default());
    let p = proxy::spawn(current, status.clone()).await.unwrap();

    for attempt in 1..=2 {
        let start = Instant::now();
        let mut socket = TcpStream::connect(("127.0.0.1", p.port)).await.unwrap();
        let request = format!(
            "GET /api/state?token={token}&s={} HTTP/1.1\r\nHost: frame.local\r\nConnection: close\r\n\r\n",
            p.secret
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut body = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(8), socket.read_to_string(&mut body))
            .await
            .unwrap_or_else(|_| panic!("attempt {attempt} exceeded the UI's 8s budget"))
            .unwrap();
        println!("attempt {attempt}: {} bytes in {:?}", body.len(), start.elapsed());
        assert!(body.contains("200 OK"), "attempt {attempt}: {body}");
        assert!(body.contains("\"agent\""));
        assert_eq!(status.take(), None);
    }
}
