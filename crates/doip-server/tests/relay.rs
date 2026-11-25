//! End-to-end relay tests over real loopback TCP sockets.

use std::{net::SocketAddr, time::Duration};

use doip_core::Role;
use doip_protocol::{
    read_message, write_message, ClientMessage, ErrorCode, Hello, ServerMessage, MAX_CONTROL_FRAME,
    PROTOCOL_VERSION,
};
use doip_server::{Config, Server};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

const READ_TIMEOUT: Duration = Duration::from_secs(5);

async fn start(pending_timeout: Duration, handshake_timeout: Duration) -> Server {
    start_with_limit(pending_timeout, handshake_timeout, 64).await
}

async fn start_with_limit(
    pending_timeout: Duration,
    handshake_timeout: Duration,
    max_pending_sessions: usize,
) -> Server {
    let config = Config {
        listen: "127.0.0.1:0".parse().expect("valid test address"),
        handshake_timeout,
        pending_timeout,
        max_pending_sessions,
    };

    Server::bind(config).await.expect("bind test server")
}

async fn send_hello(address: SocketAddr, session: &str, token: &str, role: Role) -> TcpStream {
    let mut stream = TcpStream::connect(address).await.expect("connect");

    let message = ClientMessage::Hello(Hello {
        v: PROTOCOL_VERSION,
        session: session.to_owned(),
        token: token.to_owned(),
        role,
        client: Some("test-client/0.1.0".to_owned()),
    });

    write_message(&mut stream, &message)
        .await
        .expect("write hello");
    stream
}

async fn next_message(stream: &mut TcpStream) -> ServerMessage {
    timeout(READ_TIMEOUT, read_message::<ServerMessage, _>(stream))
        .await
        .expect("server message timed out")
        .expect("read server message")
}

async fn pair(server: &Server, session: &str) -> (TcpStream, TcpStream) {
    let address = server.local_addr();

    let mut car = send_hello(address, session, "shared-token", Role::Car).await;
    assert!(matches!(
        next_message(&mut car).await,
        ServerMessage::Waiting { .. }
    ));

    let mut diag = send_hello(address, session, "shared-token", Role::Diag).await;
    assert!(matches!(
        next_message(&mut diag).await,
        ServerMessage::Paired { .. }
    ));
    assert!(matches!(
        next_message(&mut car).await,
        ServerMessage::Paired { .. }
    ));

    (car, diag)
}

async fn write_bytes(stream: &mut TcpStream, bytes: &[u8]) {
    stream.write_all(bytes).await.expect("write tunnel data");
    stream.flush().await.expect("flush tunnel data");
}

async fn expect_bytes(stream: &mut TcpStream, expected: &[u8]) {
    let mut buffer = vec![0_u8; expected.len()];
    timeout(READ_TIMEOUT, stream.read_exact(&mut buffer))
        .await
        .expect("read timed out")
        .expect("read tunnel data");
    assert_eq!(buffer, expected);
}

#[tokio::test]
async fn pairs_and_forwards_bytes_both_ways() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let (mut car, mut diag) = pair(&server, "session-both-ways").await;

    write_bytes(&mut car, b"car->diag").await;
    expect_bytes(&mut diag, b"car->diag").await;

    write_bytes(&mut diag, b"diag->car").await;
    expect_bytes(&mut car, b"diag->car").await;

    server.shutdown().await;
}

#[tokio::test]
async fn forwards_one_mib_payload_intact() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let (mut car, mut diag) = pair(&server, "session-large").await;

    let payload: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
    let expected = payload.clone();

    let writer = tokio::spawn(async move {
        write_bytes(&mut car, &payload).await;
        car
    });

    let mut received = vec![0_u8; expected.len()];
    timeout(Duration::from_secs(10), diag.read_exact(&mut received))
        .await
        .expect("large payload timed out")
        .expect("read large payload");

    assert_eq!(received, expected);
    let _car = writer.await.expect("writer task");
    server.shutdown().await;
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let address = server.local_addr();

    let mut car = send_hello(address, "session-token", "token-a", Role::Car).await;
    assert!(matches!(
        next_message(&mut car).await,
        ServerMessage::Waiting { .. }
    ));

    let mut diag = send_hello(address, "session-token", "token-b", Role::Diag).await;
    match next_message(&mut diag).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::TokenMismatch),
        other => panic!("expected token mismatch, got {other:?}"),
    }

    server.shutdown().await;
}

#[tokio::test]
async fn duplicate_role_is_rejected() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let address = server.local_addr();

    let mut first_car = send_hello(address, "session-role", "token", Role::Car).await;
    assert!(matches!(
        next_message(&mut first_car).await,
        ServerMessage::Waiting { .. }
    ));

    let mut second_car = send_hello(address, "session-role", "token", Role::Car).await;
    match next_message(&mut second_car).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::RoleTaken),
        other => panic!("expected role-taken error, got {other:?}"),
    }

    let mut diag = send_hello(address, "session-role", "token", Role::Diag).await;
    assert!(matches!(
        next_message(&mut diag).await,
        ServerMessage::Paired { .. }
    ));
    assert!(matches!(
        next_message(&mut first_car).await,
        ServerMessage::Paired { .. }
    ));

    server.shutdown().await;
}

#[tokio::test]
async fn sessions_are_isolated() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let (mut car_one, mut diag_one) = pair(&server, "session-one").await;
    let (mut car_two, mut diag_two) = pair(&server, "session-two").await;

    write_bytes(&mut car_one, b"one").await;
    write_bytes(&mut car_two, b"two").await;

    expect_bytes(&mut diag_one, b"one").await;
    expect_bytes(&mut diag_two, b"two").await;

    server.shutdown().await;
}

#[tokio::test]
async fn malformed_frame_is_rejected() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let mut stream = TcpStream::connect(server.local_addr())
        .await
        .expect("connect");

    stream.write_all(&1_u32.to_be_bytes()).await.unwrap();
    stream.write_all(b"{").await.unwrap();

    match next_message(&mut stream).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::BadFrame),
        other => panic!("expected bad-frame error, got {other:?}"),
    }

    server.shutdown().await;
}

#[tokio::test]
async fn oversized_frame_is_rejected() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let mut stream = TcpStream::connect(server.local_addr())
        .await
        .expect("connect");

    let length = u32::try_from(MAX_CONTROL_FRAME + 1).expect("length fits");
    stream.write_all(&length.to_be_bytes()).await.unwrap();

    match next_message(&mut stream).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::BadFrame),
        other => panic!("expected bad-frame error, got {other:?}"),
    }

    server.shutdown().await;
}

#[tokio::test]
async fn handshake_timeout_is_rejected() {
    let server = start(Duration::from_secs(30), Duration::from_millis(100)).await;
    let mut stream = TcpStream::connect(server.local_addr())
        .await
        .expect("connect");

    match next_message(&mut stream).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::HandshakeTimeout),
        other => panic!("expected handshake-timeout error, got {other:?}"),
    }

    server.shutdown().await;
}

#[tokio::test]
async fn pending_timeout_is_rejected() {
    let server = start(Duration::from_millis(200), Duration::from_secs(1)).await;
    let mut car = send_hello(server.local_addr(), "session-timeout", "token", Role::Car).await;

    assert!(matches!(
        next_message(&mut car).await,
        ServerMessage::Waiting { .. }
    ));

    match next_message(&mut car).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::PendingTimeout),
        other => panic!("expected pending-timeout error, got {other:?}"),
    }

    server.shutdown().await;
}

#[tokio::test]
async fn peer_disconnect_closes_the_other_side() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let (mut car, diag) = pair(&server, "session-disconnect").await;

    drop(diag);

    let mut buffer = [0_u8; 1];
    let read = timeout(READ_TIMEOUT, car.read(&mut buffer))
        .await
        .expect("car read timed out")
        .expect("car read");
    assert_eq!(read, 0, "car must observe EOF after the diag disconnects");

    server.shutdown().await;
}

#[tokio::test]
async fn shutdown_closes_clients() {
    let server = start(Duration::from_secs(30), Duration::from_secs(1)).await;
    let (mut car, mut diag) = pair(&server, "session-shutdown").await;

    server.shutdown().await;

    for stream in [&mut car, &mut diag] {
        let mut buffer = [0_u8; 1];
        let read = timeout(READ_TIMEOUT, stream.read(&mut buffer))
            .await
            .expect("read timed out")
            .expect("read");
        assert_eq!(read, 0, "shutdown must close the client socket");
    }
}

#[tokio::test]
async fn max_pending_sessions_is_enforced() {
    let server = start_with_limit(Duration::from_secs(30), Duration::from_secs(1), 1).await;
    let address = server.local_addr();

    let mut first = send_hello(address, "session-limit-one", "token", Role::Car).await;
    assert!(matches!(
        next_message(&mut first).await,
        ServerMessage::Waiting { .. }
    ));

    let mut second = send_hello(address, "session-limit-two", "token", Role::Car).await;
    match next_message(&mut second).await {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::TooManySessions),
        other => panic!("expected too-many-sessions error, got {other:?}"),
    }

    server.shutdown().await;
}
