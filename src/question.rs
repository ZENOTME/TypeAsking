use std::collections::HashSet;

use serde_json::{Value, json};

use crate::Error;

/// A question estimating P(true). Criteria are optional.
///
/// The result is [`crate::BoolAnswer`]; callers choose their own decision threshold.
///
/// ```
/// use typeasking::BoolQuestion;
///
/// let safe = BoolQuestion::new("safe", "Is the operation safe?")
///     .when_true("Read-only access")
///     .when_false("Data is modified or deleted");
/// ```
#[derive(Clone, Debug)]
pub struct BoolQuestion {
    pub(crate) id: String,
    instructions: String,
    when_true: Option<String>,
    when_false: Option<String>,
}

impl BoolQuestion {
    /// Create a Boolean question with a stable ID and instructions.
    ///
    /// ID and instructions must not be empty or whitespace-only. The ID must be
    /// unique within the request and is used to retrieve the answer. Validation
    /// is deferred until the containing [`crate::Asking`] is first polled.
    pub fn new(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instructions: instructions.into(),
            when_true: None,
            when_false: None,
        }
    }

    /// Describe what counts as true. Calling again replaces the previous criterion.
    ///
    /// Optional; when supplied, the description must not be blank.
    pub fn when_true(mut self, description: impl Into<String>) -> Self {
        self.when_true = Some(description.into());
        self
    }

    /// Describe what counts as false. Calling again replaces the previous criterion.
    ///
    /// Optional; when supplied, the description must not be blank.
    pub fn when_false(mut self, description: impl Into<String>) -> Self {
        self.when_false = Some(description.into());
        self
    }
}

/// A single choice among named options. Keys must be unique and nonempty.
///
/// Use several Boolean questions when multiple options may independently apply.
///
/// ```
/// use typeasking::ChoiceQuestion;
///
/// let route = ChoiceQuestion::new("route", "What should happen next?")
///     .option("continue", "Continue execution")
///     .option("retry", "Retry a transient error");
/// ```
#[derive(Clone, Debug)]
pub struct ChoiceQuestion {
    pub(crate) id: String,
    instructions: String,
    pub(crate) options: Vec<(String, String)>,
}

impl ChoiceQuestion {
    /// Create a choice question with no options yet.
    ///
    /// ID and instructions must not be empty or whitespace-only. The ID must be
    /// unique within the request and is used to retrieve the answer. Validation
    /// is deferred until the containing [`crate::Asking`] is first polled.
    pub fn new(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instructions: instructions.into(),
            options: Vec::new(),
        }
    }

    /// Append an option with a stable key and its meaning.
    ///
    /// At least one option is required. Keys must be unique within this question;
    /// both key and description must not be blank. Duplicate keys produce a
    /// configuration error on poll rather than replacing an earlier option.
    /// The selected key is returned unchanged in [`crate::ChoiceAnswer::choice`].
    pub fn option(mut self, key: impl Into<String>, description: impl Into<String>) -> Self {
        self.options.push((key.into(), description.into()));
        self
    }
}

/// A fractional score on ordered levels, from 0 to `levels.len() - 1`.
///
/// Levels describe an ordered rubric, not arbitrary numerical endpoints.
///
/// ```
/// use typeasking::ScoreQuestion;
///
/// let quality = ScoreQuestion::new("quality", "Assess code quality")
///     .level("poor: contains bugs")
///     .level("fair: correct but untested")
///     .level("good: correct and tested");
/// // The resulting score can be 1.7 on this 0..=2 scale.
/// ```
#[derive(Clone, Debug)]
pub struct ScoreQuestion {
    pub(crate) id: String,
    instructions: String,
    pub(crate) levels: Vec<String>,
}

impl ScoreQuestion {
    /// Create a score question with no levels yet.
    ///
    /// ID and instructions must not be empty or whitespace-only. The ID must be
    /// unique within the request and is used to retrieve the answer. Validation
    /// is deferred until the containing [`crate::Asking`] is first polled.
    pub fn new(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instructions: instructions.into(),
            levels: Vec::new(),
        }
    }

    /// Append a level, ordered from lowest to highest. At least two are required.
    ///
    /// Descriptions must not be blank. Levels receive indexes starting at zero;
    /// three levels define scores in `[0, 2]`, including fractional values.
    pub fn level(mut self, description: impl Into<String>) -> Self {
        self.levels.push(description.into());
        self
    }
}

pub(crate) enum Question {
    Bool(BoolQuestion),
    Choice(ChoiceQuestion),
    Score(ScoreQuestion),
}

impl Question {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Bool(q) => &q.id,
            Self::Choice(q) => &q.id,
            Self::Score(q) => &q.id,
        }
    }

    pub(crate) fn encode(&self) -> Result<Value, Error> {
        let instructions = match self {
            Self::Bool(q) => &q.instructions,
            Self::Choice(q) => &q.instructions,
            Self::Score(q) => &q.instructions,
        };
        nonempty(self.id(), "question ID")?;
        nonempty(instructions, "question instructions")?;
        Ok(match self {
            Self::Bool(q) => {
                let mut value = json!({"type": "boolean", "instructions": instructions});
                let mut criteria = serde_json::Map::new();
                for (key, description) in [("true", &q.when_true), ("false", &q.when_false)] {
                    if let Some(description) = description {
                        nonempty(description, "boolean criterion")?;
                        criteria.insert(key.into(), json!(description));
                    }
                }
                if !criteria.is_empty() {
                    value["criteria"] = Value::Object(criteria);
                }
                value
            }
            Self::Choice(q) => {
                if q.options.is_empty() {
                    return Err(config("choice requires at least one option"));
                }
                let mut keys = HashSet::new();
                let mut criteria = serde_json::Map::new();
                for (key, description) in &q.options {
                    nonempty(key, "option key")?;
                    nonempty(description, "option description")?;
                    if !keys.insert(key) {
                        return Err(config("duplicate choice option key"));
                    }
                    criteria.insert(key.clone(), json!(description));
                }
                json!({"type": "choice", "instructions": instructions, "criteria": criteria})
            }
            Self::Score(q) => {
                if q.levels.len() < 2 {
                    return Err(config("score requires at least two levels"));
                }
                for level in &q.levels {
                    nonempty(level, "score level")?;
                }
                json!({"type": "score", "instructions": instructions, "criteria": q.levels})
            }
        })
    }
}

pub(crate) fn config(message: &str) -> Error {
    Error::Configuration(message.into())
}

pub(crate) fn nonempty(value: &str, name: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(config(&format!("{name} must not be empty")))
    } else {
        Ok(())
    }
}
