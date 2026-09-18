use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
};
use openasking::{Asking, BoolQuestion, ChoiceQuestion, Error, ScoreQuestion, VercelConfig};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

fn request(server: &MockServer) -> Asking {
    Asking::new(
        VercelConfig::new()
            .with_api_key("test-key")
            // Each test owns a Tokio runtime; do not reuse connections from a dropped runtime.
            .with_http_client(reqwest::Client::new())
            .with_endpoint(format!("{}/evaluation-model", server.uri())),
    )
    .state("test state")
}

fn boolean() -> BoolQuestion {
    BoolQuestion::new("safe", "Is it safe?")
}

fn first_poll(request: &mut Asking) -> Poll<Result<openasking::Answers, Error>> {
    Pin::new(request).poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn invalid_requests_fail_on_first_poll_without_runtime() {
    let cases = vec![
        Asking::new(VercelConfig::new()),
        Asking::new(VercelConfig::new()).state("state"),
        Asking::new(VercelConfig::new())
            .state("")
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state("  ")
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state(json!({}))
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state(json!([]))
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state(Value::Null)
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state(42)
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state(false)
            .bool_question(boolean()),
        Asking::new(VercelConfig::new().with_api_key(""))
            .state("state")
            .bool_question(boolean()),
        Asking::new(VercelConfig::new().with_api_key("bad\nkey"))
            .state("state")
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state("state")
            .bool_question(boolean())
            .bool_question(boolean()),
        Asking::new(VercelConfig::new())
            .state("state")
            .bool_question(BoolQuestion::new("", "Q")),
        Asking::new(VercelConfig::new())
            .state("state")
            .bool_question(BoolQuestion::new("q", "")),
        Asking::new(VercelConfig::new())
            .state("state")
            .bool_question(boolean().when_true("")),
        Asking::new(VercelConfig::new())
            .state("state")
            .choice_question(ChoiceQuestion::new("q", "Q")),
        Asking::new(VercelConfig::new())
            .state("state")
            .choice_question(
                ChoiceQuestion::new("q", "Q")
                    .option("a", "A")
                    .option("a", "B"),
            ),
        Asking::new(VercelConfig::new())
            .state("state")
            .score_question(ScoreQuestion::new("q", "Q").level("low")),
        Asking::new(VercelConfig::new())
            .state("state")
            .score_question(ScoreQuestion::new("q", "Q").level("low").level("")),
        Asking::new(
            VercelConfig::new()
                .with_api_key("k")
                .with_endpoint("ftp://example.com"),
        )
        .state("state")
        .bool_question(boolean()),
        Asking::new(
            VercelConfig::new()
                .with_api_key("k")
                .with_timeout(Duration::ZERO),
        )
        .state("state")
        .bool_question(boolean()),
    ];
    for mut request in cases {
        assert!(matches!(
            first_poll(&mut request),
            Poll::Ready(Err(Error::Configuration(_)))
        ));
        assert!(matches!(
            first_poll(&mut request),
            Poll::Ready(Err(Error::AlreadyCompleted))
        ));
    }
}

#[test]
fn serialization_errors_are_deferred() {
    struct BadState;
    impl serde::Serialize for BadState {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("cannot serialize"))
        }
    }
    let mut request = Asking::new(VercelConfig::new())
        .state(BadState)
        .bool_question(boolean());
    assert!(matches!(
        first_poll(&mut request),
        Poll::Ready(Err(Error::StateSerialization(_)))
    ));
}

#[tokio::test]
async fn mixed_questions_wire_contract_and_typed_access() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/evaluation-model"))
        .and(header("authorization", "Bearer test-key"))
        .and(header("ai-model-id", "typesafe-ai/jev"))
        .and(header("ai-gateway-protocol-version", "0.0.1"))
        .and(header("ai-gateway-auth-method", "api-key"))
        .and(header("ai-evaluation-model-specification-version", "4"))
        .and(body_json(json!({
            "state": {"action": "read"},
            "questions": {
                "safe": {"type": "boolean", "instructions": "Is it safe?", "criteria": {"true": "read only", "false": "writes"}},
                "route": {"type": "choice", "instructions": "Next?", "criteria": {"continue": "Continue", "retry": "Retry"}},
                "quality": {"type": "score", "instructions": "Quality?", "criteria": ["poor", "fair", "good"]}
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": {
                "safe": {"type": "boolean", "probability": 0.99},
                "route": {"type": "choice", "choice": "continue", "probabilities": {"continue": 0.8, "retry": 0.2}},
                "quality": {"type": "score", "score": 1.7, "probabilities": {"2": 0.7, "0": 0.0, "1": 0.3}}
            },
            "providerMetadata": {"typesafe": {"confidence": {"route": 0.6}}},
            "usage": {"inputTokens": 100, "outputTokens": 10},
            "warnings": [{"type": "other", "message": "test"}]
        }))).expect(1).mount(&server).await;
    let mut state = json!({"action": "read"});
    let future = request(&server)
        .state(&state)
        .bool_question(boolean().when_true("read only").when_false("writes"))
        .choice_question(
            ChoiceQuestion::new("route", "Next?")
                .option("continue", "Continue")
                .option("retry", "Retry"),
        )
        .score_question(
            ScoreQuestion::new("quality", "Quality?")
                .level("poor")
                .level("fair")
                .level("good"),
        );
    state["action"] = json!("write"); // request holds its own snapshot
    assert!(server.received_requests().await.unwrap().is_empty());
    let answers = future.await.unwrap();
    assert_eq!(answers.bool_answer("safe").unwrap().probability_true, 0.99);
    assert_eq!(answers.choice_answer("route").unwrap().choice, "continue");
    assert_eq!(
        answers.score_answer("quality").unwrap().probabilities,
        [0.0, 0.3, 0.7]
    );
    assert_eq!(answers.metadata["typesafe"]["confidence"]["route"], 0.6);
    assert_eq!(answers.usage.as_ref().unwrap().input_tokens, Some(100));
    assert_eq!(answers.warnings.len(), 1);
    assert!(matches!(
        answers.bool_answer("missing"),
        Err(Error::NotFound(id)) if id == "missing"
    ));
    assert!(matches!(
        answers.bool_answer("route"),
        Err(Error::TypeMismatch { .. })
    ));
    assert!(matches!(
        answers.choice_answer("safe"),
        Err(Error::TypeMismatch { .. })
    ));
    assert!(matches!(
        answers.score_answer("safe"),
        Err(Error::TypeMismatch { .. })
    ));
}

#[tokio::test]
async fn pending_polls_send_only_one_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(80))
                .set_body_json(
                    json!({"answers": {"safe": {"type": "boolean", "probability": 0.7}}}),
                ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut future = request(&server).bool_question(boolean());
    for _ in 0..20 {
        assert!(first_poll(&mut future).is_pending());
        tokio::task::yield_now().await;
    }
    assert_eq!(
        (&mut future)
            .await
            .unwrap()
            .bool_answer("safe")
            .unwrap()
            .probability_true,
        0.7
    );
    assert!(matches!(
        first_poll(&mut future),
        Poll::Ready(Err(Error::AlreadyCompleted))
    ));
}

#[tokio::test]
async fn independent_requests_join_and_spawn() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"answers": {"safe": {"type": "boolean", "probability": 0.5}}}),
            ),
        )
        .expect(3)
        .mount(&server)
        .await;
    let a = request(&server).bool_question(boolean());
    let b = request(&server).bool_question(boolean());
    let (a, b) = tokio::try_join!(a, b).unwrap();
    assert_eq!(
        a.bool_answer("safe").unwrap(),
        b.bool_answer("safe").unwrap()
    );
    tokio::spawn(request(&server).bool_question(boolean()))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn requests_can_be_in_flight_together_and_dropped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(30))
                .set_body_json(
                    json!({"answers": {"safe": {"type": "boolean", "probability": 0.5}}}),
                ),
        )
        .expect(2)
        .mount(&server)
        .await;
    let a = tokio::spawn(request(&server).bool_question(boolean()));
    let b = tokio::spawn(request(&server).bool_question(boolean()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while server.received_requests().await.unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!a.is_finished() && !b.is_finished());
    a.abort();
    b.abort();
    assert!(a.await.unwrap_err().is_cancelled());
    assert!(b.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn missing_distributions_remain_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "answers": {"c": {"type": "choice", "choice": "a"}, "s": {"type": "score", "score": 0.5}}
    }))).mount(&server).await;
    let answers = request(&server)
        .choice_question(ChoiceQuestion::new("c", "Q").option("a", "A"))
        .score_question(ScoreQuestion::new("s", "Q").level("low").level("high"))
        .await
        .unwrap();
    assert!(answers.choice_answer("c").unwrap().probabilities.is_empty());
    assert!(answers.score_answer("s").unwrap().probabilities.is_empty());
    assert!(answers.metadata.is_null());
}

#[tokio::test]
async fn numeric_score_indexes_and_declared_rounding() {
    let server = MockServer::start().await;
    let probabilities: serde_json::Map<_, _> = (0..12)
        .map(|i| (i.to_string(), json!(if i == 10 { 1.0 } else { 0.0 })))
        .collect();
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "answers": {
            "score": {"type": "score", "score": 10.0, "probabilities": probabilities},
            "rounded": {"type": "score", "score": 1.0, "probabilities": {"0": 0.33, "1": 0.33, "2": 0.33}}
        }, "rounding": {"probabilityDecimals": 2, "scoreDecimals": 2}
    }))).mount(&server).await;
    let mut score = ScoreQuestion::new("score", "Q");
    for i in 0..12 {
        score = score.level(format!("level {i}"));
    }
    let answers = request(&server)
        .score_question(score)
        .score_question(
            ScoreQuestion::new("rounded", "Q")
                .level("a")
                .level("b")
                .level("c"),
        )
        .await
        .unwrap();
    assert_eq!(
        answers.score_answer("score").unwrap().probabilities[10],
        1.0
    );
}

#[tokio::test]
async fn malformed_answers_are_rejected() {
    let bad_answers = [
        json!({}),
        json!({"other": {"type": "boolean", "probability": 0.5}}),
        json!({"safe": {"type": "boolean", "probability": 1.1}}),
        json!({"safe": {"type": "boolean", "probability": -0.1}}),
        json!({"safe": {"type": "choice", "choice": "a"}}),
        json!({"safe": {"type": "boolean", "probability": 0.5}, "extra": {"type": "boolean", "probability": 0.5}}),
        json!({"safe": {"type": "boolean"}}),
    ];
    for answers in bad_answers {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers": answers})))
            .mount(&server)
            .await;
        assert!(matches!(
            request(&server).bool_question(boolean()).await,
            Err(Error::InvalidResponse(_))
        ));
    }
}

#[tokio::test]
async fn invalid_choices_and_scores_are_rejected() {
    let cases = [
        json!({"type": "choice", "choice": "unknown"}),
        json!({"type": "choice", "choice": "a", "probabilities": {"a": 1.0}}),
        json!({"type": "choice", "choice": "a", "probabilities": {"a": 0.2, "b": 0.8}}),
        json!({"type": "choice", "choice": "a", "probabilities": {"a": 0.8, "b": 0.8}}),
        json!({"type": "score", "score": 2.0}),
        json!({"type": "score", "score": 0.5, "probabilities": {"0": 0.5, "2": 0.5}}),
        json!({"type": "score", "score": 0.5, "probabilities": {"0": 1.0, "1": 0.0}}),
    ];
    for answer in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"answers": {"q": answer}})),
            )
            .mount(&server)
            .await;
        let r = request(&server);
        let r = if answer["type"] == "choice" {
            r.choice_question(
                ChoiceQuestion::new("q", "Q")
                    .option("a", "A")
                    .option("b", "B"),
            )
        } else {
            r.score_question(ScoreQuestion::new("q", "Q").level("low").level("high"))
        };
        let result = r.await;
        assert!(
            matches!(result, Err(Error::InvalidResponse(_))),
            "answer {answer}: {result:?}"
        );
    }
}

#[tokio::test]
async fn http_errors_and_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_string("sensitive response"))
        .expect(1)
        .mount(&server)
        .await;
    let error = request(&server).bool_question(boolean()).await.unwrap_err();
    assert!(matches!(error, Error::Http { status: 429 }));
    assert!(!format!("{error:?}").contains("sensitive"));
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;
    let error = Asking::new(
        VercelConfig::new()
            .with_api_key("test-key")
            .with_endpoint(server.uri())
            .with_timeout(Duration::from_millis(50)),
    )
    .state("state")
    .bool_question(boolean())
    .await
    .unwrap_err();
    assert!(matches!(error, Error::Transport(e) if e.is_timeout()));
}

// Subprocesses test environment semantics without mutating global environment in
// a multithreaded test process (set_var is unsafe on edition 2024).
#[test]
fn environment_credentials() {
    if let Ok(mode) = std::env::var("JEV_TEST_ENV_CHILD") {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let server = MockServer::start().await;
            let expected = if mode == "env" {
                "Bearer env-test-key"
            } else {
                "Bearer explicit-key"
            };
            if mode != "missing" {
                Mock::given(header("authorization", expected))
                    .respond_with(ResponseTemplate::new(200).set_body_json(
                        json!({"answers": {"safe": {"type": "boolean", "probability": 1.0}}}),
                    ))
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            let mut config = VercelConfig::new().with_endpoint(server.uri());
            if mode == "override" || mode == "explicit" {
                config = config.with_api_key("explicit-key");
            }
            let r = Asking::new(config).state("state").bool_question(boolean());
            if mode == "missing" {
                assert!(matches!(r.await, Err(Error::Configuration(_))));
            } else {
                r.await.unwrap();
            }
        });
        return;
    }
    for mode in ["env", "override", "explicit", "missing"] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "environment_credentials"])
            .env("JEV_TEST_ENV_CHILD", mode)
            .env_remove("AI_GATEWAY_API_KEY");
        if mode == "env" || mode == "override" {
            command.env("AI_GATEWAY_API_KEY", "env-test-key");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "child {mode} failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
