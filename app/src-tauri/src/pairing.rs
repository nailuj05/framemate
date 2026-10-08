//! The scanned pairing payload, persisted so the app reconnects without rescanning.
//!
//! `FM1 <host> <port> <token> <pin> [<ip>]` : see crates/agent/src/pair.rs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const TAG: &str = "FM1";
const ABSENT: &str = "-";
/// 128 bits in Crockford base32.
const PIN_LEN: usize = 26;
/// Must match the agent's token alphabet (crates/agent/src/config.rs).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pairing {
    /// `<hostname>.local`, or `None` when the agent couldn't determine it.
    pub host: Option<String>,
    /// IPv4 fallback
    pub ip: Option<String>,
    pub port: u16,
    pub token: String,
    pub pin: String,
}

impl Pairing {
    pub fn parse(payload: &str) -> Result<Self, String> {
        let fields: Vec<&str> = payload.split_whitespace().collect();
        let [tag, host, port, token, pin, rest @ ..] = fields.as_slice() else {
            return Err("not a FrameMate pairing code".into());
        };
        if *tag != TAG {
            // app is older version than agent (or the other way round)
            return Err(format!("unknown pairing format {tag}; update the app or the agent"));
        }
        let pin = normalise_pin(pin)?;
        let optional = |s: &str| (s != ABSENT).then(|| s.to_owned());
        Ok(Self {
            host: optional(host),
            ip: rest.first().and_then(|ip| optional(ip)),
            port: port.parse().map_err(|_| format!("bad port {port}"))?,
            token: token.to_string(),
            pin,
        })
    }

    // try mDNS first then address
    pub fn candidates(&self) -> Vec<String> {
        self.host.iter().chain(self.ip.iter()).cloned().collect()
    }
}

fn normalise_pin(pin: &str) -> Result<String, String> {
    let pin: String = pin
        .chars()
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect();
    if pin.len() != PIN_LEN || !pin.bytes().all(|b| ALPHABET.contains(&b)) {
        return Err("pairing code is damaged; print a new one with `pair` and scan it".into());
    }
    Ok(pin)
}

pub fn load(path: &PathBuf) -> Option<Pairing> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn store(path: &PathBuf, pairing: &Pairing) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec(pairing).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_the_agent_prints() {
        let pin = "9F8Q3K2M7PWZ4X5TJH6NBCDRVA";
        let p = Pairing::parse(&format!("FM1 frame.local 7381 ABCDE-FGHJK {pin} 192.168.1.50")).unwrap();
        assert_eq!(p.host.as_deref(), Some("frame.local"));
        assert_eq!(p.ip.as_deref(), Some("192.168.1.50"));
        assert_eq!((p.port, p.token.as_str(), p.pin.as_str()), (7381, "ABCDE-FGHJK", pin));
        assert_eq!(p.candidates(), ["frame.local", "192.168.1.50"]);

        // No address: the field is absent, not empty.
        let p = Pairing::parse(&format!("FM1 frame.local 7381 ABCDE-FGHJK {pin}")).unwrap();
        assert_eq!(p.ip, None);
        assert_eq!(p.candidates(), ["frame.local"]);

        // No hostname either; only the address is usable.
        let p = Pairing::parse(&format!("FM1 - 7381 ABCDE-FGHJK {pin} 192.168.1.50")).unwrap();
        assert_eq!(p.host, None);
        assert_eq!(p.candidates(), ["192.168.1.50"]);
    }

    #[test]
    fn rejects_payloads_it_cannot_trust() {
        let pin = "9F8Q3K2M7PWZ4X5TJH6NBCDRVA";
        for bad in [
            "",
            "FM1 frame.local 7381",
            &format!("FM2 frame.local 7381 ABCDE-FGHJK {pin}"),
            "FM1 frame.local 7381 ABCDE-FGHJK TOOSHORT",
            // U is not in the Crockford alphabet.
            &format!("FM1 frame.local 7381 ABCDE-FGHJK {}", "U".repeat(26)),
            &format!("FM1 frame.local notaport ABCDE-FGHJK {pin}"),
        ] {
            assert!(Pairing::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn a_hand_typed_pin_is_read_leniently() {
        let typed = "9f8q3k2m7pwz4x5tjh6nbcdrva";
        let p = Pairing::parse(&format!("FM1 frame.local 7381 ABCDE-FGHJK {typed}")).unwrap();
        assert_eq!(p.pin, "9F8Q3K2M7PWZ4X5TJH6NBCDRVA");
        // O/I/L are the look-alikes Crockford folds away.
        let p = Pairing::parse("FM1 frame.local 7381 ABCDE-FGHJK OIL23456789012345678901234").unwrap();
        assert_eq!(&p.pin[..3], "011");
    }

    #[test]
    fn survives_a_round_trip() {
        let pin = "9F8Q3K2M7PWZ4X5TJH6NBCDRVA";
        let p = Pairing::parse(&format!("FM1 frame.local 7381 ABCDE-FGHJK {pin} 192.168.1.50")).unwrap();
        let path = std::env::temp_dir().join("framemate-pairing-test.json");
        store(&path, &p).unwrap();
        assert_eq!(load(&path).unwrap(), p);
        let _ = std::fs::remove_file(&path);
    }
}
