use typeasking::{Asking, BoolQuestion, ChoiceQuestion, ScoreQuestion};

#[tokio::main]
async fn main() -> Result<(), typeasking::Error> {
    let answers = Asking::new()
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
