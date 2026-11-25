//! Relay handshake protocol shared by `doip-client` and `doip-server`.
//!
//! # Wire format
//!
//! The handshake consists of length-prefixed JSON control frames:
//!
//! ```text
//! +----------------+---------------------------+
//! | u32 BE length  | compact JSON (length B)   |
//! +----------------+---------------------------+
//! ```
//!
//! A client starts with [`ClientMessage::Hello`]. The server answers with
//! [`ServerMessage::Waiting`], [`ServerMessage::Paired`] or
//! [`ServerMessage::Error`]. After `Paired`, both sides switch to a raw byte
//! stream: the relay never parses DoIP, it only forwards bytes.
//!
//! Control frames are limited to [`MAX_CONTROL_FRAME`] bytes. The length prefix
//! is read before the body, so oversized frames are rejected without allocating.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use doip_core::Role;

/// Version of the relay handshake implemented by this crate.
pub const PROTOCOL_VERSION: u16 = 1;

/// Default TCP port of the relay server.
pub const DEFAULT_RELAY_PORT: u16 = 13500;

/// Maximum allowed size of one JSON control frame.
pub const MAX_CONTROL_FRAME: usize = 16 * 1024;

/// Maximum accepted session identifier length.
pub const MAX_SESSION_ID_LEN: usize = 128;

/// Maximum accepted session token length.
pub const MAX_TOKEN_LEN: usize = 256;

/// Maximum accepted client name length.
pub const MAX_CLIENT_LEN: usize = 64;

/// First message sent by a client that wants to join a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Protocol version; must equal [`PROTOCOL_VERSION`].
    pub v: u16,
    /// Session identifier shared by both sides of the tunnel.
    pub session: String,
    /// Session token taken from the `DT1-...` code.
    pub token: String,
    /// Role claimed by this connection.
    pub role: Role,
    /// Optional client name/version for server logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

impl Hello {
    /// Checks version and field limits before the server registers the session.
    pub fn validate(&self) -> Result<(), HelloError> {
        if self.v != PROTOCOL_VERSION {
            return Err(HelloError::UnsupportedVersion {
                version: self.v,
                expected: PROTOCOL_VERSION,
            });
        }

        if !valid_text(&self.session, MAX_SESSION_ID_LEN) {
            return Err(HelloError::InvalidSession);
        }

        if !valid_text(&self.token, MAX_TOKEN_LEN) {
            return Err(HelloError::InvalidToken);
        }

        if let Some(client) = &self.client {
            if client.len() > MAX_CLIENT_LEN {
                return Err(HelloError::InvalidClient);
            }
        }

        Ok(())
    }
}

/// Checks that a string is non-blank and within the byte limit.
fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}

/// Messages accepted by the relay server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Request to register one side of a session.
    Hello(Hello),
}

/// Messages sent by the relay server during the handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The connection is registered and is waiting for the other role.
    Waiting {
        /// Session identifier.
        session: String,
    },
    /// Both roles are present; the connection now carries raw bytes.
    Paired {
        /// Session identifier.
        session: String,
    },
    /// The handshake failed; the server closes the connection afterwards.
    Error {
        /// Machine-readable error code.
        code: ErrorCode,
        /// Human-readable explanation for logs.
        message: String,
    },
}

impl ServerMessage {
    /// Builds an error message.
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }
}

/// Machine-readable reason for a rejected handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The control frame was malformed or too large.
    BadFrame,
    /// The client requested an unsupported protocol version.
    UnsupportedVersion,
    /// The `Hello` fields failed validation.
    InvalidHello,
    /// The token does not match the one registered for the session.
    TokenMismatch,
    /// The requested role is already connected for this session.
    RoleTaken,
    /// The server is holding too many pending sessions.
    TooManySessions,
    /// The peer did not arrive before the pending-session timeout.
    PendingTimeout,
    /// The client did not send a valid `Hello` in time.
    HandshakeTimeout,
    /// The server is shutting down.
    ServerShutdown,
}

/// Error produced while validating a [`Hello`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HelloError {
    /// The protocol version is not supported.
    #[error("unsupported protocol version {version}; expected {expected}")]
    UnsupportedVersion {
        /// Version requested by the client.
        version: u16,
        /// Version implemented by this build.
        expected: u16,
    },
    /// The session identifier is empty or too long.
    #[error("invalid session id: expected 1..={MAX_SESSION_ID_LEN} bytes")]
    InvalidSession,
    /// The token is empty or too long.
    #[error("invalid token: expected 1..={MAX_TOKEN_LEN} bytes")]
    InvalidToken,
    /// The optional client name is too long.
    #[error("invalid client name: expected at most {MAX_CLIENT_LEN} bytes")]
    InvalidClient,
}

/// Error produced while reading or writing a control frame.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The underlying socket failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The frame body exceeded [`MAX_CONTROL_FRAME`].
    #[error("control frame too large: {len} bytes (max {max})")]
    TooLarge {
        /// Observed frame length.
        len: usize,
        /// Maximum accepted frame length.
        max: usize,
    },
    /// The frame length was zero.
    #[error("control frame is empty")]
    Empty,
    /// The frame body was not valid JSON for the expected message.
    #[error("invalid control frame JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Reads one length-prefixed JSON control frame.
///
/// Only the exact frame body is read, so bytes sent after the frame stay in the
/// socket for the raw tunnel phase.
pub async fn read_message<T, R>(reader: &mut R) -> Result<T, FrameError>
where
    T: DeserializeOwned,
    R: AsyncRead + Unpin,
{
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;

    if length == 0 {
        return Err(FrameError::Empty);
    }

    if length > MAX_CONTROL_FRAME {
        return Err(FrameError::TooLarge {
            len: length,
            max: MAX_CONTROL_FRAME,
        });
    }

    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await?;

    Ok(serde_json::from_slice(&body)?)
}

/// Writes one length-prefixed JSON control frame and flushes it.
pub async fn write_message<T, W>(writer: &mut W, message: &T) -> Result<(), FrameError>
where
    T: Serialize + ?Sized,
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(message)?;

    if body.is_empty() {
        return Err(FrameError::Empty);
    }

    if body.len() > MAX_CONTROL_FRAME {
        return Err(FrameError::TooLarge {
            len: body.len(),
            max: MAX_CONTROL_FRAME,
        });
    }

    let length = u32::try_from(body.len()).expect("frame length fits into u32");
    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn hello(role: Role) -> Hello {
        Hello {
            v: PROTOCOL_VERSION,
            session: "session-1".to_owned(),
            token: "token-1".to_owned(),
            role,
            client: Some("test-client/0.1.0".to_owned()),
        }
    }

    #[tokio::test]
    async fn client_hello_roundtrips() {
        let (mut sender, mut receiver) = duplex(4096);
        let message = ClientMessage::Hello(hello(Role::Car));

        write_message(&mut sender, &message).await.unwrap();
        let decoded: ClientMessage = read_message(&mut receiver).await.unwrap();

        assert_eq!(decoded, message);
    }

    #[tokio::test]
    async fn server_messages_roundtrip() {
        let (mut sender, mut receiver) = duplex(4096);

        for message in [
            ServerMessage::Waiting {
                session: "session-1".to_owned(),
            },
            ServerMessage::Paired {
                session: "session-1".to_owned(),
            },
            ServerMessage::error(ErrorCode::TokenMismatch, "token mismatch"),
        ] {
            write_message(&mut sender, &message).await.unwrap();
            let decoded: ServerMessage = read_message(&mut receiver).await.unwrap();
            assert_eq!(decoded, message);
        }
    }

    #[tokio::test]
    async fn frame_starts_with_big_endian_length() {
        let (mut sender, mut receiver) = duplex(4096);
        write_message(
            &mut sender,
            &ServerMessage::Paired {
                session: "s".to_owned(),
            },
        )
        .await
        .unwrap();

        let mut prefix = [0_u8; 4];
        receiver.read_exact(&mut prefix).await.unwrap();
        let length = u32::from_be_bytes(prefix) as usize;

        let mut body = vec![0_u8; length];
        receiver.read_exact(&mut body).await.unwrap();
        assert_eq!(body, br#"{"type":"paired","session":"s"}"#);
    }

    #[tokio::test]
    async fn rejects_empty_frame() {
        let (mut sender, mut receiver) = duplex(64);
        sender.write_all(&0_u32.to_be_bytes()).await.unwrap();

        let error = read_message::<ServerMessage, _>(&mut receiver)
            .await
            .unwrap_err();
        assert!(matches!(error, FrameError::Empty));
    }

    #[tokio::test]
    async fn rejects_oversized_frame_without_reading_body() {
        let (mut sender, mut receiver) = duplex(64);
        let length = u32::try_from(MAX_CONTROL_FRAME + 1).unwrap();
        sender.write_all(&length.to_be_bytes()).await.unwrap();

        let error = read_message::<ServerMessage, _>(&mut receiver)
            .await
            .unwrap_err();
        assert!(matches!(error, FrameError::TooLarge { .. }));
    }

    #[tokio::test]
    async fn rejects_invalid_json() {
        let (mut sender, mut receiver) = duplex(64);
        sender.write_all(&1_u32.to_be_bytes()).await.unwrap();
        sender.write_all(b"{").await.unwrap();

        let error = read_message::<ServerMessage, _>(&mut receiver)
            .await
            .unwrap_err();
        assert!(matches!(error, FrameError::Json(_)));
    }

    #[test]
    fn validates_hello() {
        assert!(hello(Role::Car).validate().is_ok());

        let mut message = hello(Role::Car);
        message.v = PROTOCOL_VERSION + 1;
        assert!(matches!(
            message.validate(),
            Err(HelloError::UnsupportedVersion { .. })
        ));

        let mut message = hello(Role::Diag);
        message.session = "   ".to_owned();
        assert_eq!(message.validate(), Err(HelloError::InvalidSession));

        let mut message = hello(Role::Diag);
        message.token = String::new();
        assert_eq!(message.validate(), Err(HelloError::InvalidToken));

        let mut message = hello(Role::Diag);
        message.client = Some("x".repeat(MAX_CLIENT_LEN + 1));
        assert_eq!(message.validate(), Err(HelloError::InvalidClient));
    }

    #[test]
    fn roles_use_lowercase_json() {
        assert_eq!(serde_json::to_string(&Role::Car).unwrap(), "\"car\"");
        assert_eq!(serde_json::to_string(&Role::Diag).unwrap(), "\"diag\"");
    }

    #[test]
    fn hello_json_uses_type_tag_and_short_fields() {
        let json = serde_json::to_value(ClientMessage::Hello(hello(Role::Car))).unwrap();

        assert_eq!(json["type"], "hello");
        assert_eq!(json["v"], PROTOCOL_VERSION);
        assert_eq!(json["role"], "car");
        assert_eq!(json["session"], "session-1");
    }
}
