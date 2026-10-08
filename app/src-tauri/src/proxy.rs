//! Pinned TLS to the Frame, WebView can't pin so I just proxy it via loopback + secret
//!
//! Loopback isn't isolated between apps so we use a secret injected by rust on start to limit access to our frontend

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::pairing::Pairing;

/// Enough for a request line plus the WebSocket headers.
const HEADER_LIMIT: usize = 8192;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Pause after a failed accept, so a persistent error (EMFILE) cannot become a busy loop.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// A client that opens the loopback socket and then says nothing is dropped after this.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// Crockford base32
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Presented in the handshake and never checked: the pin is the whole trust decision.
/// Deliberately not the Frame's real address: rustls omits SNI for IP literals, so the real
/// host would make the handshake differ by candidate, and the ClientHello is plaintext on the wire. Matches `SAN` in crates/agent/src/tls.rs
const HANDSHAKE_NAME: &str = "framemate-agent.invalid";

/// Set when a connection fails in a way the user has to act on
#[derive(Default)]
pub struct Status(Mutex<Option<String>>);

impl Status {
    pub fn take(&self) -> Option<String> {
        self.guard().take()
    }

    fn set(&self, message: impl Into<String>) {
        *self.guard() = Some(message.into());
    }

    /// Poisoning is ignored: this is one `Option<String>`, and a panicked relay task must not
    /// stop the app reporting anything ever again.
    fn guard(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct Proxy {
    pub port: u16,
    pub secret: String,
}

/// Separates a Frame that isn't answering, which the UI already reports as offline, from one
/// that answered but isn't the Frame we paired with — the only case the user must act on.
enum Failure {
    Unreachable(String),
    Rejected(String),
}

/// Proxy Target for the WebView, valid for the life of the process
pub struct Target {
    pairing: Pairing,
    connector: tokio_rustls::TlsConnector,
    preferred: Arc<AtomicUsize>,
}

impl Target {
    pub fn new(pairing: Pairing) -> Result<Self, String> {
        let verifier = Arc::new(PinnedKey::new(pairing.pin.clone()));
        let mut config = rustls::ClientConfig::builder_with_provider(verifier.provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        // The agent only speaks HTTP/1.1; WebSockets over h2 would need RFC 8441.
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            pairing,
            connector: tokio_rustls::TlsConnector::from(Arc::new(config)),
            preferred: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn token(&self) -> String {
        self.pairing.token.clone()
    }
}

pub type Current = Arc<Mutex<Option<Target>>>;

/// Binds loopback and serves until the process exits.
pub async fn spawn(current: Current, status: Arc<Status>) -> Result<Proxy, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let secret = random_secret()?;

    let gate = format!("s={secret}");
    tokio::spawn(async move {
        loop {
            let inbound = match listener.accept().await {
                Ok((inbound, _)) => inbound,
                // Don't spin at 100% on a persistent error such as EMFILE.
                Err(e) => {
                    eprintln!("framemate: accept failed: {e}");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            };
            let (current, status, gate) = (current.clone(), status.clone(), gate.clone());
            tokio::spawn(async move {
                // Cloned out of the lock so a slow connection doesn't hold up re-pairing.
                let Some((pairing, connector, preferred)) = current
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|t| (t.pairing.clone(), t.connector.clone(), t.preferred.clone()))
                else {
                    return;
                };
                match relay(inbound, &connector, &pairing, &preferred, &gate).await {
                    Err(Failure::Rejected(e)) => status.set(e),
                    // Routine (asleep, off the network). Logged for diagnosis, not shown: the
                    // UI already says "offline", and the frontend retries every few seconds.
                    Err(Failure::Unreachable(e)) => eprintln!("framemate: {e}"),
                    Ok(()) => {}
                }
            });
        }
    });
    Ok(Proxy { port, secret })
}

async fn relay(
    mut inbound: TcpStream,
    connector: &tokio_rustls::TlsConnector,
    pairing: &Pairing,
    preferred: &AtomicUsize,
    gate: &str,
) -> Result<(), Failure> {
    // Read only the request line, check the gate, then forward it verbatim
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    let deadline = tokio::time::Instant::now() + HEAD_TIMEOUT;
    while !head.ends_with(b"\r\n") {
        match tokio::time::timeout_at(deadline, inbound.read(&mut byte)).await {
            // Probe, hang-up, or a connection parked without ever sending a request line.
            Err(_) | Ok(Ok(0)) | Ok(Err(_)) => return Ok(()),
            Ok(Ok(_)) => head.push(byte[0]),
        }
        if head.len() > HEADER_LIMIT {
            return Ok(());
        }
    }
    if !String::from_utf8_lossy(&head).contains(gate) {
        // Another app on the phone, ignore
        return Ok(());
    }

    let mut upstream = connect(connector, pairing, preferred).await?;
    upstream.write_all(&head).await.map_err(|e| Failure::Unreachable(e.to_string()))?;
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut upstream).await;
    Ok(())
}

// Tries the mDNS name and the address, starting with whichever worked last
async fn connect(
    connector: &tokio_rustls::TlsConnector,
    pairing: &Pairing,
    preferred: &AtomicUsize,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, Failure> {
    let candidates = pairing.candidates();
    if candidates.is_empty() {
        return Err(Failure::Rejected("the pairing code carries no address".into()));
    }
    let first = preferred.load(Ordering::Relaxed) % candidates.len();
    let mut last = String::new();
    for offset in 0..candidates.len() {
        let index = (first + offset) % candidates.len();
        let host = &candidates[index];
        let tcp = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), pairing.port))).await
        {
            Ok(Ok(tcp)) => tcp,
            Ok(Err(e)) => {
                last = format!("{host}: {e}");
                continue;
            }
            Err(_) => {
                last = format!("{host}: no answer within {}s", CONNECT_TIMEOUT.as_secs());
                continue;
            }
        };
        let name = ServerName::try_from(HANDSHAKE_NAME).map_err(|e| Failure::Rejected(e.to_string()))?;
        match connector.connect(name, tcp).await {
            Ok(tls) => {
                preferred.store(index, Ordering::Relaxed);
                return Ok(tls);
            }
            // A certificate problem means something answered and it isn't ours; don't fall
            // through to the next address, which would turn a pin mismatch into a vague
            // timeout. A plain IO error (reset, EOF, sleeping Frame) is just unreachable.
            Err(e) if rejected_us(&e) => return Err(Failure::Rejected(format!("{host}: {e}"))),
            Err(e) => {
                last = format!("{host}: {e}");
                continue;
            }
        }
    }
    Err(Failure::Unreachable(last))
}

/// tokio-rustls wraps rustls errors in `io::Error`; everything else is transport trouble.
fn rejected_us(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|e| matches!(e, rustls::Error::InvalidCertificate(_) | rustls::Error::General(_)))
}

#[derive(Debug)]
struct PinnedKey {
    pin: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PinnedKey {
    fn new(pin: String) -> Self {
        Self { pin, provider: Arc::new(rustls::crypto::ring::default_provider()) }
    }
}

impl ServerCertVerifier for PinnedKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let cert = webpki::EndEntityCert::try_from(end_entity)
            .map_err(|_| rustls::Error::General("unparseable certificate".into()))?;
        if pin_of(cert.subject_public_key_info().as_ref()) == self.pin {
            return Ok(ServerCertVerified::assertion());
        }
        Err(rustls::Error::General(
            "this is not the Frame the app was paired with; scan the pairing code again".into(),
        ))
    }

    /// Required by the trait but unreachable: the agent is TLS 1.3 only (see tls.rs). Kept
    /// delegating rather than stubbed, so it stays correct if that ever changes.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Must match `Identity::pin` in the agent: 128 bits of SHA-256(SPKI), base32
fn pin_of(spki: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, spki);
    base32(&digest.as_ref()[..16])
}

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn random_secret() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| "no randomness available".to_owned())?;
    Ok(base32(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_matches_the_agents_encoding() {
        // Same vector as `pin_formula_is_stable` in crates/agent/src/tls.rs. The pin is derived
        // twice, in two crates that share no code; this is what stops them drifting apart.
        // 91 bytes is the real SPKI length for the keys rcgen generates.
        assert_eq!(pin_of(&[0xab; 91]), "KB2BXCR99PG5ADYCFQZDNYKSPR");
        assert_eq!(base32(&[0x00]), "00");
        assert_eq!(base32(&[0xff, 0xff]), "ZZZG");
        assert!(base32(&[0x5a; 16]).bytes().all(|b| ALPHABET.contains(&b)));
    }

    #[test]
    fn secrets_differ_per_run() {
        let (a, b) = (random_secret().unwrap(), random_secret().unwrap());
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
    }
}
