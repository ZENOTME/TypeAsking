//! Live latency comparison. Makes 3 warmup + 8 requests per round (paid API calls).
//! Run with AI_GATEWAY_API_KEY set: cargo run --example latency -- 3
use std::time::Instant;
use openasking::{Asking, BoolQuestion, ChoiceQuestion, Error, ScoreQuestion, VercelConfig};

const STATE: &str = "A customer was charged twice for one order and requests a refund. The service is otherwise working.";

fn base() -> Asking {
    Asking::new(VercelConfig::new()).state(STATE)
}
fn boolean() -> BoolQuestion {
    BoolQuestion::new("refund", "Does the customer request a refund?")
}
fn choice() -> ChoiceQuestion {
    ChoiceQuestion::new("route", "Which team should handle the request?")
        .option("billing", "Payments and refunds")
        .option("technical", "Application bugs")
}
fn score() -> ScoreQuestion {
    ScoreQuestion::new("urgency", "How urgent is the request?")
        .level("low")
        .level("medium")
        .level("high")
}

async fn measured(request: Asking) -> Result<f64, Error> {
    let start = Instant::now();
    request.await?;
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rounds: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "3".into())
        .parse()?;
    if rounds == 0 {
        return Err("rounds must be positive".into());
    }
    // Establish up to three connections before the measured rounds.
    tokio::try_join!(
        base().bool_question(boolean()),
        base().choice_question(choice()),
        base().score_question(score()),
    )?;
    let names = [
        "single_bool",
        "batch_three",
        "parallel_three",
        "sequential_three",
    ];
    let mut samples: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::new());
    for round in 0..rounds {
        // Rotate execution order to reduce fixed-order warmup effects.
        for offset in 0..4 {
            let mode = (round + offset) % 4;
            let start = Instant::now();
            let individual_ms = match mode {
                0 => vec![measured(base().bool_question(boolean())).await?],
                1 => vec![
                    measured(
                        base()
                            .bool_question(boolean())
                            .choice_question(choice())
                            .score_question(score()),
                    )
                    .await?,
                ],
                2 => {
                    let (a, b, c) = tokio::try_join!(
                        measured(base().bool_question(boolean())),
                        measured(base().choice_question(choice())),
                        measured(base().score_question(score())),
                    )?;
                    vec![a, b, c]
                }
                3 => {
                    let a = measured(base().bool_question(boolean())).await?;
                    let b = measured(base().choice_question(choice())).await?;
                    let c = measured(base().score_question(score())).await?;
                    vec![a, b, c]
                }
                _ => unreachable!(),
            };
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            samples[mode].push(elapsed_ms);
            println!(
                "{}",
                serde_json::json!({"round": round + 1, "mode": names[mode], "wall_ms": elapsed_ms, "request_ms": individual_ms})
            );
        }
    }
    for (mode, values) in samples.iter_mut().enumerate() {
        values.sort_by(f64::total_cmp);
        let median = if values.len() % 2 == 0 {
            (values[values.len() / 2 - 1] + values[values.len() / 2]) / 2.0
        } else {
            values[values.len() / 2]
        };
        println!(
            "{}",
            serde_json::json!({"summary": names[mode], "rounds": rounds, "median_ms": median, "min_ms": values[0], "max_ms": values[values.len()-1]})
        );
    }
    Ok(())
}
