#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

mod answer;
mod error;
mod question;
mod transport;

pub use answer::{Answers, BoolAnswer, ChoiceAnswer, ScoreAnswer, Usage};
pub use error::Error;
pub use question::{BoolQuestion, ChoiceQuestion, ScoreQuestion};

use question::Question;
use serde::Serialize;
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

/// A single lazy evaluation request. Configure it, then await it directly.
///
/// Requires a Tokio runtime for network I/O. All input data is owned; the future
/// is `Send` and can be used in `tokio::spawn`, `try_join!`, or buffered streams.
/// Dropping it stops local polling; the server may already have accepted work.
/// Each instance performs one evaluation, using the `typesafe-ai/jev` model.
/// There are no automatic retries. Independent requests share the default HTTP pool.
///
/// # Errors
///
/// Awaiting returns [`Error::Configuration`] for missing credentials, empty or
/// invalid state, no questions, duplicate IDs, or invalid question settings.
/// Serialization failures become [`Error::StateSerialization`]. These checks
/// happen on the first poll, before network I/O. Transport failures, unsuccessful
/// HTTP responses and invalid answers return the corresponding [`Error`] variant.
/// Polling again after completion returns [`Error::AlreadyCompleted`].
///
/// # Panics
///
/// Calling a configuration method after the request has been polled panics.
/// Network execution requires an active Tokio runtime with I/O and time enabled.
///
/// # Examples
///
/// ```no_run
/// use typeasking::{Asking, BoolQuestion, Error};
///
/// async fn check() -> Result<f64, Error> {
///     let answers = Asking::new()
///         .state("The build passed.")
///         .bool_question(BoolQuestion::new("passed", "Did the build pass?"))
///         .await?;
///     Ok(answers.bool_answer("passed")?.probability_true)
/// }
/// ```
#[must_use = "Asking does nothing until polled or awaited"]
pub struct Asking {
    execution: Execution,
}

type ResponseFuture = Pin<Box<dyn Future<Output = Result<Answers, Error>> + Send>>;

enum Execution {
    Building(Box<Config>),
    Running(ResponseFuture),
    Done,
}

pub(crate) struct Config {
    api_key: Option<String>,
    state: Option<Result<Value, serde_json::Error>>,
    questions: Vec<Question>,
    endpoint: String,
    timeout: Duration,
    client: Option<reqwest::Client>,
}

impl Default for Asking {
    fn default() -> Self {
        Self::new()
    }
}

impl Asking {
    /// Create a lazy request and read `AI_GATEWAY_API_KEY` from the environment.
    ///
    /// Does not load `.env` files or perform network I/O. A missing or non-Unicode
    /// key is treated as absent; use [`Self::with_api_key`] to supply one explicitly.
    /// Missing credentials produce an error on poll, not during construction.
    /// Defaults to the Vercel evaluation endpoint and a 60-second timeout.
    pub fn new() -> Self {
        Self {
            execution: Execution::Building(Box::new(Config {
                api_key: std::env::var("AI_GATEWAY_API_KEY").ok(),
                state: None,
                questions: Vec::new(),
                endpoint: "https://ai-gateway.vercel.sh/v4/ai/evaluation-model".into(),
                timeout: Duration::from_secs(60),
                client: None,
            })),
        }
    }

    /// Override the environment key, including when the environment is unset.
    ///
    /// Empty keys and invalid HTTP header values are rejected on the first poll.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.config().api_key = Some(key.into());
        self
    }

    /// Serialize and own a snapshot of state. Serialization errors surface on poll.
    /// State must be a nonempty string, object, or array (not null/bool/number).
    /// Whitespace-only strings are empty. Calling this again replaces the snapshot.
    /// Borrowed input is only borrowed for this call; it can be changed or dropped
    /// afterwards without affecting the request.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn state(mut self, state: impl Serialize) -> Self {
        self.config().state = Some(serde_json::to_value(state));
        self
    }

    /// Append a Boolean question to this request, taking ownership of its definition.
    ///
    /// The ID must be unique across all question types in this request.
    /// Validation happens on the first poll; this method does not send a request.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn bool_question(mut self, question: BoolQuestion) -> Self {
        self.config().questions.push(Question::Bool(question));
        self
    }

    /// Append a Choice question to this request, taking ownership of its definition.
    ///
    /// The ID must be unique across all question types in this request.
    /// Validation happens on the first poll; this method does not send a request.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn choice_question(mut self, question: ChoiceQuestion) -> Self {
        self.config().questions.push(Question::Choice(question));
        self
    }

    /// Append a Score question to this request, taking ownership of its definition.
    ///
    /// The ID must be unique across all question types in this request.
    /// Validation happens on the first poll; this method does not send a request.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn score_question(mut self, question: ScoreQuestion) -> Self {
        self.config().questions.push(Question::Score(question));
        self
    }

    /// Set the full evaluation endpoint, e.g. a local test server URL.
    ///
    /// The default is `https://ai-gateway.vercel.sh/v4/ai/evaluation-model`.
    /// No path is appended. The URL must use HTTP(S), have a host and contain no
    /// username/password. The API key is sent to this endpoint.
    /// This adapter follows the experimental AI Gateway evaluation v4 protocol.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config().endpoint = endpoint.into();
        self
    }

    /// Set the total HTTP request timeout, including reading the response.
    ///
    /// Defaults to 60 seconds. Zero is rejected on the first poll.
    /// A timeout returns [`Error::Transport`]; inspect its source's `is_timeout()`.
    /// This request-level timeout also applies to a custom HTTP client.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config().timeout = timeout;
        self
    }

    /// Use a custom HTTP client instead of the default shared connection pool.
    ///
    /// Pass `client.clone()` to share a custom pool between requests. Redirect and
    /// proxy behavior follow the supplied client's settings; the default client
    /// does not follow redirects. Per-request credentials and headers are still set
    /// by TypeAsking.
    ///
    /// # Panics
    ///
    /// Panics if this request has already been polled.
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.config().client = Some(client);
        self
    }

    fn config(&mut self) -> &mut Config {
        match &mut self.execution {
            Execution::Building(config) => config,
            _ => panic!("cannot configure Asking after it has been polled"),
        }
    }
}

impl Future for Asking {
    type Output = Result<Answers, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if matches!(this.execution, Execution::Building(_)) {
            let Execution::Building(config) =
                std::mem::replace(&mut this.execution, Execution::Done)
            else {
                unreachable!()
            };
            // Validation is synchronous on first poll, before networking or runtime use.
            match transport::prepare(*config) {
                Ok(future) => this.execution = Execution::Running(future),
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
        match &mut this.execution {
            Execution::Running(future) => match future.as_mut().poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(result) => {
                    this.execution = Execution::Done;
                    Poll::Ready(result)
                }
            },
            Execution::Done => Poll::Ready(Err(Error::AlreadyCompleted)),
            Execution::Building(_) => unreachable!(),
        }
    }
}
