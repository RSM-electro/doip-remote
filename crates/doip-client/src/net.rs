//! Relay sessions and network task lifecycle.
//!
//! Two roles share one relay session:
//!
//! * **Car** — connects to the relay, waits for the Diag peer, then dials the
//!   vehicle (`vehicle_ip:doip_port`) and bridges the two sockets.
//! * **Diag** — opens a local DoIP listener (`127.0.0.1:13400` by default),
//!   waits for the Car peer, then bridges the first local diagnostic
//!   connection to the relay.
//!
//! The relay is a transparent L4 pipe after the handshake, so no DoIP frame is
//! parsed here. Every task observes a [`CancellationToken`]; [`NetworkService`]
//! owns the token and a [`JoinSet`] of all spawned tasks, and
//! [`NetworkService::shutdown`] waits for them to close their sockets.

use std::{
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use doip_core::{Role, SessionCode};
use doip_protocol::{
    read_message, write_message, ClientMessage, Hello, ServerMessage, PROTOCOL_VERSION,
};
use tokio::{
    io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc::UnboundedSender,
    task::JoinSet,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::app::ConnectionStatus;

/// Shutdown budget before remaining network tasks are aborted.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Time allowed to reach the relay.
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Time allowed to write the `Hello` control frame.
const CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the client waits for the second peer.
///
/// The server expires pending sessions after 300 s; this is a slightly larger
/// client-side bound so the server's `pending_timeout` error usually arrives
/// first.
const RELAY_PAIR_TIMEOUT: Duration = Duration::from_secs(330);

/// Time allowed to dial the vehicle.
const VEHICLE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Name sent to the relay for its logs.
const CLIENT_NAME: &str = concat!("doip-client/", env!("CARGO_PKG_VERSION"));

/// Network state changes surfaced to the UI loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetEvent {
    /// The tunnel status changed.
    Status(ConnectionStatus),
    /// The session ended (paired tunnel closed, error or cancellation).
    Finished,
}

/// Owns all network tasks and the cancellation token that stops them.
#[derive(Debug)]
pub struct NetworkService {
    cancel: CancellationToken,
    events: UnboundedSender<NetEvent>,
    tasks: JoinSet<()>,
}

impl NetworkService {
    /// Creates an idle service. Sessions are started by [`start_car`] and
    /// [`start_diag`].
    ///
    /// [`start_car`]: Self::start_car
    /// [`start_diag`]: Self::start_diag
    #[must_use]
    pub fn new(cancel: CancellationToken, events: UnboundedSender<NetEvent>) -> Self {
        Self {
            cancel,
            events,
            tasks: JoinSet::new(),
        }
    }

    /// Spawns the Car session: relay pairing, vehicle dial-out, bridge.
    ///
    /// With `hold`, the paired relay socket is kept open instead of dialing the
    /// vehicle. This is a connectivity test mode: it needs no vehicle and no
    /// local proxy.
    pub fn start_car(&mut self, session: SessionCode, hold: bool) {
        let cancel = self.cancel.clone();
        let events = self.events.clone();
        self.tasks.spawn(async move {
            run_car(session, hold, cancel, events).await;
        });
    }

    /// Spawns the Diag session: local listener, relay pairing, bridge.
    ///
    /// With `hold`, no local listener is bound; the paired relay socket is kept
    /// open instead. This avoids the default `13400` collision when both roles
    /// run on the same machine.
    pub fn start_diag(&mut self, session: SessionCode, listen: SocketAddr, hold: bool) {
        let cancel = self.cancel.clone();
        let events = self.events.clone();
        self.tasks.spawn(async move {
            run_diag(session, listen, hold, cancel, events).await;
        });
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

/// Connects to the relay and performs the handshake until `Paired`.
///
/// Emits [`ConnectionStatus::Waiting`] as soon as the relay accepts the
/// handshake and reports that the peer is still missing.
async fn connect_relay(
    session: &SessionCode,
    role: Role,
    events: &UnboundedSender<NetEvent>,
) -> io::Result<TcpStream> {
    info!(
        "Connecting to relay {} (session {})...",
        session.server, session.session_id
    );

    let mut stream = timeout(
        RELAY_CONNECT_TIMEOUT,
        TcpStream::connect(session.server.as_str()),
    )
    .await
    .map_err(|_| timeout_error("connecting to the relay", RELAY_CONNECT_TIMEOUT))??;

    let _ = stream.set_nodelay(true);

    let hello = ClientMessage::Hello(Hello {
        v: PROTOCOL_VERSION,
        session: session.session_id.clone(),
        token: session.token.clone(),
        role,
        client: Some(CLIENT_NAME.to_owned()),
    });

    timeout(CONTROL_WRITE_TIMEOUT, write_message(&mut stream, &hello))
        .await
        .map_err(|_| timeout_error("sending the handshake", CONTROL_WRITE_TIMEOUT))?
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;

    loop {
        let message = timeout(
            RELAY_PAIR_TIMEOUT,
            read_message::<ServerMessage, _>(&mut stream),
        )
        .await
        .map_err(|_| timeout_error("waiting for the peer", RELAY_PAIR_TIMEOUT))?
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;

        match message {
            ServerMessage::Waiting { session } => {
                info!("Relay accepted session {session}; waiting for the peer...");
                let _ = events.send(NetEvent::Status(ConnectionStatus::Waiting));
            }
            ServerMessage::Paired { session } => {
                info!("[OK] Relay paired session {session}.");
                return Ok(stream);
            }
            ServerMessage::Error { code, message } => {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("relay rejected the session ({code:?}): {message}"),
                ));
            }
        }
    }
}

/// Car side: pair with the Diag, dial the vehicle and forward both directions.
async fn run_car(
    session: SessionCode,
    hold: bool,
    cancel: CancellationToken,
    events: UnboundedSender<NetEvent>,
) {
    let result = car_session(&session, hold, &cancel, &events).await;

    if let Err(error) = result {
        warn!("Car session failed: {error}");
    }

    let _ = events.send(NetEvent::Finished);
}

async fn car_session(
    session: &SessionCode,
    hold: bool,
    cancel: &CancellationToken,
    events: &UnboundedSender<NetEvent>,
) -> io::Result<()> {
    let relay = tokio::select! {
        () = cancel.cancelled() => return Err(cancelled_error()),
        result = connect_relay(session, Role::Car, events) => result?,
    };

    if hold {
        let _ = events.send(NetEvent::Status(ConnectionStatus::Connected));
        info!("[OK] Hold mode: relay session paired; no vehicle dial-out.");
        return hold_open(relay, cancel).await;
    }

    let vehicle = vehicle_address(session).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "session code has no vehicle IP; pass --vehicle <IP[:PORT]>",
        )
    })?;

    info!("Dialing vehicle {vehicle}...");
    let mut vehicle_stream = match timeout(
        VEHICLE_CONNECT_TIMEOUT,
        TcpStream::connect(vehicle.as_str()),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            return Err(io::Error::new(
                error.kind(),
                format!("could not connect to the vehicle at {vehicle}: {error}"),
            ));
        }
        Err(_) => {
            return Err(timeout_error(
                "connecting to the vehicle",
                VEHICLE_CONNECT_TIMEOUT,
            ));
        }
    };

    let _ = vehicle_stream.set_nodelay(true);
    info!("[OK] Tunnel established: vehicle {vehicle} <-> relay.");

    let mut relay = relay;
    let _ = events.send(NetEvent::Status(ConnectionStatus::Connected));
    bridge(&mut relay, &mut vehicle_stream, "vehicle", cancel).await;
    Ok(())
}

/// Diag side: open the local listener, pair with the Car and bridge one client.
async fn run_diag(
    session: SessionCode,
    listen: SocketAddr,
    hold: bool,
    cancel: CancellationToken,
    events: UnboundedSender<NetEvent>,
) {
    let result = diag_session(&session, listen, hold, &cancel, &events).await;

    if let Err(error) = result {
        warn!("Diag session failed: {error}");
    }

    let _ = events.send(NetEvent::Finished);
}

async fn diag_session(
    session: &SessionCode,
    listen: SocketAddr,
    hold: bool,
    cancel: &CancellationToken,
    events: &UnboundedSender<NetEvent>,
) -> io::Result<()> {
    if hold {
        let relay = tokio::select! {
            () = cancel.cancelled() => return Err(cancelled_error()),
            result = connect_relay(session, Role::Diag, events) => result?,
        };

        let _ = events.send(NetEvent::Status(ConnectionStatus::Connected));
        info!("[OK] Hold mode: relay session paired; no local listener bound.");
        return hold_open(relay, cancel).await;
    }

    // Bind first: an occupied port fails fast, before the Car is disturbed.
    let listener = TcpListener::bind(listen).await.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not listen on {listen}: {error}"),
        )
    })?;

    serve_diag_inner(listener, session, cancel, events).await
}

/// Serves one already-bound Diag listener (used by the end-to-end test).
#[cfg(test)]
async fn serve_diag(
    listener: TcpListener,
    session: SessionCode,
    cancel: CancellationToken,
    events: UnboundedSender<NetEvent>,
) {
    let result = serve_diag_inner(listener, &session, &cancel, &events).await;

    if let Err(error) = result {
        warn!("Diag session failed: {error}");
    }

    let _ = events.send(NetEvent::Finished);
}

async fn serve_diag_inner(
    listener: TcpListener,
    session: &SessionCode,
    cancel: &CancellationToken,
    events: &UnboundedSender<NetEvent>,
) -> io::Result<()> {
    let listen = listener.local_addr()?;
    info!("[OK] Local DoIP listener on {listen}.");

    let relay = tokio::select! {
        () = cancel.cancelled() => return Err(cancelled_error()),
        result = connect_relay(session, Role::Diag, events) => result?,
    };

    info!("Waiting for a local diagnostic client on {listen}...");
    let (mut local, peer) = tokio::select! {
        () = cancel.cancelled() => return Err(cancelled_error()),
        accepted = listener.accept() => accepted?,
    };

    info!("Diagnostic client connected from {peer}.");
    let _ = local.set_nodelay(true);
    info!("[OK] Tunnel established: relay <-> local client {peer}.");

    let mut relay = relay;
    let _ = events.send(NetEvent::Status(ConnectionStatus::Connected));
    bridge(&mut relay, &mut local, "diagnostic client", cancel).await;
    Ok(())
}

/// Forwards raw bytes until both sides finish or the token is cancelled.
async fn bridge(
    relay: &mut TcpStream,
    local: &mut TcpStream,
    local_name: &'static str,
    cancel: &CancellationToken,
) {
    let _ = relay.set_nodelay(true);
    let _ = local.set_nodelay(true);

    let result = tokio::select! {
        () = cancel.cancelled() => None,
        result = copy_bidirectional(relay, local) => Some(result),
    };

    let _ = relay.shutdown().await;
    let _ = local.shutdown().await;

    match result {
        Some(Ok((relay_to_local, local_to_relay))) => info!(
            local = local_name,
            relay_to_local, local_to_relay, "Tunnel closed."
        ),
        Some(Err(error)) => warn!(local = local_name, %error, "Tunnel failed."),
        None => info!(local = local_name, "Tunnel stopped by cancellation."),
    }
}

/// Keeps a paired relay socket open without a local proxy or vehicle.
///
/// This is the `--hold` connectivity-test mode: bytes are read and discarded so
/// both peers can verify that pairing against the VPS works and the socket stays
/// alive. It is **not** a DoIP tunnel.
async fn hold_open(mut relay: TcpStream, cancel: &CancellationToken) -> io::Result<()> {
    let mut buffer = vec![0_u8; 4096];
    let mut received: u64 = 0;

    loop {
        let read = tokio::select! {
            () = cancel.cancelled() => {
                info!(received, "Hold mode stopped by cancellation.");
                break;
            }
            result = relay.read(&mut buffer) => result?,
        };

        if read == 0 {
            info!(received, "Peer closed the relay connection.");
            break;
        }

        received += read as u64;
        info!(received, bytes = read, "Hold mode: relay socket alive.");
    }

    let _ = relay.shutdown().await;
    Ok(())
}

/// Builds the vehicle socket address from the session code.
fn vehicle_address(session: &SessionCode) -> Option<String> {
    let host = session.vehicle_ip.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }

    Some(format_host_port(host, session.doip_port))
}

/// Formats `host:port`, bracketing IPv6 literals.
fn format_host_port(host: &str, port: u16) -> String {
    if host.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv6()) && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn timeout_error(action: &str, duration: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("timed out {action} after {} s", duration.as_secs()),
    )
}

fn cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "session cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use doip_protocol::{ErrorCode, MAX_CONTROL_FRAME};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::mpsc,
    };

    fn session(relay: SocketAddr, id: &str, token: &str, role: Role) -> SessionCode {
        SessionCode {
            role,
            server: relay.to_string(),
            session_id: id.to_owned(),
            token: token.to_owned(),
            key: "test-key".to_owned(),
            bridge: true,
            doip_port: 13400,
            vehicle_ip: None,
        }
    }

    fn drain(receiver: &mut mpsc::UnboundedReceiver<NetEvent>) -> Vec<NetEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn shutdown_stops_an_idle_service_promptly() {
        let cancel = CancellationToken::new();
        let (events, _receiver) = mpsc::unbounded_channel();
        let service = NetworkService::new(cancel, events);

        tokio::time::timeout(Duration::from_secs(2), service.shutdown())
            .await
            .expect("shutdown must not hang");
    }

    #[tokio::test]
    async fn handshake_waits_for_waiting_then_pairs() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");

        let relay = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let ClientMessage::Hello(hello) = read_message::<ClientMessage, _>(&mut socket)
                .await
                .expect("read hello");

            assert_eq!(hello.role, Role::Car);
            assert_eq!(hello.session, "handshake-session");

            write_message(
                &mut socket,
                &ServerMessage::Waiting {
                    session: hello.session.clone(),
                },
            )
            .await
            .expect("write waiting");
            write_message(
                &mut socket,
                &ServerMessage::Paired {
                    session: hello.session,
                },
            )
            .await
            .expect("write paired");
        });

        let (events, mut receiver) = mpsc::unbounded_channel();
        let stream = connect_relay(
            &session(address, "handshake-session", "token", Role::Car),
            Role::Car,
            &events,
        )
        .await
        .expect("paired stream");

        assert!(stream.peer_addr().is_ok());
        assert!(drain(&mut receiver).contains(&NetEvent::Status(ConnectionStatus::Waiting)));
        relay.await.expect("relay task");
    }

    #[tokio::test]
    async fn handshake_error_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_message::<ClientMessage, _>(&mut socket)
                .await
                .expect("read hello");
            write_message(
                &mut socket,
                &ServerMessage::error(ErrorCode::TokenMismatch, "session token does not match"),
            )
            .await
            .expect("write error");
        });

        let (events, _receiver) = mpsc::unbounded_channel();
        let error = connect_relay(
            &session(address, "wrong-token", "token", Role::Car),
            Role::Car,
            &events,
        )
        .await
        .expect_err("handshake must fail");

        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert!(error.to_string().contains("TokenMismatch"));
    }

    #[tokio::test]
    async fn oversized_handshake_reply_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_message::<ClientMessage, _>(&mut socket).await;
            let length = u32::try_from(MAX_CONTROL_FRAME + 1).expect("length fits");
            socket
                .write_all(&length.to_be_bytes())
                .await
                .expect("write length");
        });

        let (events, _receiver) = mpsc::unbounded_channel();
        let error = connect_relay(
            &session(address, "oversized", "token", Role::Car),
            Role::Car,
            &events,
        )
        .await
        .expect_err("oversized reply must fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    /// Echoes every byte until the peer closes.
    async fn spawn_echo_vehicle() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind echo");
        let address = listener.local_addr().expect("echo address");

        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept echo");
            let mut buffer = [0_u8; 1024];

            loop {
                match socket.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if socket.write_all(&buffer[..read]).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        (address, task)
    }

    /// Waits until the status stream reports `Connected`, returning every event.
    async fn wait_for_connected(rx: &mut mpsc::UnboundedReceiver<NetEvent>) -> Vec<NetEvent> {
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let event = timeout(remaining, rx.recv())
                .await
                .expect("timed out waiting for Connected")
                .expect("event channel closed");

            seen.push(event);
            if event == NetEvent::Status(ConnectionStatus::Connected) {
                return seen;
            }
        }
    }

    #[tokio::test]
    async fn hold_mode_pairs_and_keeps_the_relay_socket_open() {
        let relay = doip_server::Server::bind(doip_server::Config {
            listen: "127.0.0.1:0".parse().expect("relay address"),
            handshake_timeout: Duration::from_secs(2),
            pending_timeout: Duration::from_secs(10),
            max_pending_sessions: 8,
        })
        .await
        .expect("bind relay");

        let car = session(relay.local_addr(), "hold-e2e", "shared-token", Role::Car);
        let diag = session(relay.local_addr(), "hold-e2e", "shared-token", Role::Diag);

        let cancel = CancellationToken::new();
        let (car_tx, mut car_rx) = mpsc::unbounded_channel();
        let (diag_tx, mut diag_rx) = mpsc::unbounded_channel();

        let car_task = tokio::spawn(run_car(car, true, cancel.clone(), car_tx));
        let diag_task = tokio::spawn(run_diag(
            diag,
            "127.0.0.1:0".parse().expect("listen address"),
            true,
            cancel.clone(),
            diag_tx,
        ));

        let car_events = wait_for_connected(&mut car_rx).await;
        let diag_events = wait_for_connected(&mut diag_rx).await;

        // The second peer is paired immediately, so only the first one observes
        // `Waiting`; both must end up `Connected`.
        assert!(
            car_events.contains(&NetEvent::Status(ConnectionStatus::Waiting))
                || diag_events.contains(&NetEvent::Status(ConnectionStatus::Waiting)),
            "the first peer must observe the waiting state"
        );

        // Hold mode keeps the paired socket open until the session is cancelled.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !car_task.is_finished(),
            "the Car hold session must stay alive after pairing"
        );

        cancel.cancel();

        timeout(Duration::from_secs(10), car_task)
            .await
            .expect("car task timed out")
            .expect("car task panicked");
        timeout(Duration::from_secs(10), diag_task)
            .await
            .expect("diag task timed out")
            .expect("diag task panicked");

        relay.shutdown().await;
    }

    #[tokio::test]
    async fn car_and_diag_forward_bytes_through_the_relay() {
        let (vehicle, vehicle_task) = spawn_echo_vehicle().await;

        let relay = doip_server::Server::bind(doip_server::Config {
            listen: "127.0.0.1:0".parse().expect("relay address"),
            handshake_timeout: Duration::from_secs(2),
            pending_timeout: Duration::from_secs(10),
            max_pending_sessions: 8,
        })
        .await
        .expect("bind relay");

        let local_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind local");
        let local_address = local_listener.local_addr().expect("local address");

        let car_session = SessionCode {
            vehicle_ip: Some(vehicle.ip().to_string()),
            doip_port: vehicle.port(),
            ..session(relay.local_addr(), "client-e2e", "shared-token", Role::Car)
        };
        let diag_session = session(relay.local_addr(), "client-e2e", "shared-token", Role::Diag);

        let cancel = CancellationToken::new();
        let (car_tx, mut car_rx) = mpsc::unbounded_channel();
        let (diag_tx, mut diag_rx) = mpsc::unbounded_channel();

        let diag_cancel = cancel.clone();
        let diag_task = tokio::spawn(serve_diag(
            local_listener,
            diag_session,
            diag_cancel,
            diag_tx,
        ));
        let car_cancel = cancel.clone();
        let car_task = tokio::spawn(run_car(car_session, false, car_cancel, car_tx));

        let mut tester = TcpStream::connect(local_address)
            .await
            .expect("connect local Diagnostic client");
        tester
            .write_all(b"echo through the relay")
            .await
            .expect("write payload");

        let mut echo = vec![0_u8; b"echo through the relay".len()];
        timeout(Duration::from_secs(10), tester.read_exact(&mut echo))
            .await
            .expect("echo timed out")
            .expect("read echo");
        assert_eq!(echo, b"echo through the relay");

        drop(tester);

        timeout(Duration::from_secs(10), car_task)
            .await
            .expect("car task timed out")
            .expect("car task panicked");
        timeout(Duration::from_secs(10), diag_task)
            .await
            .expect("diag task timed out")
            .expect("diag task panicked");

        assert!(drain(&mut car_rx).contains(&NetEvent::Status(ConnectionStatus::Connected)));
        assert!(drain(&mut diag_rx).contains(&NetEvent::Status(ConnectionStatus::Connected)));

        relay.shutdown().await;
        let _ = vehicle_task.await;
    }
}
