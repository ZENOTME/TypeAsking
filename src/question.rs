use std::collections::HashSet;

use serde_json::{Value, json};

use crate::Error;

/// A question estimating P(true). Criteria are optional.
#[derive(Clone, Debug)]
pub struct BoolQuestion {
    pub(crate) id: String,
    instructions: String,
    when_true: Option<String>,
    when_false: Option<String>,
}

impl BoolQuestion {
    pub fn new(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instructions: instructions.into(),
            when_true: None,
            when_false: None,
        }
    }

    pub fn when_true(mut self, description: impl Into<String>) -> Self {
        self.when_true = Some(description.into());
        self
    }

    pub fn when_false(mut self, description: impl Into<String>) -> Self {
        self.when_false = Some(description.into());
        self
    }
}

/// A single choice among named options. Keys must be unique and nonempty.
#[derive(Clone, Debug)]
pub struct ChoiceQuestion {
    pub(crate) id: String,
    instructions: String,
    pub(crate) options: Vec<(String, String)>,
}

impl ChoiceQuestion {
    pub fn new(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instructions: instructions.into(),
            options: Vec::new(),
        }
    }

    pub fn option(mut self, key: impl Into<String>, description: impl Into<String>) -> Self {
        self.options.push((key.into(), description.into()));
        self
    }
}

/// A fractional score on ordered levels, from 0 to `levels.len() - 1`.
#[derive(Clone, Debug)]
pub struct ScoreQuestion {
    pub(crate) id: String,
    instructions: String,
    pub(crate) levels: Vec<String>,
}

impl ScoreQuestion {
    pub fn new(id: impl Into<String>, instructions: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instructions: instructions.into(),
            levels: Vec::new(),
        }
    }

    /// Append a level, ordered from lowest to highest. At least two are required.
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
