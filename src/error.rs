use thiserror::Error;

/// Any request, transport, response validation, or answer access failure.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Configuration(String),
    #[error("could not serialize state: {0}")]
    StateSerialization(#[source] serde_json::Error),
    #[error("HTTP transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    // Do not include the server body: it can echo input or credentials.
    #[error("gateway returned HTTP {status}")]
    Http { status: u16 },
    #[error("invalid evaluation response: {0}")]
    InvalidResponse(String),
    #[error("Asking has already completed")]
    AlreadyCompleted,
    #[error("no answer with ID {0:?}")]
    NotFound(String),
    #[error("answer {id:?} has type {actual}, expected {expected}")]
    TypeMismatch {
        id: String,
        expected: &'static str,
        actual: &'static str,
    },
}
