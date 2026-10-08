mod access;
mod cdp;
mod check;
mod config;
mod devices;
mod encoder;
mod fmp4;
mod hub;
mod pair;
mod power;
mod server;
mod service;
mod steamos;
mod stream;
mod tls;
mod v4l2;

use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
usage: framemate-agent [COMMAND]

Without a command, runs the agent.

commands:
  install-service    start the agent with the user session (systemd user unit)
  uninstall-service  remove that unit again
  pair               print the pairing QR code for the companion app
  token              print the API token for the companion app
  check              check the running agent and print what the app needs
  rotate-token       replace the API token (and restart the agent to use it)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("install-service") => return service::install().await,
        Some("uninstall-service") => return service::uninstall().await,
        Some("check") => return check::run().await,
        Some("rotate-token") => return service::rotate_token().await,
        Some("pair") => return pair::run(std::env::args().nth(2).as_deref() == Some("--text")).await,
        Some("token") => {
            println!("{}", config::format_token(&config::load_or_create_token()?));
            return Ok(());
        }
        Some(_) => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }

    // Under systemd, journald adds timestamps and doesn't render ANSI colors.
    let under_systemd = std::env::var_os("INVOCATION_ID").is_some();
    let log = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_env("FRAMEMATE_LOG").unwrap_or_else(|_| "info".into()))
        .with_ansi(!under_systemd);
    if under_systemd {
        log.without_time().init();
    } else {
        log.init();
    }

    let config = config::Config::from_env()?;
    let hub = hub::Hub::new(config::hostname());
    let stream = stream::LiveStream::new(hub.clone(), config.stream.clone());

    tokio::spawn(cdp::run(hub.clone(), config.cdp_url.clone()));
    tokio::spawn(power::run(hub.clone(), config.power_supply_dir.clone()));
    tokio::spawn(steamos::run(hub.clone()));

    server::serve(hub, stream, &config).await
}
