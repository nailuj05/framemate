//! `pair`: cert, local-domain, token and IP in a QR code.
//!
//! Works without a running agent by only reading the token and key off disk

use std::net::IpAddr;

use qrcode::{Color, EcLevel, QrCode};

use crate::config;
use crate::tls::Identity;

/// Format marker. The app is side-loaded, so its version drifts from the agent's freely; this
/// lets a mismatch be reported instead of misparsed.
const TAG: &str = "FM1";

/// Placeholder for a field the agent couldn't determine, so parsing stays positional.
const ABSENT: &str = "-";

/// Modules of quiet zone. The spec asks for 4; 2 scans fine and saves four terminal lines.
const QUIET: isize = 2;

pub async fn run(text_only: bool) -> anyhow::Result<()> {
    let config = config::Config::from_env()?;
    let token = config::format_token(&config.token);
    let identity = Identity::load_or_create()?;
    let payload = payload(&host(), config.listen_tls.port(), &token, &identity.pin(), crate::access::lan_address());

    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M)?;
    let needed = code.width() + 2 * QUIET as usize;
    match () {
        _ if text_only => {}
        _ if needed > columns() => {
            println!("Terminal is {} columns, the QR code needs {needed}. Enter this in the app:\n", columns());
        }
        _ => println!("{}", half_blocks(&code)),
    }
    println!("  {payload}\n");
    warn_if_the_agent_disagrees(&config).await;
    println!("Scan it in the app under Settings, or type the fields in by hand.");
    println!("Treat it like a password: it contains the access token.");
    Ok(())
}

/// A running agent serves the token and key it read at startup. If either file changed since then, this code is already wrong
async fn warn_if_the_agent_disagrees(config: &config::Config) {
    let authority = match config.listen.ip() {
        ip if ip.is_unspecified() => format!("127.0.0.1:{}", config.listen.port()),
        ip => format!("{}", std::net::SocketAddr::new(ip, config.listen.port())),
    };
    // No answer at all means no agent running, which is normal right after install.
    if crate::cdp::http_get(&authority, "/healthz").await.is_err() {
        return;
    }
    if crate::cdp::http_get(&authority, &format!("/api/state?token={}", config.token)).await.is_err() {
        println!("WARNING: the running agent does not accept the token above, so its files were");
        println!("replaced while it was running. Restart the agent before pairing:");
        println!("  systemctl --user restart framemate-agent.service\n");
    }
}

/// `<hostname>.local`, which is how the app reaches the Frame when mDNS works.
fn host() -> String {
    let hostname = config::hostname();
    match hostname.as_str() {
        "" => ABSENT.to_owned(),
        h if h.contains('.') => h.to_owned(),
        h => format!("{h}.local"),
    }
}

fn payload(host: &str, port: u16, token: &str, pin: &str, ip: Option<IpAddr>) -> String {
    let mut out = format!("{TAG} {host} {port} {token} {pin}");
    // Last and optional, so IPv6 colons and a missing address are both harmless to parse.
    if let Some(ip) = ip {
        out.push(' ');
        out.push_str(&ip.to_string());
    }
    out
}

/// Terminal width, or 80 when stdout isn't a terminal.
fn columns() -> usize {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes one winsize through the pointer.
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    match size.ws_col {
        cols if ok && cols > 0 => usize::from(cols),
        _ => 80,
    }
}

/// Two QR rows per text line, making it compact by using half blocks
fn half_blocks(code: &QrCode) -> String {
    let width = code.width();
    let modules = code.to_colors();
    let dark = |x: isize, y: isize| {
        (0..width as isize).contains(&x)
            && (0..width as isize).contains(&y)
            && modules[y as usize * width + x as usize] == Color::Dark
    };
    let mut out = String::new();
    for row in 0..(width as isize + 2 * QUIET + 1) / 2 {
        for x in -QUIET..width as isize + QUIET {
            let y = row * 2 - QUIET;
            out.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_positional() {
        let pin = "9F8Q3K2M7PWZ4X5TJH6NBCDRVA";
        let v4 = payload("frame.local", 7381, "ABCDE-FGHJK", pin, Some("192.168.1.50".parse().unwrap()));
        assert_eq!(v4, format!("FM1 frame.local 7381 ABCDE-FGHJK {pin} 192.168.1.50"));
        assert_eq!(v4.split(' ').count(), 6);

        // IPv6 has no spaces, so it doesn't disturb the field split.
        let v6 = payload("frame.local", 7381, "ABCDE-FGHJK", pin, Some("fd12:3456:789a::1".parse().unwrap()));
        assert_eq!(v6.split(' ').nth(5), Some("fd12:3456:789a::1"));
        assert_eq!(v6.split(' ').count(), 6);

        // No address at all: the field is simply absent, earlier ones keep their positions.
        let none = payload(ABSENT, 7381, "ABCDE-FGHJK", pin, None);
        assert_eq!(none.split(' ').count(), 5);
        assert_eq!(none.split(' ').nth(1), Some(ABSENT));
    }

    #[test]
    fn fits_a_standard_terminal() {
        let pin = "9F8Q3K2M7PWZ4X5TJH6NBCDRVA";
        for ip in ["192.168.1.50", "fd12:3456:789a::1", "2001:db8:85a3::8a2e:370:7334"] {
            let payload = payload("frame.local", 7381, "ABCDE-FGHJK", pin, Some(ip.parse().unwrap()));
            let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M).unwrap();
            let render = half_blocks(&code);
            let (cols, rows) = (render.lines().map(|l| l.chars().count()).max().unwrap(), render.lines().count());
            assert!(cols <= 80 && rows <= 24, "{ip}: {cols}x{rows} doesn't fit 80x24");
        }
    }
}
