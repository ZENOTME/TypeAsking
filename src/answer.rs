use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::{Error, question::Question};

#[derive(Debug, Clone, PartialEq)]
pub struct BoolAnswer {
    /// Model-estimated P(true), not confidence in whichever outcome was selected.
    pub probability_true: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceAnswer {
    pub choice: String,
    /// Empty if the provider omitted the distribution.
    pub probabilities: HashMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoreAnswer {
    pub score: f64,
    /// In level order, including indexes >= 10. Empty if omitted by the provider.
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
#[derive(Debug, Clone)]
pub struct Answers {
    answers: HashMap<String, Answer>,
    /// Provider metadata, including optional TypeSafe confidence. Never synthesized.
    pub metadata: Value,
    pub usage: Option<Usage>,
    pub warnings: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

impl Answers {
    pub fn bool_answer(&self, id: &str) -> Result<&BoolAnswer, Error> {
        match self.get(id)? {
            Answer::Bool(a) => Ok(a),
            other => Err(mismatch(id, "boolean", other)),
        }
    }

    pub fn choice_answer(&self, id: &str) -> Result<&ChoiceAnswer, Error> {
        match self.get(id)? {
            Answer::Choice(a) => Ok(a),
            other => Err(mismatch(id, "choice", other)),
        }
    }

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
    Bool { probability: f64 },
    #[serde(rename = "choice")]
    Choice {
        choice: String,
        probabilities: Option<HashMap<String, f64>>,
    },
    #[serde(rename = "score")]
    Score {
        score: f64,
        probabilities: Option<HashMap<String, f64>>,
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
