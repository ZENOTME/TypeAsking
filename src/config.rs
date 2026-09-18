//! Provider-specific settings, independent of any evaluation request.
use std::time::Duration;

/// Configuration for Vercel AI Gateway. Reads `AI_GATEWAY_API_KEY` on construction.
///
/// Defaults to model `typesafe-ai/jev` and the Gateway v4 evaluation endpoint.
/// Clone or borrow this configuration to create independent concurrent requests.
#[derive(Clone)]
pub struct VercelConfig(pub(crate) Settings);

/// Configuration for the direct TypeSafe API. Reads `TYPESAFE_API_KEY` on construction.
///
/// Defaults to model `jev-latest` and `https://api.typesafe.ai/v1/systemone`.
/// No shell startup files or `.env` files are loaded by the SDK.
/// Boolean questions are encoded as native Noul questions; answers still expose
/// [`crate::BoolAnswer::probability_true`]. Native probabilities and scores are
/// validated with a half-hundredth per-value rounding tolerance, matching observed
/// native responses (which do not supply Gateway rounding metadata).
#[derive(Clone)]
pub struct TypeSafeConfig(pub(crate) Settings);

/// A provider configuration accepted by [`crate::Asking::new`].
///
/// Concrete configs convert automatically, by value or by reference. This enum
/// also allows choosing a backend at runtime without changing the request type.
#[derive(Clone)]
pub enum Config {
    /// Evaluate through Vercel AI Gateway.
    Vercel(VercelConfig),
    /// Evaluate directly through TypeSafe.
    TypeSafe(TypeSafeConfig),
}

#[derive(Clone, Copy)]
pub(crate) enum Backend {
    Vercel,
    TypeSafe,
}

#[derive(Clone)]
pub(crate) struct Settings {
    pub(crate) api_key: Option<String>,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) timeout: Duration,
    pub(crate) client: Option<reqwest::Client>,
}

macro_rules! configuration {
    ($name:ident, $variant:ident, $env:literal, $endpoint:literal, $model:literal) => {
        impl $name {
            #[doc = concat!("Read `", $env, "` from the process environment when constructing this config.")]
            ///
            #[doc = concat!("Defaults to model `", $model, "` and endpoint `", $endpoint, "`.")]
            /// `Default::default()` behaves identically. No `.env` or shell startup
            /// files are loaded, and no other provider's key is used as a fallback.
            /// Use [`Self::with_api_key`] to override the environment value.
            ///
            /// Does not perform I/O. Missing credentials and invalid settings are
            /// reported when the containing request is first polled.
            pub fn new() -> Self {
                Self(Settings {
                    api_key: std::env::var($env).ok(),
                    endpoint: $endpoint.into(),
                    model: $model.into(),
                    timeout: Duration::from_secs(60),
                    client: None,
                })
            }
            #[doc = concat!("Override the API key read from `", $env, "`.")]
            ///
            /// This value takes precedence even when the environment variable is set.
            /// Empty or invalid header values fail on poll.
            pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
                self.0.api_key = Some(key.into());
                self
            }
            /// Set the full HTTP(S) evaluation URL. No path is appended.
            ///
            /// The URL may not contain a username/password. Credentials are sent
            /// to this address. Changing it does not change the selected protocol.
            pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
                self.0.endpoint = endpoint.into();
                self
            }
            /// Select a model using this provider's naming scheme. Must not be blank.
            pub fn with_model(mut self, model: impl Into<String>) -> Self {
                self.0.model = model.into();
                self
            }
            /// Set the total HTTP timeout, including response reading (default 60s).
            ///
            /// Zero fails validation on poll. Timeout failures return
            /// [`crate::Error::Transport`], whose source supports `is_timeout()`.
            pub fn with_timeout(mut self, timeout: Duration) -> Self {
                self.0.timeout = timeout;
                self
            }
            /// Use a custom HTTP pool instead of the default shared pool.
            ///
            /// Client clones share connections. Redirect/proxy settings follow
            /// the supplied client; the default client does not follow redirects.
            pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
                self.0.client = Some(client);
                self
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl From<$name> for Config {
            fn from(value: $name) -> Self {
                Self::$variant(value)
            }
        }
        impl From<&$name> for Config {
            fn from(value: &$name) -> Self {
                value.clone().into()
            }
        }
    };
}
configuration!(
    VercelConfig,
    Vercel,
    "AI_GATEWAY_API_KEY",
    "https://ai-gateway.vercel.sh/v4/ai/evaluation-model",
    "typesafe-ai/jev"
);
configuration!(
    TypeSafeConfig,
    TypeSafe,
    "TYPESAFE_API_KEY",
    "https://api.typesafe.ai/v1/systemone",
    "jev-latest"
);
impl From<&Config> for Config {
    fn from(value: &Config) -> Self {
        value.clone()
    }
}

impl Config {
    pub(crate) fn into_parts(self) -> (Backend, Settings) {
        match self {
            Self::Vercel(c) => (Backend::Vercel, c.0),
            Self::TypeSafe(c) => (Backend::TypeSafe, c.0),
        }
    }
}
impl Backend {
    pub(crate) fn key_env(self) -> &'static str {
        match self {
            Self::Vercel => "AI_GATEWAY_API_KEY",
            Self::TypeSafe => "TYPESAFE_API_KEY",
        }
    }
}
