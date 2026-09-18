//! Compare live provider latency with identical mixed questions.
//! Requires TYPESAFE_API_KEY and AI_GATEWAY_API_KEY. Makes 2 + 2 * rounds requests.
use std::time::{Duration, Instant};
use openasking::{
    Asking, BoolQuestion, ChoiceQuestion, Config, ScoreQuestion, TypeSafeConfig, VercelConfig,
};

fn request(config: &Config) -> Asking {
    Asking::new(config)
        .state("A customer was charged twice for one order and requests a refund. The service is otherwise working.")
        .bool_question(BoolQuestion::new("refund", "Does the customer request a refund?"))
        .choice_question(ChoiceQuestion::new("route", "Which team should handle the request?")
            .option("billing", "Payments and refunds").option("technical", "Application bugs"))
        .score_question(ScoreQuestion::new("urgency", "How urgent is the request?")
            .level("low").level("medium").level("high"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rounds: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "10".into())
        .parse()?;
    if rounds == 0 {
        return Err("rounds must be positive".into());
    }
    for key in ["TYPESAFE_API_KEY", "AI_GATEWAY_API_KEY"] {
        if std::env::var(key).map_or(true, |v| v.trim().is_empty()) {
            return Err(format!("missing {key}").into());
        }
    }
    let configs: [Config; 2] = [
        TypeSafeConfig::new()
            .with_timeout(Duration::from_secs(30))
            .into(),
        VercelConfig::new()
            .with_timeout(Duration::from_secs(30))
            .into(),
    ];
    let names = ["typesafe", "vercel"];
    let mut samples: [Vec<f64>; 2] = Default::default();
    let mut errors = [0; 2];
    for round in 0..=rounds {
        for offset in 0..2 {
            let i = (round + offset) % 2;
            let request = request(&configs[i]);
            let start = Instant::now();
            let result = request.await;
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            let phase = if round == 0 { "first" } else { "warm" };
            match result {
                Ok(answers) => {
                    if round > 0 {
                        samples[i].push(ms);
                    }
                    println!(
                        "{}",
                        serde_json::json!({"provider":names[i],"phase":phase,"round":round,"wall_ms":ms,
                        "refund":answers.bool_answer("refund")?.probability_true,
                        "route":answers.choice_answer("route")?.choice,
                        "urgency":answers.score_answer("urgency")?.score})
                    );
                }
                Err(e) => {
                    errors[i] += 1;
                    println!(
                        "{}",
                        serde_json::json!({"provider":names[i],"phase":phase,"round":round,"wall_ms":ms,"error":e.to_string()})
                    );
                }
            }
        }
    }
    for i in 0..2 {
        let v = &mut samples[i];
        v.sort_by(f64::total_cmp);
        if !v.is_empty() {
            let n = v.len();
            let median = (v[(n - 1) / 2] + v[n / 2]) / 2.0;
            println!(
                "{}",
                serde_json::json!({"summary":names[i],"warm_successes":n,"errors":errors[i],
                "median_ms":median,"mean_ms":v.iter().sum::<f64>()/n as f64,"min_ms":v[0],"max_ms":v[n-1]})
            );
        }
    }
    if errors.iter().any(|&n| n != 0) {
        return Err("some requests failed; see results".into());
    }
    Ok(())
}
