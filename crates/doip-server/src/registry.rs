//! Session registry: validates pairing rules and starts tunnels.

use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

use doip_core::Role;
use doip_protocol::{write_message, ErrorCode, Hello, ServerMessage};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc::Receiver, time::MissedTickBehavior};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{debug, info, warn};

use crate::{config::Config, tunnel};

/// Upper bound for writing a small control message to a client.
const CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(1);

/// A socket that passed the handshake and is owned by the registry.
pub(crate) struct Peer {
    pub(crate) stream: TcpStream,
    pub(crate) hello: Hello,
    pub(crate) peer: SocketAddr,
}

struct PendingPeer {
    stream: TcpStream,
    peer: SocketAddr,
}

struct Session {
    token: String,
    created: Instant,
    car: Option<PendingPeer>,
    diag: Option<PendingPeer>,
}

impl Session {
    fn new(token: String) -> Self {
        Self {
            token,
            created: Instant::now(),
            car: None,
            diag: None,
        }
    }
}

/// Runs the pairing state machine until cancellation.
pub(crate) async fn run(
    mut events: Receiver<Peer>,
    config: Config,
    cancel: CancellationToken,
    tracker: TaskTracker,
) {
    let mut sessions: HashMap<String, Session> = HashMap::new();
    let mut scan = tokio::time::interval(config.scan_interval());
    scan.set_missed_tick_behavior(MissedTickBehavior::Delay);
    scan.tick().await;

    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = scan.tick() => expire(&mut sessions, &config).await,
            event = events.recv() => match event {
                Some(peer) => handle(peer, &mut sessions, &config, &cancel, &tracker).await,
                None => break,
            },
        }
    }

    for (session_id, _session) in sessions.drain() {
        info!(session = %session_id, "closing pending session on shutdown");
    }
    debug!("session registry stopped");
}

/// Handles one validated `Hello`.
async fn handle(
    peer: Peer,
    sessions: &mut HashMap<String, Session>,
    config: &Config,
    cancel: &CancellationToken,
    tracker: &TaskTracker,
) {
    let session_id = peer.hello.session.clone();
    let role = peer.hello.role;
    let token = peer.hello.token.clone();

    let mut session = match sessions.remove(&session_id) {
        Some(session) => session,
        None => {
            if sessions.len() >= config.max_pending_sessions {
                reject(
                    peer,
                    ErrorCode::TooManySessions,
                    "too many pending sessions",
                )
                .await;
                return;
            }
            Session::new(token.clone())
        }
    };

    if session.token != token {
        sessions.insert(session_id.clone(), session);
        reject(
            peer,
            ErrorCode::TokenMismatch,
            "session token does not match",
        )
        .await;
        return;
    }

    let occupied = match role {
        Role::Car => session.car.is_some(),
        Role::Diag => session.diag.is_some(),
    };

    if occupied {
        sessions.insert(session_id.clone(), session);
        reject(
            peer,
            ErrorCode::RoleTaken,
            "this role is already connected for the session",
        )
        .await;
        return;
    }

    let pending = PendingPeer {
        stream: peer.stream,
        peer: peer.peer,
    };

    match role {
        Role::Car => session.car = Some(pending),
        Role::Diag => session.diag = Some(pending),
    }

    if session.car.is_some() && session.diag.is_some() {
        let mut car = session.car.take().expect("car slot present");
        let mut diag = session.diag.take().expect("diag slot present");

        let paired = ServerMessage::Paired {
            session: session_id.clone(),
        };
        let car_sent = send(&mut car.stream, &paired).await;
        let diag_sent = send(&mut diag.stream, &paired).await;

        if car_sent && diag_sent {
            info!(
                session = %session_id,
                car = %car.peer,
                diag = %diag.peer,
                "paired session"
            );
            tracker.spawn(tunnel::run(
                car.stream,
                diag.stream,
                session_id,
                cancel.clone(),
            ));
        } else {
            warn!(session = %session_id, "failed to notify both peers; closing pair");
            let _ = car.stream.shutdown().await;
            let _ = diag.stream.shutdown().await;
        }

        return;
    }

    let waiting = ServerMessage::Waiting {
        session: session_id.clone(),
    };
    let sent = match role {
        Role::Car => match session.car.as_mut() {
            Some(pending) => send(&mut pending.stream, &waiting).await,
            None => false,
        },
        Role::Diag => match session.diag.as_mut() {
            Some(pending) => send(&mut pending.stream, &waiting).await,
            None => false,
        },
    };

    if sent {
        info!(session = %session_id, role = %role, "waiting for peer");
        sessions.insert(session_id, session);
    } else {
        warn!(session = %session_id, "failed to send waiting message");
    }
}

/// Closes sessions whose second peer never arrived.
async fn expire(sessions: &mut HashMap<String, Session>, config: &Config) {
    let now = Instant::now();
    let expired: Vec<String> = sessions
        .iter()
        .filter(|(_, session)| {
            now.saturating_duration_since(session.created) >= config.pending_timeout
        })
        .map(|(session_id, _)| session_id.clone())
        .collect();

    for session_id in expired {
        if let Some(session) = sessions.remove(&session_id) {
            info!(session = %session_id, "pending session timed out");
            close_session(
                session_id,
                session,
                ErrorCode::PendingTimeout,
                "pending session timed out",
            )
            .await;
        }
    }
}

/// Sends an error to both pending peers and closes them.
async fn close_session(session_id: String, session: Session, code: ErrorCode, message: &str) {
    let notice = ServerMessage::error(code, message);

    for pending in [session.car, session.diag].into_iter().flatten() {
        let mut stream = pending.stream;
        let _ = send(&mut stream, &notice).await;
        let _ = stream.shutdown().await;
        debug!(session = %session_id, peer = %pending.peer, %message, "pending peer closed");
    }
}

/// Writes a control message with a short timeout.
async fn send(stream: &mut TcpStream, message: &ServerMessage) -> bool {
    match tokio::time::timeout(CONTROL_WRITE_TIMEOUT, write_message(stream, message)).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            debug!(%error, "failed to send control message");
            false
        }
        Err(_) => {
            debug!("timed out sending control message");
            false
        }
    }
}

/// Sends a rejection and closes the socket (best effort).
async fn reject(mut peer: Peer, code: ErrorCode, message: &str) {
    debug!(
        peer = %peer.peer,
        session = %peer.hello.session,
        role = %peer.hello.role,
        %message,
        "rejecting handshake"
    );
    let _ = send(&mut peer.stream, &ServerMessage::error(code, message)).await;
    let _ = peer.stream.shutdown().await;
}
