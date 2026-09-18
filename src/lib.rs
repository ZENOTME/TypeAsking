#![doc = include_str!("../README.md")]

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
    /// Read `AI_GATEWAY_API_KEY` now. Missing credentials are reported on poll.
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
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.config().api_key = Some(key.into());
        self
    }

    /// Serialize and own a snapshot of state. Serialization errors surface on poll.
    /// State must be a nonempty string, object, or array (not null/bool/number).
    pub fn state(mut self, state: impl Serialize) -> Self {
        self.config().state = Some(serde_json::to_value(state));
        self
    }

    pub fn bool_question(mut self, question: BoolQuestion) -> Self {
        self.config().questions.push(Question::Bool(question));
        self
    }

    pub fn choice_question(mut self, question: ChoiceQuestion) -> Self {
        self.config().questions.push(Question::Choice(question));
        self
    }

    pub fn score_question(mut self, question: ScoreQuestion) -> Self {
        self.config().questions.push(Question::Score(question));
        self
    }

    /// Set the full evaluation endpoint, e.g. a local test server URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config().endpoint = endpoint.into();
        self
    }

    /// Total HTTP request timeout, including reading the response; defaults to 60s.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config().timeout = timeout;
        self
    }

    /// Reuse an explicitly configured HTTP connection pool across requests.
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
