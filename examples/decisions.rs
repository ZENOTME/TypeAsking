use typeasking::{
    Asking, BoolQuestion, ChoiceQuestion, Config, ScoreQuestion, TypeSafeConfig, VercelConfig,
};

#[tokio::main]
async fn main() -> Result<(), typeasking::Error> {
    let config: Config = if std::env::args().any(|a| a == "--typesafe") {
        TypeSafeConfig::new().into()
    } else {
        VercelConfig::new().into()
    };
    let answers = Asking::new(config)
        .state(
            serde_json::json!({"event": "The customer was charged twice and requests a refund."}),
        )
        .bool_question(BoolQuestion::new(
            "refund",
            "Does the customer want a refund?",
        ))
        .choice_question(
            ChoiceQuestion::new("route", "Which team should handle this?")
                .option("billing", "Payments and refunds")
                .option("technical", "Application bugs"),
        )
        .score_question(
            ScoreQuestion::new("urgency", "How urgent is this request?")
                .level("low")
                .level("medium")
                .level("high"),
        )
        .await?;
    println!(
        "Refund probability: {}",
        answers.bool_answer("refund")?.probability_true
    );
    println!("Route: {}", answers.choice_answer("route")?.choice);
    println!("Urgency: {}", answers.score_answer("urgency")?.score);
    Ok(())
}
