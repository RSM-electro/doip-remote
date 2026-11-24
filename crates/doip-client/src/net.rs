//! Network task lifecycle.
//!
//! The relay/server is not implemented yet, but the shutdown path already has
//! to be correct: every long-running `tokio` task must observe a
//! [`CancellationToken`] and drop its sockets when the user finishes
//! diagnostics. Dropping a listener closes the OS socket; accepted connections
//! are closed explicitly so they do not linger in `TIME_WAIT`.
//!
//! [`NetworkService`] owns the token and a [`JoinSet`] of all spawned tasks.
//! Call [`NetworkService::shutdown`] once to cancel everything and wait for the
//! tasks to finish.

use std::{net::SocketAddr, time::Duration};

use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Shutdown budget before remaining network tasks are aborted.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Placeholder ping interval until the relay protocol exists.
const PING_INTERVAL: Duration = Duration::from_secs(5);

/// Owns all network tasks and the cancellation token that stops them.
#[derive(Debug)]
pub struct NetworkService {
    cancel: CancellationToken,
    tasks: JoinSet<()>,
}

impl NetworkService {
    /// Starts the placeholder network tasks.
    ///
    /// `listen` is reserved for the future DoIP listener (for example
    /// `127.0.0.1:13400`); the client passes `None` until the server exists.
    #[must_use]
    pub fn start(cancel: CancellationToken, listen: Option<SocketAddr>) -> Self {
        let mut tasks = JoinSet::new();

        let ping_cancel = cancel.clone();
        tasks.spawn(async move {
            ping_task(ping_cancel).await;
        });

        if let Some(addr) = listen {
            let listen_cancel = cancel.clone();
            tasks.spawn(async move {
                bind_and_serve(addr, listen_cancel).await;
            });
        }

        info!("Network tasks started (stub).");

        Self { cancel, tasks }
    }

    /// Cancels every task, waits for the join set to drain and closes sockets.
    pub async fn shutdown(mut self) {
        self.cancel.cancel();

        let drained = tokio::time::timeout(SHUTDOWN_GRACE, async {
            while let Some(result) = self.tasks.join_next().await {
                if let Err(error) = result {
                    warn!("Network task ended with error: {error}");
                }
            }
        })
        .await;

        if drained.is_err() {
            warn!("Network tasks did not stop in time, aborting.");
            self.tasks.abort_all();
            while self.tasks.join_next().await.is_some() {}
        }

        info!("[OK] Network tasks stopped, sockets closed.");
    }
}

/// Periodically pings the relay. Everything wakes up on cancellation.
async fn ping_task(cancel: CancellationToken) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(PING_INTERVAL) => {
                // No relay yet: this is where the keep-alive would go.
            }
        }
    }
}

/// Binds the DoIP listener and serves it until cancellation.
async fn bind_and_serve(addr: SocketAddr, cancel: CancellationToken) {
    match TcpListener::bind(addr).await {
        Ok(listener) => serve_listener(listener, cancel).await,
        Err(error) => warn!("Failed to open socket {addr}: {error}"),
    }
}

/// Accepts placeholder connections until the token is cancelled.
///
/// The listener is dropped on exit, which releases the port. Nothing is kept in
/// `TIME_WAIT` for the listener itself; accepted streams are reset explicitly.
async fn serve_listener(listener: TcpListener, cancel: CancellationToken) {
    if let Ok(addr) = listener.local_addr() {
        info!("DoIP listener started on {addr} (stub).");
    }

    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => close_stream(stream, peer),
                Err(error) => {
                    warn!("Accept error: {error}");
                    break;
                }
            },
        }
    }

    // Dropping the listener closes the listening socket and releases the port.
    drop(listener);
    info!("DoIP listener stopped.");
}

/// Closes an accepted connection without leaving it in `TIME_WAIT`.
#[allow(deprecated)] // `set_linger` is deprecated in tokio, but linger(0) is what
                     // prevents a graceful FIN from keeping the socket around.
fn close_stream(stream: TcpStream, peer: SocketAddr) {
    // Linger(0) sends RST on close instead of a graceful FIN handshake.
    if let Err(error) = stream.set_linger(Some(Duration::ZERO)) {
        warn!("Failed to set linger for {peer}: {error}");
    }
    drop(stream);
    info!("Connection with {peer} closed.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn shutdown_stops_all_tasks_promptly() {
        let cancel = CancellationToken::new();
        let service = NetworkService::start(cancel, None);

        tokio::time::timeout(Duration::from_secs(2), service.shutdown())
            .await
            .expect("shutdown must not hang");
    }

    #[tokio::test]
    async fn listener_releases_its_port_after_cancellation() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral listener");
        let addr = listener.local_addr().expect("local address");
        let cancel = CancellationToken::new();

        let task = tokio::spawn(serve_listener(listener, cancel.clone()));

        // Prove that the listener is actually accepting before we stop it.
        TcpStream::connect(addr)
            .await
            .expect("listener must accept connections");

        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("listener task must stop")
            .expect("listener task must not panic");

        // The port is free again: no lingering socket blocks the rebind.
        TcpListener::bind(addr)
            .await
            .expect("port must be released after shutdown");
    }

    #[tokio::test]
    async fn cancellable_listener_survives_an_explicit_shutdown() {
        let probe = TcpListener::bind("127.0.0.1:0").await.expect("probe bind");
        let addr = probe.local_addr().expect("probe address");
        drop(probe);

        let cancel = CancellationToken::new();
        let service = NetworkService::start(cancel.clone(), Some(addr));

        // Give the background bind a moment, then cancel everything.
        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::time::timeout(Duration::from_secs(2), service.shutdown())
            .await
            .expect("shutdown must not hang");

        TcpListener::bind(addr)
            .await
            .expect("port must be released after shutdown");
    }
}
