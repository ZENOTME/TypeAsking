//! Isolated adapter for the experimental AI Gateway evaluation v4 protocol.
use std::sync::OnceLock;

use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};

use crate::{
    Config, Error, ResponseFuture,
    answer::WireResponse,
    question::{config, nonempty},
};

// Cloning reqwest::Client shares its pool. Initialization happens only on execution.
static CLIENT: OnceLock<Client> = OnceLock::new();

fn shared_client() -> Result<Client, Error> {
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let _ = CLIENT.set(client.clone());
    Ok(CLIENT.get().cloned().unwrap_or(client))
}

pub(crate) fn prepare(mut options: Config) -> Result<ResponseFuture, Error> {
    let state = options
        .state
        .take()
        .ok_or_else(|| config("state is required"))?
        .map_err(Error::StateSerialization)?;
    let valid = match &state {
        Value::String(s) => !s.trim().is_empty(),
        Value::Object(m) => !m.is_empty(),
        Value::Array(a) => !a.is_empty(),
        _ => false,
    };
    if !valid {
        return Err(config("state must be a nonempty string, object, or array"));
    }
    if options.questions.is_empty() {
        return Err(config("at least one question is required"));
    }
    let mut questions = serde_json::Map::new();
    for question in &options.questions {
        let value = question.encode()?;
        if questions.insert(question.id().into(), value).is_some() {
            return Err(config("duplicate question ID"));
        }
    }
    let key = options
        .api_key
        .take()
        .ok_or_else(|| config("AI_GATEWAY_API_KEY or with_api_key is required"))?;
    nonempty(&key, "API key")?;
    let mut auth = HeaderValue::from_str(&format!("Bearer {key}"))
        .map_err(|_| config("API key is not a valid HTTP header value"))?;
    auth.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth);
    headers.insert(
        "ai-gateway-auth-method",
        HeaderValue::from_static("api-key"),
    );
    headers.insert(
        "ai-gateway-protocol-version",
        HeaderValue::from_static("0.0.1"),
    );
    headers.insert(
        "ai-evaluation-model-specification-version",
        HeaderValue::from_static("4"),
    );
    headers.insert("ai-model-id", HeaderValue::from_static("typesafe-ai/jev"));
    let url = Url::parse(&options.endpoint).map_err(|_| config("invalid evaluation endpoint"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(config(
            "endpoint must be an HTTP(S) URL without user credentials",
        ));
    }
    if options.timeout.is_zero() {
        return Err(config("timeout must be greater than zero"));
    }
    let body = json!({ "state": state, "questions": questions });
    Ok(Box::pin(async move {
        let client = match options.client {
            Some(client) => client,
            None => shared_client()?,
        };
        let response = client
            .post(url)
            .headers(headers)
            .header(
                "user-agent",
                concat!("typeasking/", env!("CARGO_PKG_VERSION")),
            )
            .timeout(options.timeout)
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
            });
        }
        let bytes = response.bytes().await?;
        // Avoid echoing response data into error messages (it can include user input).
        let wire: WireResponse = serde_json::from_slice(&bytes)
            .map_err(|_| Error::InvalidResponse("malformed evaluation JSON".into()))?;
        wire.decode(&options.questions)
    }))
}
