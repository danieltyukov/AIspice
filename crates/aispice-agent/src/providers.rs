//! Building a provider from its id, the configuration and the key store.

use std::sync::Arc;

use crate::config::Config;
use crate::keys::{KeyError, KeyStore};
use crate::provider::{AnthropicProvider, OpenAiProvider, Provider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub needs_key: bool,
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "anthropic",
        display_name: "Anthropic",
        needs_key: true,
    },
    ProviderInfo {
        id: "openai",
        display_name: "OpenAI",
        needs_key: true,
    },
    ProviderInfo {
        id: "google",
        display_name: "Google",
        needs_key: true,
    },
    ProviderInfo {
        id: "openrouter",
        display_name: "OpenRouter",
        needs_key: true,
    },
    ProviderInfo {
        id: "ollama",
        display_name: "Ollama",
        needs_key: false,
    },
    ProviderInfo {
        id: "custom",
        display_name: "Custom endpoint",
        needs_key: false,
    },
];

pub fn info(id: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.id == id)
}

pub fn display_name(id: &str) -> String {
    info(id).map_or_else(|| id.to_string(), |p| p.display_name.to_string())
}

/// The model used when the configuration names none. Only Anthropic has
/// one: model lists elsewhere change too often to hard-code, so the user
/// picks from the live list.
pub fn default_model(id: &str) -> Option<&'static str> {
    match id {
        "anthropic" => Some("claude-opus-5-5"),
        _ => None,
    }
}

/// Output limit per call when the configuration names none. Claude models
/// take 64k comfortably over streaming; many OpenAI-compatible models reject
/// anything above their own cap, and 16k is under nearly all of them.
pub fn default_max_tokens(id: &str) -> u32 {
    match id {
        "anthropic" => 64_000,
        _ => 16_384,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("No {display} API key. Run `aispice keys set {id}` or add it in Settings.")]
    MissingKey { id: String, display: String },
    #[error(
        "Unknown provider `{0}`. Use anthropic, openai, google, openrouter, ollama or custom, \
         or add a base_url under [providers.{0}] in config.toml."
    )]
    UnknownProvider(String),
    #[error(
        "The `{0}` provider needs a base URL. Set base_url under [providers.{0}] in config.toml or in Settings."
    )]
    MissingBaseUrl(String),
    #[error(transparent)]
    Keys(#[from] KeyError),
}

/// Build the provider `id`, applying any base URL override from `config`.
///
/// Blocking: it may read the OS keychain.
pub fn build_provider(
    id: &str,
    config: &Config,
    keys: &KeyStore,
) -> Result<Arc<dyn Provider>, BuildError> {
    let base_url = config.base_url_for(id).map(str::to_string);
    let required_key = || -> Result<String, BuildError> {
        keys.get(id)?.ok_or_else(|| BuildError::MissingKey {
            id: id.to_string(),
            display: display_name(id),
        })
    };
    let with_base = |p: OpenAiProvider| match &base_url {
        Some(url) => p.with_base_url(url),
        None => p,
    };
    let provider: Arc<dyn Provider> = match id {
        "anthropic" => {
            let p = AnthropicProvider::new(required_key()?);
            Arc::new(match &base_url {
                Some(url) => p.with_base_url(url),
                None => p,
            })
        }
        "openai" => Arc::new(with_base(OpenAiProvider::openai(required_key()?))),
        "google" => Arc::new(with_base(OpenAiProvider::google(required_key()?))),
        "openrouter" => Arc::new(with_base(OpenAiProvider::openrouter(required_key()?))),
        "ollama" => Arc::new(with_base(
            OpenAiProvider::ollama().with_api_key(keys.get(id)?),
        )),
        _ => match base_url {
            Some(url) => Arc::new(OpenAiProvider::custom(url, keys.get(id)?).with_id(id)),
            None if id == "custom" => return Err(BuildError::MissingBaseUrl(id.to_string())),
            None => return Err(BuildError::UnknownProvider(id.to_string())),
        },
    };
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderSettings;

    fn config_with(id: &str, url: &str) -> Config {
        let mut config = Config::default();
        config.providers.insert(
            id.into(),
            ProviderSettings {
                base_url: Some(url.into()),
            },
        );
        config
    }

    #[test]
    fn missing_key_message_is_actionable() {
        let err = build_provider("anthropic", &Config::default(), &KeyStore::memory()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "No Anthropic API key. Run `aispice keys set anthropic` or add it in Settings."
        );
        let err =
            build_provider("openrouter", &Config::default(), &KeyStore::memory()).unwrap_err();
        assert!(err.to_string().starts_with("No OpenRouter API key."));
    }

    #[test]
    fn builds_each_provider() {
        let keys = KeyStore::memory();
        for id in ["anthropic", "openai", "google", "openrouter"] {
            keys.set(id, "k").unwrap();
            assert_eq!(
                build_provider(id, &Config::default(), &keys).unwrap().id(),
                id
            );
        }
        // Ollama needs no key.
        let ollama = build_provider("ollama", &Config::default(), &KeyStore::memory()).unwrap();
        assert_eq!(ollama.id(), "ollama");
    }

    #[test]
    fn env_keys_are_used() {
        let keys = KeyStore::memory().with_env(|var| (var == "GEMINI_API_KEY").then(|| "g".into()));
        assert!(build_provider("google", &Config::default(), &keys).is_ok());
    }

    #[test]
    fn custom_endpoints_need_a_base_url() {
        let err = build_provider("custom", &Config::default(), &KeyStore::memory()).unwrap_err();
        assert!(matches!(err, BuildError::MissingBaseUrl(_)));
        let p = build_provider(
            "custom",
            &config_with("custom", "http://localhost:8000/v1"),
            &KeyStore::memory(),
        )
        .unwrap();
        assert_eq!(p.id(), "custom");
    }

    #[test]
    fn any_named_endpoint_with_a_base_url_works() {
        let p = build_provider(
            "lmstudio",
            &config_with("lmstudio", "http://localhost:1234/v1"),
            &KeyStore::memory(),
        )
        .unwrap();
        assert_eq!(p.id(), "lmstudio");
        let err = build_provider("mystery", &Config::default(), &KeyStore::memory()).unwrap_err();
        assert!(matches!(err, BuildError::UnknownProvider(_)));
    }

    #[tokio::test]
    async fn base_url_overrides_reach_the_provider() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"id": "llama3.2"}]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let config = config_with("ollama", &format!("{}/v1", server.uri()));
        let p = build_provider("ollama", &config, &KeyStore::memory()).unwrap();
        let models = p.list_models().await.unwrap();
        assert_eq!(models[0].id, "llama3.2");
    }

    #[test]
    fn defaults_per_provider() {
        assert_eq!(default_model("anthropic"), Some("claude-opus-5-5"));
        assert_eq!(default_model("openai"), None);
        assert_eq!(default_max_tokens("anthropic"), 64_000);
        assert_eq!(default_max_tokens("ollama"), 16_384);
        assert_eq!(display_name("google"), "Google");
        assert_eq!(display_name("lmstudio"), "lmstudio");
    }
}
