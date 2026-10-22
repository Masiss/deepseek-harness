//! Public protocol foundation for native Harness clients.
//!
//! The crate contains wire data, launch-token credential handling, and HTTP transport;
//! interface rendering and Host lifecycle remain outside this crate.

pub mod auth;
pub mod envelope;
pub mod transport;

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
    #[error("HTTP status {0}")]
    HttpStatus(u16),
    #[error("request cancelled before completion")]
    Cancelled,
    #[error("authentication response did not include a cookie")]
    MissingCredential,
    #[error("authentication response contained an invalid cookie")]
    InvalidCredential,
    #[error("remote RPC failed ({code}): {message}")]
    Remote { code: String, message: String },
}

impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self { Self::InvalidEnvelope("JSON decoding") }
}
