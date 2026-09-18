# TypeAsking

A Rust SDK for typed Jev decisions through TypeSafe or Vercel AI Gateway. `Asking` is itself a
`Future`: configure it and `.await` directly, without `ask()`, `send()`, or `finish()`.

`new()` and `default()` read the API key from the process environment when the
config is created:

| Config | API key environment variable | Default model |
| --- | --- | --- |
| `TypeSafeConfig` | `TYPESAFE_API_KEY` | `jev-latest` |
| `VercelConfig` | `AI_GATEWAY_API_KEY` | `typesafe-ai/jev` |

Set the variable for your chosen provider before running your program:

```sh
# Direct TypeSafe API
export TYPESAFE_API_KEY="your-typesafe-key"
# Vercel AI Gateway
export AI_GATEWAY_API_KEY="your-gateway-key"
```

`.with_api_key("...")` overrides the environment value. The SDK does not load
`.env` or shell startup files, and does not fall back to the other provider's key.
If the key is missing or empty, awaiting the request returns `Error::Configuration`.

```rust,no_run
use typeasking::{Asking, BoolQuestion, ChoiceQuestion, ScoreQuestion, TypeSafeConfig};

async fn example() -> Result<(), typeasking::Error> {
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

    let config = TypeSafeConfig::new(); // reads TYPESAFE_API_KEY
    let answers = Asking::new(&config)
        .state(serde_json::json!({"operation": "read", "tests": "passed"}))
        .bool_question(safe)
        .choice_question(route)
        .score_question(quality)
        .await?;

    println!("P(safe): {}", answers.bool_answer("safe")?.probability_true);
    println!("Next: {}", answers.choice_answer("route")?.choice);
    println!("Quality: {}", answers.score_answer("quality")?.score);
    Ok(())
}
```

Choose the provider in the configuration; question and answer APIs stay the same:

```rust
use typeasking::{TypeSafeConfig, VercelConfig};

let direct = TypeSafeConfig::new(); // TYPESAFE_API_KEY, model jev-latest
let gateway = VercelConfig::new();  // AI_GATEWAY_API_KEY, model typesafe-ai/jev
let explicit = TypeSafeConfig::new().with_api_key("your-key");
```

Pass either config (or a reference) to `Asking::new(config)`. The SDK does not load
`.env` or shell startup files. A Tokio runtime is required for HTTP I/O.

In 0.2, settings moved from `Asking` to the provider config. To migrate from 0.1,
replace `Asking::new().with_api_key(key)` with
`Asking::new(VercelConfig::new().with_api_key(key))`.

## Concurrency

Several questions sharing state are sent in one request. Independent `Asking`
objects can be joined, spawned, or fed to a bounded stream:

```rust,no_run
use typeasking::{Asking, BoolQuestion, TypeSafeConfig};

async fn example() -> Result<(), typeasking::Error> {
    let config = TypeSafeConfig::new();
    let a = Asking::new(&config).state("Build passed")
        .bool_question(BoolQuestion::new("passed", "Did the build pass?"));
    let b = Asking::new(&config).state("Tests failed")
        .bool_question(BoolQuestion::new("passed", "Did the tests pass?"));
    let (a, b) = tokio::try_join!(a, b)?;
    Ok(())
}
```

The default HTTP pool is shared. On either config, `.with_http_client(client.clone())`
sets a custom pool; `.with_timeout(Duration)` changes the 60-second request timeout.
`.with_model(name)` selects a provider model; `.with_endpoint(url)` overrides its
full URL without changing the protocol.
Dropping a future cancels local work but does not guarantee cancellation of an
already accepted server request. There are no automatic retries or background tasks.

Licensed under the [MIT License](https://github.com/ZENOTME/TypeAsking/blob/main/LICENSE).
