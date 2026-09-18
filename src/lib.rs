#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

mod answer;
mod config;
mod error;
mod question;
mod transport;
mod typesafe;

pub use answer::{Answers, BoolAnswer, ChoiceAnswer, ScoreAnswer, Usage};
pub use config::{Config, TypeSafeConfig, VercelConfig};
pub use error::Error;
pub use question::{BoolQuestion, ChoiceQuestion, ScoreQuestion};

use question::Question;
use serde::Serialize;
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// A single lazy evaluation request. Configure it, then await it directly.
///
/// Requires a Tokio runtime for network I/O. All input data is owned; the future
/// is `Send` and can be used in `tokio::spawn`, `try_join!`, or buffered streams.
/// Dropping it stops local polling; the server may already have accepted work.
/// Each instance performs one evaluation using its provider configuration.
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
/// use openasking::{Asking, BoolQuestion, Error, VercelConfig};
///
/// async fn check() -> Result<f64, Error> {
///     let answers = Asking::new(VercelConfig::new())
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
    Building(Box<Request>),
    Running(ResponseFuture),
    Done,
}

pub(crate) struct Request {
    provider: Config,
    state: Option<Result<Value, serde_json::Error>>,
    questions: Vec<Question>,
}

impl Asking {
    /// Create an independent lazy request with explicit provider settings.
    ///
    /// Accepts [`VercelConfig`], [`TypeSafeConfig`] or [`Config`], owned or borrowed.
    /// Borrowed settings are cloned; this future never borrows the configuration.
    /// Credentials were read when the configuration was created. No networking
    /// happens until polling. Reuse the configuration for concurrent requests.
    ///
    /// ```
    /// use openasking::{Asking, TypeSafeConfig, BoolQuestion};
    ///
    /// let config = TypeSafeConfig::new();
    /// let first = Asking::new(&config).state("Build passed")
    ///     .bool_question(BoolQuestion::new("passed", "Did the build pass?"));
    /// let second = Asking::new(&config).state("Build failed")
    ///     .bool_question(BoolQuestion::new("passed", "Did the build pass?"));
    /// // Poll/await the two independent requests when ready.
    /// ```
    pub fn new(config: impl Into<Config>) -> Self {
        Self {
            execution: Execution::Building(Box::new(Request {
                provider: config.into(),
                state: None,
                questions: Vec::new(),
            })),
        }
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

    fn config(&mut self) -> &mut Request {
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
