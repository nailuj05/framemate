//! The agent's TLS identity.
//!
//! The app pins `Identity::pin()`, a hash of the public key, and never checks the certificate's
//! name or validity. So only the key is kept on disk and the certificate is rebuilt at every
//! start: a cert/key mismatch in the config directory becomes impossible, and the pin stays
//! valid for as long as the key file does (including across `rotate-token`).

use std::sync::Arc;

use anyhow::Context;
use rcgen::PublicKeyData;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

/// 128 bits of SHA-256(SubjectPublicKeyInfo). Impersonating the agent needs a second preimage
/// rather than a collision, so this is ample, and it keeps the pairing QR one version smaller.
const PIN_BYTES: usize = 16;

/// Placeholder name for the handshake
const SAN: &str = "framemate-agent.invalid";

pub struct Identity(rcgen::KeyPair);

impl Identity {
    /// `$XDG_CONFIG_HOME/framemate/key.der` (PKCS#8), created on first use.
    pub fn load_or_create() -> anyhow::Result<Self> {
        let path = crate::config::config_dir()?.join("key.der");
        match std::fs::read(&path) {
            Ok(der) => match rcgen::KeyPair::try_from(der) {
                Ok(key) => return Ok(Self(key)),
                // Replacing it costs a re-pairing; refusing to start costs everything.
                Err(e) => tracing::warn!("{}: not a usable key ({e}), replacing it", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // Anything else (a permission problem, bad disk) would otherwise look like "no key"
            // and overwrite one that is still perfectly good, silently unpairing every device.
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        }
        let key = rcgen::KeyPair::generate().context("generating a TLS key")?;
        crate::config::write_private(&path, &key.serialize_der())?;
        tracing::info!("generated a new TLS key; the app needs to be paired again");
        Ok(Self(key))
    }

    /// What the app pins, and what goes in the pairing QR.
    pub fn pin(&self) -> String {
        let spki = self.0.subject_public_key_info();
        let digest = ring::digest::digest(&ring::digest::SHA256, &spki);
        crate::config::base32(&digest.as_ref()[..PIN_BYTES])
    }

    pub fn server_config(&self) -> anyhow::Result<Arc<rustls::ServerConfig>> {
        let cert = rcgen::CertificateParams::new(vec![SAN.to_owned()])
            .context("building certificate parameters")?
            .self_signed(&self.0)
            .context("self-signing the certificate")?;
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.0.serialize_der()));
        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .context("selecting TLS versions")?
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], key)
            .context("loading the certificate")?;
        // TLS 1.3 only: rustls is built without its `tls12` feature, so "safe defaults" above
        // resolve to 1.3 alone. Both ends are ours, so there is nothing to stay compatible with.
        // Never let h2 be negotiated either: WebSockets over h2 need extended CONNECT (RFC 8441),
        // which axum doesn't implement, so /api/ws would break for any client offering it.
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_follows_the_key() {
        let (a, b) = (
            Identity(rcgen::KeyPair::generate().unwrap()),
            Identity(rcgen::KeyPair::generate().unwrap()),
        );
        assert_eq!(a.pin().len(), 26);
        assert_ne!(a.pin(), b.pin());
        // Stable across certificates, which are regenerated on every start.
        assert_eq!(a.pin(), a.pin());
        assert!(a.server_config().is_ok());
    }

    /// Mirrors `pin_matches_the_agents_encoding` in app/src-tauri/src/proxy.rs, which derives
    /// the pin from its own copy of this formula. Both must agree or pairing silently breaks.
    #[test]
    fn pin_formula_is_stable() {
        let digest = ring::digest::digest(&ring::digest::SHA256, &[0xab; 91]);
        assert_eq!(crate::config::base32(&digest.as_ref()[..PIN_BYTES]), "KB2BXCR99PG5ADYCFQZDNYKSPR");
    }

    #[test]
    fn key_survives_a_der_round_trip() {
        let key = rcgen::KeyPair::generate().unwrap();
        let reloaded = rcgen::KeyPair::try_from(key.serialize_der()).unwrap();
        assert_eq!(Identity(key).pin(), Identity(reloaded).pin());
    }
}
