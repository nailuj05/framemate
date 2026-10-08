//! `install-service` / `uninstall-service`: a systemd user unit that runs `flatpak run …`, since
//! Flatpaks can't autostart in Game Mode (XDG autostart only runs in Plasma).

use std::path::{Path, PathBuf};

use anyhow::Context;
use zbus::Connection;

const UNIT: &str = "framemate-agent.service";

pub async fn install() -> anyhow::Result<()> {
    let flatpak = flatpak_run();
    let exec = match &flatpak {
        // `flatpak run` moves the app into its own scope outside this unit's cgroup, so
        // stopping the unit would only kill the launcher; --die-with-parent ties them together.
        Some((flag, app_id)) => {
            format!("/usr/bin/flatpak run {flag}--die-with-parent --command=framemate-agent {app_id}")
        }
        None => std::env::current_exe()?.display().to_string(),
    };
    let unit = format!(
        "# Installed by `framemate-agent install-service`; remove with `uninstall-service`.\n\
         [Unit]\n\
         Description=FrameMate agent (Steam Frame companion)\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    );
    let path = unit_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, unit).with_context(|| format!("writing {}", path.display()))?;

    // Create the token and TLS key now, so the starting service and a following `token` or
    // `pair` call can't race over generating them.
    crate::config::load_or_create_token()?;
    crate::tls::Identity::load_or_create()?;

    let systemd = match Systemd::reachable().await {
        Ok(systemd) => systemd,
        Err(e) => {
            enable_without_systemd(&path)?;
            println!("Installed {UNIT}, but systemd isn't reachable from this terminal ({e:#}).");
            println!("That happens in the Frame's Desktop Mode. The agent starts with the next restart");
            println!("of the Frame. To start it now, run this instead:\n");
            println!("{}", rerun_on_user_bus(&flatpak, "install-service"));
            return Ok(());
        }
    };
    systemd.call("EnableUnitFiles", &(&[UNIT][..], false, true)).await?;
    systemd.call("RestartUnit", &(UNIT, "replace")).await?;
    println!("Installed and started {UNIT} ({}).", path.display());
    // `-u {UNIT}` shows nothing: `flatpak run` moves the app into its own scope.
    println!("Logs: journalctl --user -f _COMM=framemate-agent");
    println!();
    crate::check::run().await
}

pub async fn uninstall() -> anyhow::Result<()> {
    let path = unit_path()?;
    let systemd = match Systemd::reachable().await {
        Ok(systemd) => systemd,
        Err(e) => {
            remove_file(&wants_link(&path))?;
            remove_file(&path)?;
            println!("Removed {UNIT}, but systemd isn't reachable from this terminal ({e:#}).");
            println!("A running agent stops with the next restart of the Frame. To stop it now, run:\n");
            println!("{}", rerun_on_user_bus(&flatpak_run(), "uninstall-service"));
            return Ok(());
        }
    };
    // Ignore errors: the unit may not be loaded or enabled.
    let _ = systemd.call("StopUnit", &(UNIT, "replace")).await;
    let _ = systemd.call("DisableUnitFiles", &(&[UNIT][..], false)).await;
    remove_file(&path)?;
    systemd.call("Reload", &()).await?;
    println!("Removed {UNIT}.");
    Ok(())
}

pub async fn rotate_token() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::env::var_os("FRAMEMATE_TOKEN").is_none(),
        "FRAMEMATE_TOKEN is set and overrides the token file"
    );
    let token = crate::config::format_token(&crate::config::rotate_token()?);
    println!("New token: {token}");
    match Systemd::reachable().await {
        // TryRestartUnit only restarts it if it's running; NoSuchUnit without install-service.
        Ok(systemd) => match systemd.call("TryRestartUnit", &(UNIT, "replace")).await {
            Ok(()) => println!("Restarted {UNIT}; enter the new token in the app."),
            Err(_) => println!("{UNIT} isn't installed; restart the agent to use the new token."),
        },
        Err(_) => {
            println!("The running agent keeps the old token until it restarts. Restart the Frame, or run:\n");
            println!(
                "  env XDG_RUNTIME_DIR=/run/user/$(id -u) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u)/bus \\\n    systemctl --user restart {UNIT}"
            );
        }
    }
    Ok(())
}

/// `(installation flag incl. trailing space, app id)` when running as a Flatpak.
fn flatpak_run() -> Option<(String, String)> {
    let app_id = std::env::var("FLATPAK_ID").ok()?;
    let installation = std::fs::read_to_string("/.flatpak-info").ok().and_then(|info| installation_flag(&info));
    if installation.is_none() {
        eprintln!("warning: couldn't tell whether the agent is a --user or --system install");
    }
    Some((installation.map(|f| format!("{f} ")).unwrap_or_default(), app_id))
}

/// What `systemctl enable` does on disk; systemd picks it up at the next start of the session.
fn enable_without_systemd(unit: &Path) -> anyhow::Result<()> {
    let link = wants_link(unit);
    std::fs::create_dir_all(link.parent().unwrap())?;
    remove_file(&link)?;
    std::os::unix::fs::symlink(unit, &link).with_context(|| format!("linking {}", link.display()))
}

fn wants_link(unit: &Path) -> PathBuf {
    unit.with_file_name("default.target.wants").join(UNIT)
}

fn remove_file(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// The Frame's Desktop Mode is a nested Plasma session whose own session bus has no systemd
/// behind it (issue #6). Inside the sandbox only that bus is visible, so the command has to be
/// started with the real user bus.
fn rerun_on_user_bus(flatpak: &Option<(String, String)>, command: &str) -> String {
    let run = match flatpak {
        Some((flag, app_id)) => format!("flatpak run {flag}{app_id}"),
        None => "framemate-agent".into(),
    };
    format!(
        "  env XDG_RUNTIME_DIR=/run/user/$(id -u) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u)/bus \\\n    {run} {command}"
    )
}

/// `--user` or `--system`, from `app-path` in the sandbox's `/.flatpak-info`. Plain `flatpak run`
/// fails when no system installation exists (fresh Frames, Flatpak 1.15.8).
fn installation_flag(flatpak_info: &str) -> Option<&'static str> {
    let path = flatpak_info.lines().find_map(|l| l.trim().strip_prefix("app-path="))?;
    if path.starts_with("/var/lib/flatpak/") {
        Some("--system")
    } else if path.contains("/.local/share/flatpak/") {
        Some("--user")
    } else {
        None // a custom installation (installations.d); plain `flatpak run` finds it
    }
}

/// `~/.config/systemd/user/…` on the host. Deliberately not `$XDG_CONFIG_HOME`, which a
/// Flatpak remaps into its own data directory.
fn unit_path() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config/systemd/user").join(UNIT))
}

struct Systemd(Connection);

impl Systemd {
    /// Connects and checks that systemd answers on this session bus.
    async fn reachable() -> anyhow::Result<Self> {
        let systemd = Self(Connection::session().await.context("connecting to the session bus")?);
        systemd.call("Reload", &()).await?;
        Ok(systemd)
    }

    async fn call<B>(&self, method: &str, body: &B) -> anyhow::Result<()>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.0
            .call_method(
                Some("org.freedesktop.systemd1"),
                "/org/freedesktop/systemd1",
                Some("org.freedesktop.systemd1.Manager"),
                method,
                body,
            )
            .await
            .with_context(|| format!("systemd {method}"))?;
        Ok(())
    }
}
