# TypeAsking

A Rust SDK for typed Jev decisions through Vercel AI Gateway. `Asking` is itself a
`Future`: configure it and `.await` directly, without `ask()`, `send()`, or `finish()`.

```rust,no_run
use typeasking::{Asking, BoolQuestion, ChoiceQuestion, ScoreQuestion};

# async fn example() -> Result<(), typeasking::Error> {
let safe = BoolQuestion::new("safe", "Is the operation safe?")
    .when_true("It only reads data")
    .when_false("It changes or deletes data");
let route = ChoiceQuestion::new("route", "What should happen next?")
    .option("continue", "Continue execution")
    .option("retry", "Retry a transient failure");
let quality = ScoreQuestion::new("quality", "Assess code quality")
    .level("poor: contains bugs")
    .level("fair: correct but untested")
    .level("good: correct and tested");

let answers = Asking::new() // reads AI_GATEWAY_API_KEY
    .state(serde_json::json!({"operation": "read", "tests": "passed"}))
    .bool_question(safe)
    .choice_question(route)
    .score_question(quality)
    .await?;

println!("P(safe): {}", answers.bool_answer("safe")?.probability_true);
println!("Next: {}", answers.choice_answer("route")?.choice);
println!("Quality: {}", answers.score_answer("quality")?.score);
# Ok(())
# }
```

Use `.with_api_key(key)` to override the environment, even if it is unset. The SDK
does not load `.env` files. A Tokio runtime is required for HTTP I/O.

## Concurrency

Several questions sharing state are sent in one request. Independent `Asking`
objects can be joined, spawned, or fed to a bounded stream:

```rust,no_run
use typeasking::{Asking, BoolQuestion};
# async fn example() -> Result<(), typeasking::Error> {
let a = Asking::new().state("Build passed")
    .bool_question(BoolQuestion::new("passed", "Did the build pass?"));
let b = Asking::new().state("Tests failed")
    .bool_question(BoolQuestion::new("passed", "Did the tests pass?"));
let (a, b) = tokio::try_join!(a, b)?;
# Ok(())
# }
```

The default HTTP pool is shared. `.with_http_client(client.clone())` allows a
custom pool; `.with_timeout(Duration)` changes the 60-second request timeout.
Dropping a future cancels local work but does not guarantee cancellation of an
already accepted server request. There are no automatic retries or background tasks.
