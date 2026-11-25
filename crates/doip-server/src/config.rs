//! Server configuration.

use std::{net::SocketAddr, time::Duration};

use doip_protocol::DEFAULT_RELAY_PORT;

/// Runtime settings for the relay.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen on.
    pub listen: SocketAddr,
    /// Maximum time allowed for the client to send a valid `Hello`.
    pub handshake_timeout: Duration,
    /// How long a session may wait for its second peer.
    pub pending_timeout: Duration,
    /// Maximum number of sessions with at least one connected peer.
    pub max_pending_sessions: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], DEFAULT_RELAY_PORT)),
            handshake_timeout: Duration::from_secs(10),
            pending_timeout: Duration::from_secs(300),
            max_pending_sessions: 1024,
        }
    }
}

impl Config {
    /// Interval between pending-session expiry sweeps.
    ///
    /// Kept between 50 ms and 30 s so tests expire quickly while a production
    /// server does not wake up unnecessarily often.
    pub(crate) fn scan_interval(&self) -> Duration {
        (self.pending_timeout / 4).clamp(Duration::from_millis(50), Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_interval_is_bounded() {
        let config = Config {
            pending_timeout: Duration::from_millis(10),
            ..Config::default()
        };
        assert_eq!(config.scan_interval(), Duration::from_millis(50));

        let config = Config {
            pending_timeout: Duration::from_secs(300),
            ..Config::default()
        };
        assert_eq!(config.scan_interval(), Duration::from_secs(30));
    }
}
