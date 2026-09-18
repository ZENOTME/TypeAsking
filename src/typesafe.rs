//! Decode the native API into the common validated answer representation.
use crate::{Error, answer::WireResponse};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
struct Response {
    model: String,
    answers: HashMap<String, Answer>,
    usage: Usage,
}
#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        probabilities: HashMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        probabilities: HashMap<String, f64>,
        confidence: f64,
        legend: HashMap<String, String>,
    },
}

pub(crate) fn decode(bytes: &[u8]) -> Result<WireResponse, Error> {
    let response: Response = serde_json::from_slice(bytes)
        .map_err(|_| Error::InvalidResponse("malformed TypeSafe evaluation JSON".into()))?;
    let mut answers = serde_json::Map::new();
    let mut confidence = serde_json::Map::new();
    let mut legends = serde_json::Map::new();
    for (id, answer) in response.answers {
        let (value, certainty) = match answer {
            Answer::Noul { noul } => (json!({"type": "boolean", "probability": noul}), None),
            Answer::Choice {
                choice,
                probabilities,
                confidence,
            } => (
                json!({"type": "choice", "choice": choice, "probabilities": probabilities}),
                Some(confidence),
            ),
            Answer::Score {
                score,
                probabilities,
                confidence,
                legend,
            } => {
                // Each native level label must correspond to a probability entry.
                if legend.len() != probabilities.len()
                    || legend.keys().any(|key| !probabilities.contains_key(key))
                {
                    return Err(Error::InvalidResponse(
                        "TypeSafe legend and probability indexes differ".into(),
                    ));
                }
                legends.insert(id.clone(), json!(legend));
                (
                    json!({"type": "score", "score": score, "probabilities": probabilities}),
                    Some(confidence),
                )
            }
        };
        if let Some(c) = certainty {
            if !c.is_finite() || !(0.0..=1.0).contains(&c) {
                return Err(Error::InvalidResponse(
                    "TypeSafe confidence must be between 0 and 1".into(),
                ));
            }
            confidence.insert(id.clone(), json!(c));
        }
        answers.insert(id, value);
    }
    let normalized: Value = json!({
        "answers": answers,
        "usage": {"inputTokens": response.usage.input_tokens, "outputTokens": response.usage.output_tokens},
        "providerMetadata": {"typesafe": {"model": response.model, "confidence": confidence, "legend": legends}},
        // Native live responses round probabilities/scores to hundredths but do
        // not include Gateway's rounding descriptor. This is our compatibility
        // tolerance, not a claim that the server supplied rounding metadata.
        "rounding": {"probabilityDecimals": 2, "scoreDecimals": 2}
    });
    serde_json::from_value(normalized)
        .map_err(|_| Error::InvalidResponse("could not normalize TypeSafe response".into()))
}
