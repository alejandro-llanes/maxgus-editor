//! A language-server client.
//!
//! The client speaks JSON-RPC over a server's stdio, driven by tokio. Framing
//! and position arithmetic are pure functions so they can be tested without a
//! server; the client itself is generic over its transport so the tests drive
//! it through an in-memory duplex pipe.

pub mod client;
pub mod diagnostics;
pub mod position;
pub mod protocol;

pub use client::{Client, ServerEvent};
pub use diagnostics::{Diagnostic, DiagnosticSet, Severity};
pub use position::{LspPosition, LspRange, PositionEncoding};
pub use protocol::{Message, Notification, Request, RequestId, Response, ResponseError};

#[derive(Debug, thiserror::Error)]
pub enum LspError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed message: {0}")]
    Protocol(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the language server exited")]
    ServerGone,
    #[error("server returned an error: {code} {message}")]
    ServerError { code: i64, message: String },
    #[error("request timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("could not start `{command}`: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, LspError>;
