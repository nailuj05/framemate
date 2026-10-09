//! Configuration from environment variables. XDG paths, so it works on the host and inside
//! the Flatpak (where XDG_CONFIG_HOME points into ~/.var/app/<id>/config).

use std::io::Read;
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::stream::StreamConfig;

pub struct Config {
    pub listen: SocketAddr,
    /// Pinned TLS for the app (see tls.rs); the plaintext `listen` port stays for browsers.
    pub listen_tls: SocketAddr,
    pub cdp_url: String,
    pub token: String,
    pub power_supply_dir: PathBuf,
    pub stream: StreamConfig,
    /// Accept clients from outside the local network (see access.rs).
    pub allow_remote: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        // Loopback by default: nothing on the network needs the cleartext port since the app
        // moved to TLS, and `check` plus the installer only ever probe it locally. Set it to
        // `[::]:7380` to expose the plain API on the LAN again (the token then travels in clear).
        let listen = env_or("FRAMEMATE_LISTEN", "127.0.0.1:7380")
            .parse()
            .context("FRAMEMATE_LISTEN must be host:port")?;
        let listen_tls = env_or("FRAMEMATE_LISTEN_TLS", "[::]:7381")
            .parse()
            .context("FRAMEMATE_LISTEN_TLS must be host:port")?;
        let token = match std::env::var("FRAMEMATE_TOKEN") {
            Ok(token) if !token.is_empty() => token,
            _ => load_or_create_token()?,
        };
        let token = normalize_token(&token);
        // format_token splits in the middle, so a non-ASCII token would panic later.
        anyhow::ensure!(token.is_ascii(), "FRAMEMATE_TOKEN must be ASCII");
        let fps: u32 = env_or("FRAMEMATE_STREAM_FPS", "30").parse().context("FRAMEMATE_STREAM_FPS")?;
        anyhow::ensure!((1..=120).contains(&fps), "FRAMEMATE_STREAM_FPS must be 1–120");
        let bitrate: u32 = env_or("FRAMEMATE_STREAM_BITRATE", "6000000").parse().context("FRAMEMATE_STREAM_BITRATE")?;
        anyhow::ensure!(bitrate > 0, "FRAMEMATE_STREAM_BITRATE must be > 0");
        Ok(Self {
            listen,
            listen_tls,
            cdp_url: env_or("FRAMEMATE_CDP", "http://127.0.0.1:8080"),
            token,
            power_supply_dir: env_or("FRAMEMATE_POWER_SUPPLY_DIR", "/sys/class/power_supply").into(),
            stream: StreamConfig {
                source_device: env_or("FRAMEMATE_STREAM_SOURCE", "/dev/video99").into(),
                encoder_device: std::env::var("FRAMEMATE_STREAM_ENCODER").unwrap_or_else(|_| default_encoder().into()),
                fps,
                bitrate,
            },
            allow_remote: matches!(env_or("FRAMEMATE_ALLOW_REMOTE", "").as_str(), "1" | "true" | "yes"),
        })
    }
}

/// videoN numbers depend on driver probe order; the udev symlink is stable.
fn default_encoder() -> &'static str {
    if std::path::Path::new("/dev/video-enc0").exists() { "/dev/video-enc0" } else { "/dev/video23" }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

pub fn config_dir() -> anyhow::Result<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

pub fn state_dir() -> anyhow::Result<PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state")
}

fn xdg_dir(var: &str, fallback: &str) -> anyhow::Result<PathBuf> {
    let base = match std::env::var_os(var) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").context("HOME not set")?).join(fallback),
    };
    Ok(base.join("framemate"))
}

/// Crockford base32: no I, L, O, U, so tokens survive being read aloud or typed on a phone.
const TOKEN_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const TOKEN_LEN: usize = 10; // 50 bits

/// Canonical form for comparison: case, dashes and spaces don't matter, and the
/// look-alikes O/I/L read as 0/1 (Crockford decoding).
pub fn normalize_token(token: &str) -> String {
    token
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
}

/// `ABCDE-FGHJK` for display.
pub fn format_token(token: &str) -> String {
    let (a, b) = token.split_at(token.len() / 2);
    format!("{a}-{b}")
}

fn is_current_format(token: &str) -> bool {
    token.len() == TOKEN_LEN && token.bytes().all(|b| TOKEN_ALPHABET.contains(&b))
}

/// `$XDG_CONFIG_HOME/framemate/token`, created on first run; other formats are replaced.
pub fn load_or_create_token() -> anyhow::Result<String> {
    let path = config_dir()?.join("token");
    match std::fs::read_to_string(&path) {
        Ok(token) => {
            let token = normalize_token(token.trim());
            // An older format is replaced on purpose; see is_current_format.
            if is_current_format(&token) {
                return Ok(token);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Minting a replacement here would hand `pair` a token the running agent rejects, and
        // re-pairing would never fix it; only a restart would, with nothing to say so.
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "reading {}. Delete it and restart the agent to get a new token \
                     (the app has to be paired again afterwards)",
                    path.display()
                )
            });
        }
    }
    write_new_token(&path)
}

/// Replaces the token; the running agent only reads it at startup.
pub fn rotate_token() -> anyhow::Result<String> {
    write_new_token(&config_dir()?.join("token"))
}

fn write_new_token(path: &std::path::Path) -> anyhow::Result<String> {
    let mut bytes = [0u8; TOKEN_LEN];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    // 256 is a multiple of 32, so `% 32` is unbiased.
    let token: String = bytes.iter().map(|b| TOKEN_ALPHABET[(b % 32) as usize] as char).collect();

    write_private(path, format_token(&token).as_bytes())?;
    tracing::info!("generated a new API token");
    Ok(token)
}

/// Writes `bytes` to a 0600 file in a 0700 directory, creating both.
pub fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    std::io::Write::write_all(&mut file, bytes)?;
    Ok(())
}

/// Crockford base32, no padding
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(TOKEN_ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(TOKEN_ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_base32() {
        assert_eq!(base32(&[]), "");
        assert_eq!(base32(&[0x00]), "00");
        assert_eq!(base32(&[0xff, 0xff]), "ZZZG");
        // 16 bytes is the pin: 128 bits over 5-bit groups.
        assert_eq!(base32(&[0xab; 16]).len(), 26);
        assert!(base32(&[0x5a; 16]).bytes().all(|b| TOKEN_ALPHABET.contains(&b)));
    }

    #[test]
    fn tokens_compare_leniently() {
        assert_eq!(normalize_token("abcde-fghjk"), "ABCDEFGHJK");
        assert_eq!(normalize_token("0O1Il-ab cd"), "00111ABCD");
        assert_eq!(format_token("ABCDEFGHJK"), "ABCDE-FGHJK");
        assert!(is_current_format("ABCDEFGHJK"));
        assert!(!is_current_format("cbad37e74cbd250279126bbbca61a0a9"));
    }
}
