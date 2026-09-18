use openasking::{Asking, BoolQuestion, VercelConfig};

#[tokio::main]
async fn main() -> Result<(), openasking::Error> {
    let question = BoolQuestion::new("passed", "Did the build pass?");
    let a = Asking::new(VercelConfig::new())
        .state("Build succeeded, exit 0")
        .bool_question(question.clone());
    let b = Asking::new(VercelConfig::new())
        .state("Build failed, exit 1")
        .bool_question(question);
    let (a, b) = tokio::try_join!(a, b)?;
    println!("A: {}", a.bool_answer("passed")?.probability_true);
    println!("B: {}", b.bool_answer("passed")?.probability_true);
    Ok(())
}
