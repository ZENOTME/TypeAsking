use thiserror::Error;

/// Any request, transport, response validation, or answer access failure.
///
/// Both awaiting [`crate::Asking`] and reading [`crate::Answers`] use this type,
/// so callers can propagate all SDK errors with `?`.
#[derive(Debug, Error)]
pub enum Error {
    /// Invalid or missing request settings, detected before network I/O.
    #[error("invalid configuration: {0}")]
    Configuration(String),
    /// State could not be serialized to JSON; reported on the first poll.
    #[error("could not serialize state: {0}")]
    StateSerialization(#[source] serde_json::Error),
    /// HTTP client creation, connection, timeout, or response-read failure.
    #[error("HTTP transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    /// Gateway returned a non-success HTTP status. Its body is intentionally omitted.
    #[error("gateway returned HTTP {status}")]
    Http {
        /// HTTP status code, such as 401 or 429.
        status: u16,
    },
    /// Malformed JSON or an answer inconsistent with the requested IDs/types/scale.
    #[error("invalid evaluation response: {0}")]
    InvalidResponse(String),
    /// An already completed request was polled again; create a new request instead.
    #[error("Asking has already completed")]
    AlreadyCompleted,
    /// No answer exists for the requested question ID.
    #[error("no answer with ID {0:?}")]
    NotFound(String),
    /// The question ID exists, but the accessor expects a different answer type.
    #[error("answer {id:?} has type {actual}, expected {expected}")]
    TypeMismatch {
        /// Question ID passed to the accessor.
        id: String,
        /// Type required by the accessor: boolean, choice, or score.
        expected: &'static str,
        /// Type of the stored answer: boolean, choice, or score.
        actual: &'static str,
    },
}
