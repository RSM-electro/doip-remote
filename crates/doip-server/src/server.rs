//! Accept loop and connection lifecycle.

use std::{io, net::SocketAddr, time::Duration};

use doip_protocol::{
    read_message, write_message, ClientMessage, ErrorCode, FrameError, ServerMessage,
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{debug, info, warn};

use crate::{config::Config, registry, registry::Peer};

/// Capacity of the handshake → registry event channel.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Upper bound for writing a small control message to a client.
const CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(1);

/// A running relay server.
///
/// The accept loop and the session registry start in [`Server::bind`]. Call
/// [`Server::shutdown`] to cancel everything and wait for in-flight tasks.
pub struct Server {
    local_addr: SocketAddr,
    cancel: CancellationToken,
    tracker: TaskTracker,
    accept_task: Option<JoinHandle<()>>,
    registry_task: JoinHandle<()>,
}

impl Server {
    /// Binds the relay socket and starts accepting connections.
    pub async fn bind(config: Config) -> io::Result<Self> {
        let listener = TcpListener::bind(config.listen).await?;
        let local_addr = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let tracker = TaskTracker::new();
        let (events, receiver) = mpsc::channel(EVENT_CHANNEL_CAPACITY);

        let registry_task = tokio::spawn(registry::run(
            receiver,
            config.clone(),
            cancel.clone(),
            tracker.clone(),
        ));
        let accept_task = tokio::spawn(accept_loop(
            listener,
            events,
            config,
            cancel.clone(),
            tracker.clone(),
        ));

        Ok(Self {
            local_addr,
            cancel,
            tracker,
            accept_task: Some(accept_task),
            registry_task,
        })
    }

    /// Returns the actual bound address (useful with port `0` in tests).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Waits until the accept loop stops.
    pub async fn wait(&mut self) {
        if let Some(task) = self.accept_task.as_mut() {
            if let Err(error) = task.await {
                warn!(%error, "accept task failed");
            }
        }
        self.accept_task = None;
    }

    /// Cancels and waits for all network tasks.
    pub async fn shutdown(mut self) {
        self.cancel.cancel();

        if let Some(task) = self.accept_task.take() {
            let _ = task.await;
        }
        let _ = self.registry_task.await;

        self.tracker.close();
        self.tracker.wait().await;
        info!("relay stopped");
    }
}

/// Accepts TCP connections until cancellation.
async fn accept_loop(
    listener: TcpListener,
    events: mpsc::Sender<Peer>,
    config: Config,
    cancel: CancellationToken,
    tracker: TaskTracker,
) {
    match listener.local_addr() {
        Ok(address) => info!(%address, "relay listening"),
        Err(error) => warn!(%error, "could not read listener address"),
    }

    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    tracker.spawn(handle_connection(
                        stream,
                        peer,
                        events.clone(),
                        config.clone(),
                        cancel.clone(),
                    ));
                }
                Err(error) => {
                    warn!(%error, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }

    drop(listener);
    debug!("accept loop stopped");
}

/// Reads and validates one `Hello`, then hands the socket to the registry.
async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    events: mpsc::Sender<Peer>,
    config: Config,
    cancel: CancellationToken,
) {
    let _ = stream.set_nodelay(true);

    let handshake = tokio::select! {
        () = cancel.cancelled() => return,
        result = tokio::time::timeout(
            config.handshake_timeout,
            read_message::<ClientMessage, _>(&mut stream),
        ) => result,
    };

    let hello = match handshake {
        Err(_) => {
            reject(
                &mut stream,
                ErrorCode::HandshakeTimeout,
                "handshake timed out",
            )
            .await;
            return;
        }
        Ok(Err(FrameError::Io(error))) => {
            debug!(%peer, %error, "connection closed during handshake");
            return;
        }
        Ok(Err(error)) => {
            reject(&mut stream, ErrorCode::BadFrame, error.to_string()).await;
            return;
        }
        Ok(Ok(ClientMessage::Hello(hello))) => hello,
    };

    if let Err(error) = hello.validate() {
        reject(&mut stream, ErrorCode::InvalidHello, error.to_string()).await;
        return;
    }

    debug!(
        %peer,
        role = %hello.role,
        session = %hello.session,
        client = ?hello.client,
        "handshake received"
    );

    if events
        .send(Peer {
            stream,
            hello,
            peer,
        })
        .await
        .is_err()
    {
        warn!(%peer, "session registry is unavailable");
    }
}

/// Sends a rejection and closes the connection (best effort).
async fn reject(stream: &mut TcpStream, code: ErrorCode, message: impl Into<String>) {
    let message = ServerMessage::error(code, message);

    match tokio::time::timeout(CONTROL_WRITE_TIMEOUT, write_message(stream, &message)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => debug!(%error, "failed to send rejection"),
        Err(_) => debug!("timed out sending rejection"),
    }

    let _ = stream.shutdown().await;
}
