//! Plaintext L4 relay server for `doip-remote`.
//!
//! The server authenticates two connections with a session id and token, pairs
//! one `Car` with one `Diag`, then forwards raw bytes between them. It does not
//! parse DoIP: the tunnel is a transparent TCP proxy after the handshake.
//!
//! TLS is intentionally not implemented yet; see the project `AGENTS.md` for
//! the staged plan.

#![forbid(unsafe_code)]

pub mod config;
mod registry;
mod server;
mod tunnel;

pub use config::Config;
pub use server::Server;
