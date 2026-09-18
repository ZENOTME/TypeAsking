//! Shared HTTP execution with provider-specific request/response adapters.
use std::sync::OnceLock;

use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};

use crate::{
    Error, Request, ResponseFuture,
    answer::WireResponse,
    config::Backend,
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

pub(crate) fn prepare(mut request: Request) -> Result<ResponseFuture, Error> {
    let (backend, mut options) = request.provider.into_parts();
    let state = request
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
    if request.questions.is_empty() {
        return Err(config("at least one question is required"));
    }
    let mut questions = serde_json::Map::new();
    for question in &request.questions {
        let mut value = question.encode()?;
        if matches!(backend, Backend::TypeSafe) && value["type"] == "boolean" {
            value["type"] = json!("noul");
        }
        if questions.insert(question.id().into(), value).is_some() {
            return Err(config("duplicate question ID"));
        }
    }
    let key = options.api_key.take().ok_or_else(|| {
        config(&format!(
            "{} or with_api_key is required",
            backend.key_env()
        ))
    })?;
    nonempty(&key, "API key")?;
    let mut auth = HeaderValue::from_str(&format!("Bearer {key}"))
        .map_err(|_| config("API key is not a valid HTTP header value"))?;
    auth.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth);
    nonempty(&options.model, "model")?;
    if matches!(backend, Backend::Vercel) {
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
        headers.insert(
            "ai-model-id",
            HeaderValue::from_str(&options.model)
                .map_err(|_| config("model is not a valid HTTP header value"))?,
        );
    }
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
    let mut body = json!({ "state": state, "questions": questions });
    if matches!(backend, Backend::TypeSafe) {
        body["model"] = json!(options.model);
    }
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
                concat!("openasking/", env!("CARGO_PKG_VERSION")),
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
        let wire: WireResponse = match backend {
            Backend::Vercel => serde_json::from_slice(&bytes)
                .map_err(|_| Error::InvalidResponse("malformed Gateway evaluation JSON".into()))?,
            Backend::TypeSafe => crate::typesafe::decode(&bytes)?,
        };
        wire.decode(&request.questions)
    }))
}
