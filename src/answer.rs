use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::{Error, question::Question};

/// The probability of the true outcome for a Boolean question.
#[derive(Debug, Clone, PartialEq)]
pub struct BoolAnswer {
    /// Model-estimated P(true), not confidence in whichever outcome was selected.
    pub probability_true: f64,
}

/// A selected option key and, when available, its full probability distribution.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceAnswer {
    /// Selected key, matching an option declared by the original question.
    pub choice: String,
    /// Probability of each declared option, in `[0, 1]`.
    ///
    /// Empty if omitted by the provider; absence does not mean zero probability.
    pub probabilities: HashMap<String, f64>,
}

/// A fractional rubric score and optional distribution over ordered levels.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreAnswer {
    /// Position in `[0, number of levels - 1]`; it need not be an integer.
    pub score: f64,
    /// Probabilities in the original level order, indexed numerically from zero.
    ///
    /// Empty if omitted by the provider. Otherwise includes every level; values
    /// are in `[0, 1]` and sum to one within the provider's declared rounding.
    pub probabilities: Vec<f64>,
}

#[derive(Debug, Clone)]
enum Answer {
    Bool(BoolAnswer),
    Choice(ChoiceAnswer),
    Score(ScoreAnswer),
}

impl Answer {
    fn kind(&self) -> &'static str {
        match self {
            Self::Bool(_) => "boolean",
            Self::Choice(_) => "choice",
            Self::Score(_) => "score",
        }
    }
}

/// All answers to one request, indexed by the original question IDs.
///
/// Obtain this value by awaiting [`crate::Asking`]. Accessors borrow the stored
/// answers without cloning them. Successful requests contain one answer per ID.
///
/// ```no_run
/// use openasking::{Answers, Error};
///
/// fn inspect(answers: &Answers) -> Result<(), Error> {
///     let safe = answers.bool_answer("safe")?;
///     let route = answers.choice_answer("route")?;
///     let quality = answers.score_answer("quality")?;
///     println!("{} {} {}", safe.probability_true, route.choice, quality.score);
///     Ok(())
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Answers {
    answers: HashMap<String, Answer>,
    /// Provider metadata, or JSON null if none was supplied.
    ///
    /// Optional TypeSafe confidence is retained here (typically under
    /// `typesafe.confidence`), separate from probabilities and never synthesized.
    /// Direct TypeSafe responses also retain the resolved model under
    /// `typesafe.model` and per-question level labels under `typesafe.legend`.
    pub metadata: Value,
    /// Token usage, when reported. Missing counts are not treated as zero.
    pub usage: Option<Usage>,
    /// Provider warnings preserved as JSON objects; empty when none are reported.
    pub warnings: Vec<Value>,
}

/// Provider-reported token counts. Each count can be independently absent.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Number of input tokens, or None if not reported.
    pub input_tokens: Option<u64>,
    /// Number of output tokens, or None if not reported.
    pub output_tokens: Option<u64>,
}

impl Answers {
    /// Borrow the Boolean answer identified by its original question ID.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] if the ID is absent, or
    /// [`Error::TypeMismatch`] if it belongs to a different question type.
    pub fn bool_answer(&self, id: &str) -> Result<&BoolAnswer, Error> {
        match self.get(id)? {
            Answer::Bool(a) => Ok(a),
            other => Err(mismatch(id, "boolean", other)),
        }
    }

    /// Borrow the choice answer identified by its original question ID.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] if the ID is absent, or
    /// [`Error::TypeMismatch`] if it belongs to a different question type.
    pub fn choice_answer(&self, id: &str) -> Result<&ChoiceAnswer, Error> {
        match self.get(id)? {
            Answer::Choice(a) => Ok(a),
            other => Err(mismatch(id, "choice", other)),
        }
    }

    /// Borrow the score answer identified by its original question ID.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] if the ID is absent, or
    /// [`Error::TypeMismatch`] if it belongs to a different question type.
    pub fn score_answer(&self, id: &str) -> Result<&ScoreAnswer, Error> {
        match self.get(id)? {
            Answer::Score(a) => Ok(a),
            other => Err(mismatch(id, "score", other)),
        }
    }

    fn get(&self, id: &str) -> Result<&Answer, Error> {
        self.answers
            .get(id)
            .ok_or_else(|| Error::NotFound(id.into()))
    }
}

fn mismatch(id: &str, expected: &'static str, answer: &Answer) -> Error {
    Error::TypeMismatch {
        id: id.into(),
        expected,
        actual: answer.kind(),
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WireAnswer {
    #[serde(rename = "boolean")]
    Bool { probability: serde_json::Number },
    #[serde(rename = "choice")]
    Choice {
        choice: String,
        probabilities: Option<HashMap<String, serde_json::Number>>,
    },
    #[serde(rename = "score")]
    Score {
        score: serde_json::Number,
        probabilities: Option<HashMap<String, serde_json::Number>>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireResponse {
    answers: HashMap<String, WireAnswer>,
    #[serde(default)]
    provider_metadata: Value,
    usage: Option<Usage>,
    #[serde(default)]
    warnings: Vec<Value>,
    #[serde(default)]
    rounding: Rounding,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rounding {
    probability_decimals: Option<u32>,
    score_decimals: Option<u32>,
}

fn tolerance(decimals: Option<u32>) -> f64 {
    decimals.map_or(1e-8, |n| 0.5 * 10_f64.powi(-(n.min(15) as i32)) + 1e-8)
}

fn invalid(message: &str) -> Error {
    Error::InvalidResponse(message.into())
}

fn probability(value: f64) -> Result<(), Error> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(invalid("probability must be finite and between 0 and 1"))
    }
}

fn distribution(values: &[f64], rounding: &Rounding) -> Result<(), Error> {
    for &p in values {
        probability(p)?;
    }
    if (values.iter().sum::<f64>() - 1.0).abs()
        > values.len() as f64 * tolerance(rounding.probability_decimals)
    {
        return Err(invalid(
            "distribution does not sum to 1 within declared rounding",
        ));
    }
    Ok(())
}

impl WireResponse {
    pub(crate) fn decode(mut self, questions: &[Question]) -> Result<Answers, Error> {
        if self.answers.len() != questions.len() {
            return Err(invalid("answer IDs do not match question IDs"));
        }
        let mut answers = HashMap::new();
        for question in questions {
            let wire = self
                .answers
                .remove(question.id())
                .ok_or_else(|| invalid("missing requested answer"))?;
            let answer = match (question, wire) {
                (Question::Bool(_), WireAnswer::Bool { probability: p }) => {
                    let p = finite_number(&p)?;
                    probability(p)?;
                    Answer::Bool(BoolAnswer {
                        probability_true: p,
                    })
                }
                (
                    Question::Choice(q),
                    WireAnswer::Choice {
                        choice,
                        probabilities,
                    },
                ) => {
                    let probabilities = probabilities.map(convert_distribution).transpose()?;
                    if !q.options.iter().any(|(key, _)| key == &choice) {
                        return Err(invalid("choice is not a declared option"));
                    }
                    if let Some(p) = &probabilities {
                        if p.len() != q.options.len()
                            || q.options.iter().any(|(key, _)| !p.contains_key(key))
                        {
                            return Err(invalid("choice distribution keys do not match options"));
                        }
                        distribution(&p.values().copied().collect::<Vec<_>>(), &self.rounding)?;
                        if p.values().any(|&v| {
                            v > p[&choice] + tolerance(self.rounding.probability_decimals)
                        }) {
                            return Err(invalid("selected choice is not a most probable option"));
                        }
                    }
                    Answer::Choice(ChoiceAnswer {
                        choice,
                        probabilities: probabilities.unwrap_or_default(),
                    })
                }
                (
                    Question::Score(q),
                    WireAnswer::Score {
                        score,
                        probabilities,
                    },
                ) => {
                    let score = finite_number(&score)?;
                    let probabilities = probabilities.map(convert_distribution).transpose()?;
                    if !score.is_finite() || score < 0.0 || score > (q.levels.len() - 1) as f64 {
                        return Err(invalid("score is outside the declared scale"));
                    }
                    let mut ordered = Vec::new();
                    if let Some(p) = probabilities {
                        if p.len() != q.levels.len() {
                            return Err(invalid("score distribution has wrong length"));
                        }
                        for i in 0..q.levels.len() {
                            ordered.push(
                                *p.get(&i.to_string())
                                    .ok_or_else(|| invalid("invalid score distribution index"))?,
                            );
                        }
                        distribution(&ordered, &self.rounding)?;
                        let mean: f64 = ordered.iter().enumerate().map(|(i, p)| i as f64 * p).sum();
                        let index_sum = (q.levels.len() * (q.levels.len() - 1) / 2) as f64;
                        let epsilon = tolerance(self.rounding.score_decimals)
                            + index_sum * tolerance(self.rounding.probability_decimals);
                        if (mean - score).abs() > epsilon {
                            return Err(invalid("score differs from distribution mean"));
                        }
                    }
                    Answer::Score(ScoreAnswer {
                        score,
                        probabilities: ordered,
                    })
                }
                _ => return Err(invalid("answer type does not match question type")),
            };
            answers.insert(question.id().into(), answer);
        }
        Ok(Answers {
            answers,
            metadata: self.provider_metadata,
            usage: self.usage,
            warnings: self.warnings,
        })
    }
}

// Keep JSON numbers in the wire representation so internally tagged enums
// remain compatible with serde_json/arbitrary_precision, including exponents.
// Convert only at the boundary to the public f64-based answer types.
pub(crate) fn finite_number(number: &serde_json::Number) -> Result<f64, Error> {
    number
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid("answer number is not representable as a finite f64"))
}

fn convert_distribution(
    values: HashMap<String, serde_json::Number>,
) -> Result<HashMap<String, f64>, Error> {
    values
        .into_iter()
        .map(|(key, value)| Ok((key, finite_number(&value)?)))
        .collect()
}
