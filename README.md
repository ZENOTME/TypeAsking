# openasking

openasking is a Rust SDK that turns context into probabilities, choices, and scores
using TypeSafe's Jev model. Provide context, define your questions, and await the
answers through either the TypeSafe API or Vercel AI Gateway.

For example, given a customer support message, you can ask whether the customer
wants a refund, which team should handle it, and how urgent it is—all in one request.

## Quick start

### 1. Add dependencies

Add openasking from this repository and Tokio for the async runtime:

```sh
cargo add openasking --git https://github.com/ZENOTME/openasking.git
cargo add tokio --features macros,rt-multi-thread
```

Use a current stable Rust toolchain; the crate uses Rust edition 2024.

### 2. Set an API key

The example below uses the direct TypeSafe API:

```sh
export TYPESAFE_API_KEY="your-typesafe-key"
```

To use Vercel AI Gateway instead, see [Providers](#providers).

### 3. Ask questions

Put this in `src/main.rs` and run `cargo run`:

```rust,no_run
use openasking::{Asking, BoolQuestion, ChoiceQuestion, ScoreQuestion, TypeSafeConfig};

#[tokio::main]
async fn main() -> Result<(), openasking::Error> {
    let answers = Asking::new(TypeSafeConfig::new())
        .state("The customer was charged twice and requests a refund.")
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

    println!("Refund probability: {}", answers.bool_answer("refund")?.probability_true);
    println!("Route: {}", answers.choice_answer("route")?.choice);
    println!("Urgency: {}", answers.score_answer("urgency")?.score);
    Ok(())
}
```

The string passed to `.state(...)` is the context for every question in this request.
Each question has a unique ID, such as `"route"`, which you use to retrieve its answer.
Awaiting sends the request and validates the response before returning `Answers`.

State can also be a serializable struct, JSON object, or array. It must serialize
to a nonempty string, object, or array. For `serde_json::json!`, add `serde_json`
as a dependency. The SDK owns a snapshot of the state, so later changes to the
original value do not affect the request.

## Question types

| Question | Define | Read |
| --- | --- | --- |
| `BoolQuestion` | A yes/no question, optionally with `.when_true(...)` and `.when_false(...)` criteria | `BoolAnswer::probability_true`, a model-estimated probability in `[0, 1]` |
| `ChoiceQuestion` | Named options using `.option(key, description)` | `ChoiceAnswer::choice`, one of your declared keys |
| `ScoreQuestion` | An ordered rubric using `.level(description)`, from lowest to highest | `ScoreAnswer::score`, a fractional position on the rubric |

A Boolean answer is a probability, not a `bool`; your application chooses any
threshold it needs. Three score levels define a range of `0..=2`, so a score can
be `1.7` rather than a whole number.

Choice and score answers also expose `probabilities` when supplied by the provider.
An empty distribution means it was omitted, not that every probability is zero.
Provider metadata, token usage, and warnings are available on `Answers`.

## Providers

| Provider | Configuration | API key environment variable | Default model |
| --- | --- | --- | --- |
| TypeSafe API | `TypeSafeConfig` | `TYPESAFE_API_KEY` | `jev-latest` |
| Vercel AI Gateway | `VercelConfig` | `AI_GATEWAY_API_KEY` | `typesafe-ai/jev` |

For Vercel AI Gateway, set its key and replace `TypeSafeConfig` in the quick start
with `VercelConfig`:

```sh
export AI_GATEWAY_API_KEY="your-gateway-key"
```

Both configurations accept the same customization methods:

```rust
use std::time::Duration;
use openasking::VercelConfig;

let config = VercelConfig::new()
    .with_model("typesafe-ai/jev")
    .with_timeout(Duration::from_secs(30));
```

`new()` and `default()` read the process environment when the configuration is
created. `.with_api_key(...)` overrides that value. The SDK does not load `.env`
or shell startup files, and does not fall back to the other provider's key.
Missing or empty credentials produce `Error::Configuration` when the request
is first polled, before network I/O.

Pass a configuration by value or reference to `Asking::new(...)`. Borrowed
configurations are cloned, so requests do not borrow them. Use `Config` when
selecting a provider at runtime.

The default request timeout is 60 seconds. `.with_http_client(client.clone())`
lets you supply a custom connection pool. `.with_endpoint(url)` overrides the
full request URL without changing the provider protocol.

## License

openasking is licensed under the [MIT License](https://github.com/ZENOTME/openasking/blob/main/LICENSE).
