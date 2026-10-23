//! Public protocol foundation for native Harness clients.
//!
//! The crate contains wire data, launch-token credential handling, and HTTP transport;
//! interface rendering and Host lifecycle remain outside this crate.

pub mod approvals;
pub mod auth;
pub mod binary_response;
pub mod chat;
pub mod envelope;
pub mod follow;
pub mod message_feedback;
pub mod models;
pub mod plugins;
pub mod remote_events;
pub mod remote_stream;
pub mod schedules;
pub mod session;
pub mod sessions;
pub mod settings;
pub mod stream;
pub mod streams;
pub mod skills;
pub mod terminal;
pub mod transport;
pub mod user_questions;
pub mod upload;
pub mod workspace_files;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid URL: {source}")]
    InvalidUrl { source: url::ParseError },
    #[error("invalid protocol envelope: {0}")]
    InvalidEnvelope(&'static str),
    #[error("response correlation mismatch: expected {expected}, got {actual}")]
    CorrelationMismatch { expected: String, actual: String },
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("transport runtime failed: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("HTTP status {0}")]
    HttpStatus(u16),
    #[error("invalid binary RPC response: {0}")]
    BinaryResponse(#[from] binary_response::BinaryResponseError),
    #[error("request cancelled before completion")]
    Cancelled,
    #[error("authentication response did not include a cookie")]
    MissingCredential,
    #[error("authentication response contained an invalid cookie")]
    InvalidCredential,
    #[error("remote RPC failed ({code}): {message}")]
    Remote {
        code: String,
        message: String,
        details: serde_json::Map<String, serde_json::Value>,
    },
}

impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidEnvelope("JSON decoding")
    }
}
