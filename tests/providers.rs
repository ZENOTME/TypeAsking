use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
};
use openasking::{
    Asking, BoolQuestion, ChoiceQuestion, Config, Error, ScoreQuestion, TypeSafeConfig,
    VercelConfig,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

fn native_reply() -> Value {
    json!({"model":"jev-1.13.0", "usage":{"input_tokens":380,"output_tokens":62}, "answers":{
        "safe":{"type":"noul","noul":0.99},
        "route":{"type":"choice","choice":"continue","probabilities":{"continue":0.8,"retry":0.2},"confidence":0.6},
        "quality":{"type":"score","score":1.19,"probabilities":{"0":0.04,"1":0.73,"2":0.23},"confidence":0.6,"legend":{"0":"poor","1":"fair","2":"good"}}
    }})
}
fn mixed(config: impl Into<Config>) -> Asking {
    Asking::new(config)
        .state(json!({"action":"read"}))
        .bool_question(
            BoolQuestion::new("safe", "Is it safe?")
                .when_true("read only")
                .when_false("writes"),
        )
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
        )
}

#[tokio::test]
async fn native_wire_contract_and_common_answers() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/systemone"))
        .and(header("authorization","Bearer native-key"))
        .and(body_json(json!({"model":"jev-pinned", "state":{"action":"read"},"questions":{
            "safe":{"type":"noul","instructions":"Is it safe?","criteria":{"true":"read only","false":"writes"}},
            "route":{"type":"choice","instructions":"Next?","criteria":{"continue":"Continue","retry":"Retry"}},
            "quality":{"type":"score","instructions":"Quality?","criteria":["poor","fair","good"]}
        }})))
        .respond_with(ResponseTemplate::new(200).set_body_json(native_reply())).expect(1).mount(&server).await;
    let config = TypeSafeConfig::new()
        .with_api_key("native-key")
        .with_endpoint(format!("{}/v1/systemone", server.uri()))
        .with_model("jev-pinned");
    let answers = mixed(&config).await.unwrap();
    assert_eq!(answers.bool_answer("safe").unwrap().probability_true, 0.99);
    assert_eq!(answers.choice_answer("route").unwrap().choice, "continue");
    assert_eq!(
        answers.score_answer("quality").unwrap().probabilities,
        [0.04, 0.73, 0.23]
    );
    assert_eq!(answers.metadata["typesafe"]["confidence"]["route"], 0.6);
    assert_eq!(
        answers.metadata["typesafe"]["legend"]["quality"]["2"],
        "good"
    );
    assert_eq!(answers.metadata["typesafe"]["model"], "jev-1.13.0");
    assert_eq!(answers.usage.as_ref().unwrap().input_tokens, Some(380));
    let requests = server.received_requests().await.unwrap();
    for name in [
        "ai-model-id",
        "ai-gateway-auth-method",
        "ai-gateway-protocol-version",
        "ai-evaluation-model-specification-version",
    ] {
        assert!(!requests[0].headers.contains_key(name));
    }
}

#[tokio::test]
async fn reused_configs_and_backends_are_isolated() {
    let native = MockServer::start().await;
    let gateway = MockServer::start().await;
    Mock::given(header("authorization", "Bearer native-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(native_reply()))
        .expect(2)
        .mount(&native)
        .await;
    let mut gateway_reply = native_reply();
    gateway_reply["answers"]["safe"] = json!({"type":"boolean","probability":0.99});
    gateway_reply["usage"] = json!({"inputTokens":380,"outputTokens":62});
    Mock::given(header("authorization", "Bearer gateway-key"))
        .and(header("ai-model-id", "typesafe-ai/custom"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gateway_reply))
        .expect(1)
        .mount(&gateway)
        .await;
    let config = TypeSafeConfig::new()
        .with_api_key("native-key")
        .with_endpoint(native.uri());
    let a = mixed(&config);
    let b = mixed(config.clone());
    let c = mixed(
        VercelConfig::new()
            .with_api_key("gateway-key")
            .with_endpoint(gateway.uri())
            .with_model("typesafe-ai/custom"),
    );
    drop(config); // futures own their settings
    let (a, b, c) = tokio::try_join!(a, b, c).unwrap();
    assert_eq!(
        a.bool_answer("safe").unwrap(),
        c.bool_answer("safe").unwrap()
    );
    assert_eq!(
        a.score_answer("quality").unwrap(),
        b.score_answer("quality").unwrap()
    );
}

#[tokio::test]
async fn native_rounding_and_invalid_responses() {
    let server = MockServer::start().await;
    let config = TypeSafeConfig::new()
        .with_api_key("key")
        .with_endpoint(server.uri());
    let mut rounded = native_reply();
    rounded["answers"]["quality"]["score"] = json!(1.0);
    rounded["answers"]["quality"]["probabilities"] = json!({"0":0.33,"1":0.33,"2":0.33});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rounded))
        .mount(&server)
        .await;
    assert_eq!(
        mixed(&config)
            .await
            .unwrap()
            .score_answer("quality")
            .unwrap()
            .score,
        1.0
    );
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/answers/safe/noul", json!(1.1)),
        ("/answers/route/confidence", json!(-0.1)),
        ("/answers/route/choice", json!("unknown")),
        (
            "/answers/route/probabilities",
            json!({"continue":0.8,"retry":0.8}),
        ),
        ("/answers/quality/legend", json!({"0":"poor"})),
        ("/answers/quality/score", json!(3.0)),
        (
            "/answers/quality/probabilities",
            json!({"0":0.0,"1":1.0,"2":0.0}),
        ),
    ] {
        let mut reply = native_reply();
        *reply.pointer_mut(pointer).unwrap() = value;
        cases.push(reply);
    }
    let mut missing = native_reply();
    missing["answers"]["route"]
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    cases.push(missing);
    let mut missing = native_reply();
    missing.as_object_mut().unwrap().remove("usage");
    cases.push(missing);
    for reply in cases {
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply))
            .mount(&server)
            .await;
        assert!(matches!(
            mixed(&config).await,
            Err(Error::InvalidResponse(_))
        ));
    }
}

#[test]
fn provider_validation_is_deferred_to_poll() {
    let configs: Vec<Config> = vec![
        TypeSafeConfig::new().with_api_key("").into(),
        TypeSafeConfig::new()
            .with_api_key("key")
            .with_model("")
            .into(),
        TypeSafeConfig::new()
            .with_api_key("key")
            .with_timeout(Duration::ZERO)
            .into(),
        VercelConfig::new()
            .with_api_key("key")
            .with_model("bad\nmodel")
            .into(),
    ];
    for config in configs {
        let mut request = mixed(config);
        assert!(matches!(
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(Error::Configuration(_)))
        ));
    }
}

#[test]
fn typesafe_environment_credentials() {
    if let Ok(mode) = std::env::var("OPENASKING_NATIVE_ENV_TEST") {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server=MockServer::start().await;
            let key=if mode=="env" {"Bearer native-env"} else {"Bearer explicit"};
            if mode!="missing" {
                Mock::given(header("authorization",key)).respond_with(ResponseTemplate::new(200).set_body_json(native_reply()))
                    .expect(1).mount(&server).await;
            }
            let mut config=TypeSafeConfig::new().with_endpoint(server.uri());
            if mode=="override" || mode=="explicit" {config=config.with_api_key("explicit");}
            if mode=="missing" {
                assert!(matches!(mixed(config).await,Err(Error::Configuration(message)) if message.contains("TYPESAFE_API_KEY")));
                assert!(server.received_requests().await.unwrap().is_empty());
            } else { mixed(config).await.unwrap(); }
        });
        return;
    }
    for mode in ["env", "override", "explicit", "missing"] {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "typesafe_environment_credentials"])
            .env("OPENASKING_NATIVE_ENV_TEST", mode)
            .env_remove("TYPESAFE_API_KEY")
            .env("AI_GATEWAY_API_KEY", "wrong-provider-key");
        if mode == "env" || mode == "override" {
            cmd.env("TYPESAFE_API_KEY", "native-env");
        }
        let result = cmd.output().unwrap();
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

// Run with and without `--features serde_json/arbitrary_precision`.
// Raw bodies preserve decimal/exponent spelling all the way into the HTTP decoder.
#[tokio::test]
async fn numeric_answers_survive_dependency_feature_unification() {
    let cases = [
        ("0.12", "0.02", "0.86", "0.75", "1.74", 0.86, 0.75, 1.74),
        (
            "12e-2", "2e-2", "86e-2", "75e-2", "174e-2", 0.86, 0.75, 1.74,
        ),
        ("0", "0", "1", "1", "2", 1.0, 1.0, 2.0),
    ];
    for native in [false, true] {
        let client = reqwest::Client::new();
        for (low, medium, high, probability, score, expected_high, expected_bool, expected_score) in
            cases
        {
            let server = MockServer::start().await;
            let (bool_type, bool_field, usage) = if native {
                ("noul", "noul", r#""input_tokens":10,"output_tokens":20"#)
            } else {
                (
                    "boolean",
                    "probability",
                    r#""inputTokens":10,"outputTokens":20"#,
                )
            };
            let body = format!(
                r#"{{"model":"jev-test","usage":{{{usage}}},"answers":{{
                "safe":{{"type":"{bool_type}","{bool_field}":{probability}}},
                "route":{{"type":"choice","choice":"high","probabilities":{{"low":{low},"medium":{medium},"high":{high}}},"confidence":0.8}},
                "quality":{{"type":"score","score":{score},"probabilities":{{"0":{low},"1":{medium},"2":{high}}},"confidence":0.8,"legend":{{"0":"poor","1":"fair","2":"good"}}}}
            }}}}"#
            );
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
                .expect(1)
                .mount(&server)
                .await;
            let config: Config = if native {
                TypeSafeConfig::new()
                    .with_api_key("test")
                    .with_endpoint(server.uri())
                    .with_http_client(client.clone())
                    .into()
            } else {
                VercelConfig::new()
                    .with_api_key("test")
                    .with_endpoint(server.uri())
                    .with_http_client(client.clone())
                    .into()
            };
            let answers = Asking::new(config)
                .state("Choose effort")
                .bool_question(BoolQuestion::new("safe", "Safe?"))
                .choice_question(
                    ChoiceQuestion::new("route", "Effort?")
                        .option("low", "Low")
                        .option("medium", "Medium")
                        .option("high", "High"),
                )
                .score_question(
                    ScoreQuestion::new("quality", "Quality?")
                        .level("poor")
                        .level("fair")
                        .level("good"),
                )
                .await
                .unwrap_or_else(|e| panic!("native={native}, high={high}: {e:?}"));
            assert_eq!(
                answers.bool_answer("safe").unwrap().probability_true,
                expected_bool
            );
            let route = answers.choice_answer("route").unwrap();
            assert_eq!(route.choice, "high");
            assert_eq!(route.probabilities["high"], expected_high);
            assert_eq!(
                answers.score_answer("quality").unwrap().score,
                expected_score
            );
            assert_eq!(answers.usage.unwrap().input_tokens, Some(10));
        }
    }
}

#[tokio::test]
async fn invalid_wire_numbers_are_rejected() {
    for native in [false, true] {
        for bad in ["1e400", "\"0.86\""] {
            let server = MockServer::start().await;
            let body = if native {
                format!(
                    r#"{{"model":"jev-test","usage":{{"input_tokens":1,"output_tokens":1}},"answers":{{"safe":{{"type":"noul","noul":{bad}}}}}}}"#
                )
            } else {
                format!(r#"{{"answers":{{"safe":{{"type":"boolean","probability":{bad}}}}}}}"#)
            };
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
                .mount(&server)
                .await;
            let config: Config = if native {
                TypeSafeConfig::new()
                    .with_api_key("test")
                    .with_endpoint(server.uri())
                    .with_http_client(reqwest::Client::new())
                    .into()
            } else {
                VercelConfig::new()
                    .with_api_key("test")
                    .with_endpoint(server.uri())
                    .with_http_client(reqwest::Client::new())
                    .into()
            };
            let result = Asking::new(config)
                .state("test")
                .bool_question(BoolQuestion::new("safe", "Safe?"))
                .await;
            assert!(
                matches!(result, Err(Error::InvalidResponse(_))),
                "{result:?}"
            );
        }
    }
}
