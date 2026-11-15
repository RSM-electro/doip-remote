//! Core types and shared logic for the `doip-remote` project.
//!
//! The crate currently provides the portable session-code format that a
//! vehicle-side agent (`Car`) hands to a diagnostic client (`Diag`).
//!
//! # Example
//!
//! ```
//! use doip_core::{Role, SessionCode};
//!
//! let session = SessionCode::new(Role::Car, "diag.example.com:13500", None);
//! let code = session.generate();
//!
//! assert!(code.starts_with("DT1-"));
//! assert_eq!(SessionCode::parse(&code).unwrap(), session);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod code;

pub use code::{Error, Role, SessionCode, CODE_PREFIX, DEFAULT_DOIP_PORT};
