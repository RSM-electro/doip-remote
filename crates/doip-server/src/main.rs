//! `doip-server` command-line entry point.

use std::{env, net::SocketAddr, process::ExitCode, time::Duration};

use doip_server::{Config, Server};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

const HELP: &str = "\
doip-server — plaintext relay for doip-remote

Usage: doip-server [OPTIONS]

Options:
  --listen <ADDR>              Listen address [default: 0.0.0.0:13500]
  --handshake-timeout <SECS>   Handshake timeout [default: 10]
  --pending-timeout <SECS>     Pending session TTL [default: 300]
  --max-pending <COUNT>        Maximum pending sessions [default: 1024]
  -h, --help                   Print help
  -V, --version                Print version";

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();

    let config = match parse_args(env::args().skip(1)) {
        Ok(Some(config)) => config,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("doip-server: {error}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };

    match run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(%error, "server stopped with an error");
            eprintln!("doip-server: {error}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut server = Server::bind(config).await?;
    info!(address = %server.local_addr(), "doip-server ready");

    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal?;
            info!("shutdown signal received");
        }
        () = server.wait() => {
            info!("accept loop stopped");
        }
    }

    server.shutdown().await;
    Ok(())
}

/// Parses CLI arguments. Returns `Ok(None)` for `--help`/`--version`.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Config>, String> {
    let mut config = Config::default();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--listen" => {
                let value = take_value(&mut args, "--listen")?;
                config.listen = value
                    .parse::<SocketAddr>()
                    .map_err(|_| format!("invalid --listen address `{value}`; expected IP:PORT"))?;
            }
            "--handshake-timeout" => {
                let value = take_value(&mut args, "--handshake-timeout")?;
                config.handshake_timeout = parse_seconds(&value, "--handshake-timeout")?;
            }
            "--pending-timeout" => {
                let value = take_value(&mut args, "--pending-timeout")?;
                config.pending_timeout = parse_seconds(&value, "--pending-timeout")?;
            }
            "--max-pending" => {
                let value = take_value(&mut args, "--max-pending")?;
                config.max_pending_sessions = value
                    .parse()
                    .map_err(|_| "--max-pending expects a positive integer".to_owned())?;
                if config.max_pending_sessions == 0 {
                    return Err("--max-pending must be greater than zero".to_owned());
                }
            }
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(None);
            }
            "--version" | "-V" => {
                println!("doip-server {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }

    Ok(Some(config))
}

fn take_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_seconds(value: &str, flag: &str) -> Result<Duration, String> {
    let seconds: u64 = value
        .parse()
        .map_err(|_| format!("{flag} expects a positive integer number of seconds"))?;

    if seconds == 0 {
        return Err(format!("{flag} must be greater than zero"));
    }

    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> impl Iterator<Item = String> {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn defaults_are_used_without_flags() {
        let config = parse_args(args(&[])).unwrap().unwrap();
        assert_eq!(config.listen, SocketAddr::from(([0, 0, 0, 0], 13500)));
        assert_eq!(config.handshake_timeout, Duration::from_secs(10));
        assert_eq!(config.pending_timeout, Duration::from_secs(300));
        assert_eq!(config.max_pending_sessions, 1024);
    }

    #[test]
    fn flags_override_defaults() {
        let config = parse_args(args(&[
            "--listen",
            "127.0.0.1:15000",
            "--handshake-timeout",
            "3",
            "--pending-timeout",
            "60",
            "--max-pending",
            "7",
        ]))
        .unwrap()
        .unwrap();

        assert_eq!(config.listen, "127.0.0.1:15000".parse().unwrap());
        assert_eq!(config.handshake_timeout, Duration::from_secs(3));
        assert_eq!(config.pending_timeout, Duration::from_secs(60));
        assert_eq!(config.max_pending_sessions, 7);
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(parse_args(args(&["--listen"])).is_err());
        assert!(parse_args(args(&["--listen", "localhost"])).is_err());
        assert!(parse_args(args(&["--handshake-timeout", "0"])).is_err());
        assert!(parse_args(args(&["--pending-timeout", "x"])).is_err());
        assert!(parse_args(args(&["--max-pending", "0"])).is_err());
        assert!(parse_args(args(&["--unknown"])).is_err());
    }

    #[test]
    fn help_and_version_stop_parsing() {
        assert!(parse_args(args(&["--help"])).unwrap().is_none());
        assert!(parse_args(args(&["--version"])).unwrap().is_none());
    }
}
