use std::collections::BTreeMap;

use crate::error::AppError;

use super::{
    AnthropicProviderOptions, ConfigEnvironment, ConfigError, HttpProviderAdapterConfig,
    OpenAiApiModeConfig, OpenAiBackendConfig, OpenAiProviderOptions, ProviderAdapterDefinition,
    ProviderApiAuthConfig, ProviderAuthConfig, ProviderCredentialAuthConfig,
    ProviderGitlabAuthConfig, ProviderProtocolPathsConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, SimpleHttpProviderOptions, StreamTransportMode,
    list_provider_adapter_models,
};
use crate::provider::auth::{AuthData, CredentialIssuer};

pub const HTTP_ADAPTER_MODEL_LIST_ADAPTER_IDS: [&str; 3] = ["openai", "anthropic", "gemini"];

#[derive(Debug, Clone)]
pub struct ProviderAdapterModelsTarget {
    pub provider_id: String,
    pub auth: ProviderAuthConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
}

pub fn draft_provider_adapter_models_target(
    provider_id: Option<&str>,
    base_url: &str,
    protocol_paths: ProviderProtocolPathsConfig,
    api_key: Option<&str>,
    api_key_env: Option<&str>,
    adapter_ids: &[String],
) -> Result<ProviderAdapterModelsTarget, ConfigError> {
    let base_url = required_trimmed(base_url, "listing adapter models requires a base URL")?;
    let adapter_ids = required_adapter_ids(
        adapter_ids,
        "draft adapter model listing requires explicit adapter_ids",
    )?;
    Ok(ProviderAdapterModelsTarget {
        provider_id: optional_trimmed(provider_id).unwrap_or("draft").to_owned(),
        auth: ProviderAuthConfig::Api(ProviderApiAuthConfig {
            base_url: Some(base_url.to_owned()),
            protocol_paths,
            api_key: optional_trimmed(api_key).map(ToOwned::to_owned),
            api_key_env: optional_trimmed(api_key_env).map(ToOwned::to_owned),
        }),
        adapters: default_http_adapter_model_list_adapters(adapter_ids.as_slice())?,
    })
}

pub fn draft_gitlab_provider_adapter_models_target(
    provider_id: Option<&str>,
    api_key: Option<&str>,
    api_key_env: Option<&str>,
    adapter_ids: &[String],
) -> Result<ProviderAdapterModelsTarget, ConfigError> {
    let adapter_ids = required_adapter_ids(
        adapter_ids,
        "draft adapter model listing requires explicit adapter_ids",
    )?;
    Ok(ProviderAdapterModelsTarget {
        provider_id: optional_trimmed(provider_id).unwrap_or("draft").to_owned(),
        auth: ProviderAuthConfig::Gitlab(ProviderGitlabAuthConfig {
            api_key: optional_trimmed(api_key).map(ToOwned::to_owned),
            api_key_env: optional_trimmed(api_key_env).map(ToOwned::to_owned),
            credential: None,
            instance_url: None,
            ai_gateway_url: None,
            ai_gateway_headers: BTreeMap::new(),
            feature_flags: BTreeMap::new(),
        }),
        adapters: default_http_adapter_model_list_adapters(adapter_ids.as_slice())?,
    })
}

pub fn draft_atomgit_provider_adapter_models_target(
    provider_id: Option<&str>,
    credential: AuthData,
    adapter_ids: &[String],
) -> Result<ProviderAdapterModelsTarget, ConfigError> {
    let adapter_ids = required_adapter_ids(
        adapter_ids,
        "draft atomgit adapter model listing requires explicit adapter_ids",
    )?;
    for adapter_id in &adapter_ids {
        if adapter_id != "openai" {
            return Err(ConfigError::Validation(format!(
                "draft atomgit adapter model listing only supports `openai`; unsupported `{adapter_id}`"
            )));
        }
    }
    Ok(ProviderAdapterModelsTarget {
        provider_id: optional_trimmed(provider_id).unwrap_or("draft").to_owned(),
        auth: ProviderAuthConfig::Credential(ProviderCredentialAuthConfig {
            issuer: CredentialIssuer::AtomGit,
            credential: Some(credential.with_issuer(CredentialIssuer::AtomGit)),
            base_url: None,
            protocol_paths: ProviderProtocolPathsConfig::default(),
            service_key_env: None,
            instance_url: None,
            ai_gateway_url: None,
            ai_gateway_headers: BTreeMap::new(),
            feature_flags: BTreeMap::new(),
        }),
        adapters: default_http_adapter_model_list_adapters(adapter_ids.as_slice())?,
    })
}

pub fn saved_provider_adapter_models_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
    adapter_ids: &[String],
) -> Result<ProviderAdapterModelsTarget, ConfigError> {
    let provider_id = required_trimmed(
        provider_id,
        "provider adapter model listing requires a provider id",
    )?;
    let adapter_ids = required_adapter_ids(
        adapter_ids,
        "saved provider adapter model listing requires explicit adapter_ids",
    )?;
    let mut adapters = BTreeMap::new();
    for adapter_id in adapter_ids {
        let adapter = match resolved.adapters.get(adapter_id.as_str()).cloned() {
            Some(adapter) => adapter,
            None => {
                let mut default_adapters =
                    default_http_adapter_model_list_adapters(std::slice::from_ref(&adapter_id))?;
                default_adapters
                    .remove(adapter_id.as_str())
                    .ok_or_else(|| {
                        ConfigError::Validation(format!(
                            "provider {provider_id} does not define adapter `{adapter_id}`"
                        ))
                    })?
            }
        };
        adapters.insert(adapter_id, adapter);
    }

    Ok(ProviderAdapterModelsTarget {
        provider_id: provider_id.to_owned(),
        auth: resolved.auth.clone(),
        adapters,
    })
}

pub async fn list_provider_adapter_models_for_target(
    target: &ProviderAdapterModelsTarget,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
) -> Vec<super::ProviderAdapterModelsResult> {
    list_provider_adapter_models(
        target.provider_id.as_str(),
        &target.auth,
        &target.adapters,
        client,
        env,
    )
    .await
}

fn default_http_adapter_model_list_adapters(
    adapter_ids: &[String],
) -> Result<BTreeMap<String, ResolvedProviderAdapterConfig>, ConfigError> {
    let mut adapters = BTreeMap::new();
    for adapter_id in adapter_ids {
        let config = match adapter_id.as_str() {
            "openai" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::OpenAi(HttpProviderAdapterConfig {
                    extra_headers: BTreeMap::new(),
                    options: OpenAiProviderOptions {
                        backend: OpenAiBackendConfig::Api,
                        api_mode: OpenAiApiModeConfig::Auto,
                        api_mode_explicit: false,
                        stream_mode: StreamTransportMode::Sse,
                        realtime_ws_url: None,
                        models_url: None,
                        auth_header: "authorization".to_owned(),
                        auth_scheme: Some("Bearer".to_owned()),
                        capability_family: None,
                    },
                }),
            },
            "anthropic" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::Anthropic(HttpProviderAdapterConfig {
                    extra_headers: BTreeMap::new(),
                    options: AnthropicProviderOptions {
                        models_url: None,
                        messages_url: None,
                        auth_header: "x-api-key".to_owned(),
                        auth_scheme: None,
                        extra_beta_header: None,
                        eager_input_streaming: None,
                    },
                }),
            },
            "gemini" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::Gemini(HttpProviderAdapterConfig {
                    extra_headers: BTreeMap::new(),
                    options: SimpleHttpProviderOptions {
                        auth_header: None,
                        auth_scheme: None,
                    },
                }),
            },
            _ => {
                return Err(ConfigError::Validation(format!(
                    "adapter model listing does not support `{adapter_id}`"
                )));
            }
        };
        adapters.insert(adapter_id.clone(), config);
    }

    Ok(adapters)
}

fn required_adapter_ids(adapter_ids: &[String], message: &str) -> Result<Vec<String>, ConfigError> {
    if adapter_ids.is_empty() {
        return Err(ConfigError::Validation(message.to_owned()));
    }

    let mut normalized = Vec::with_capacity(adapter_ids.len());
    for adapter_id in adapter_ids {
        let Some(adapter_id) = optional_trimmed(Some(adapter_id.as_str())) else {
            return Err(ConfigError::Validation(
                "adapter model listing adapter_ids must not contain empty values".to_owned(),
            ));
        };
        normalized.push(adapter_id.to_owned());
    }
    Ok(normalized)
}

fn optional_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn required_trimmed<'a>(value: &'a str, message: &str) -> Result<&'a str, ConfigError> {
    optional_trimmed(Some(value))
        .ok_or_else(|| ConfigError::App(AppError::Config(message.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_openai_adapter() -> ResolvedProviderAdapterConfig {
        ResolvedProviderAdapterConfig {
            enabled: true,
            model_discovery: Default::default(),
            definition: ProviderAdapterDefinition::OpenAi(HttpProviderAdapterConfig {
                extra_headers: BTreeMap::new(),
                options: OpenAiProviderOptions {
                    backend: OpenAiBackendConfig::Api,
                    api_mode: OpenAiApiModeConfig::Auto,
                    api_mode_explicit: false,
                    stream_mode: StreamTransportMode::Sse,
                    realtime_ws_url: None,
                    models_url: None,
                    auth_header: "authorization".to_owned(),
                    auth_scheme: Some("Bearer".to_owned()),
                    capability_family: None,
                },
            }),
        }
    }

    #[test]
    fn draft_target_requires_explicit_adapter_ids() {
        let error = draft_provider_adapter_models_target(
            Some("gateway"),
            "https://example.com",
            ProviderProtocolPathsConfig::default(),
            None,
            Some("OPENAI_API_KEY"),
            &[],
        )
        .expect_err("draft target should reject empty adapter_ids");

        assert!(
            error
                .to_string()
                .contains("draft adapter model listing requires explicit adapter_ids")
        );
    }

    #[test]
    fn draft_target_accepts_explicit_http_adapter_ids() {
        let target = draft_provider_adapter_models_target(
            Some("gateway"),
            "https://example.com",
            ProviderProtocolPathsConfig::default(),
            None,
            Some("OPENAI_API_KEY"),
            &HTTP_ADAPTER_MODEL_LIST_ADAPTER_IDS
                .iter()
                .map(|adapter_id| (*adapter_id).to_owned())
                .collect::<Vec<_>>(),
        )
        .expect("draft target should build");

        assert_eq!(target.provider_id, "gateway");
        assert_eq!(
            target.adapters.keys().cloned().collect::<Vec<_>>(),
            vec![
                "anthropic".to_owned(),
                "gemini".to_owned(),
                "openai".to_owned()
            ]
        );
    }

    #[test]
    fn draft_atomgit_target_uses_inline_credential_and_openai_adapter() {
        let credential = AuthData::OAuth {
            issuer: None,
            refresh: "refresh-token".to_owned(),
            access: "access-token".to_owned(),
            expires_at_ms: 4102444800000,
            account_id: Some("atomgit-user".to_owned()),
            enterprise_url: None,
            user: None,
        };

        let target = draft_atomgit_provider_adapter_models_target(
            Some("atomgit"),
            credential,
            &["openai".to_owned()],
        )
        .expect("draft atomgit target should build");

        assert_eq!(target.provider_id, "atomgit");
        assert_eq!(
            target.adapters.keys().cloned().collect::<Vec<_>>(),
            vec!["openai".to_owned()]
        );
        let ProviderAuthConfig::Credential(config) = target.auth else {
            panic!("atomgit target should use credential auth");
        };
        assert_eq!(config.issuer, CredentialIssuer::AtomGit);
        assert_eq!(
            config.credential.and_then(|credential| credential.issuer()),
            Some(CredentialIssuer::AtomGit)
        );
        assert!(config.base_url.is_none());
    }

    #[test]
    fn draft_atomgit_target_rejects_non_openai_adapter() {
        let credential = AuthData::OAuth {
            issuer: Some(CredentialIssuer::AtomGit),
            refresh: "refresh-token".to_owned(),
            access: "access-token".to_owned(),
            expires_at_ms: 4102444800000,
            account_id: None,
            enterprise_url: None,
            user: None,
        };

        let error = draft_atomgit_provider_adapter_models_target(
            Some("atomgit"),
            credential,
            &["anthropic".to_owned()],
        )
        .expect_err("atomgit target should reject anthropic");

        assert!(
            error
                .to_string()
                .contains("draft atomgit adapter model listing only supports `openai`")
        );
    }

    #[test]
    fn saved_target_requires_explicit_adapter_ids() {
        let mut adapters = BTreeMap::new();
        adapters.insert("openai".to_owned(), configured_openai_adapter());
        let resolved = ResolvedProviderConfig {
            enabled: true,
            default_adapter: "openai".to_owned(),
            default_model: "gpt-5".to_owned(),
            auth: ProviderAuthConfig::Api(ProviderApiAuthConfig {
                base_url: Some("https://example.com".to_owned()),
                protocol_paths: ProviderProtocolPathsConfig::default(),
                api_key: None,
                api_key_env: Some("OPENAI_API_KEY".to_owned()),
            }),
            adapters,
            models: BTreeMap::new(),
        };

        let error = saved_provider_adapter_models_target("gateway", &resolved, &[])
            .expect_err("saved target should reject empty adapter_ids");

        assert!(
            error
                .to_string()
                .contains("saved provider adapter model listing requires explicit adapter_ids")
        );
    }

    #[test]
    fn saved_target_allows_explicit_unconfigured_http_adapters() {
        let mut adapters = BTreeMap::new();
        adapters.insert("openai".to_owned(), configured_openai_adapter());
        let resolved = ResolvedProviderConfig {
            enabled: true,
            default_adapter: "openai".to_owned(),
            default_model: "gpt-5".to_owned(),
            auth: ProviderAuthConfig::Api(ProviderApiAuthConfig {
                base_url: Some("https://example.com".to_owned()),
                protocol_paths: ProviderProtocolPathsConfig::default(),
                api_key: None,
                api_key_env: Some("OPENAI_API_KEY".to_owned()),
            }),
            adapters,
            models: BTreeMap::new(),
        };

        let target = saved_provider_adapter_models_target(
            "gateway",
            &resolved,
            &["anthropic".to_owned(), "gemini".to_owned()],
        )
        .expect("target builds");

        assert!(target.adapters.contains_key("anthropic"));
        assert!(target.adapters.contains_key("gemini"));
    }

    #[test]
    fn saved_target_rejects_unknown_non_http_adapter() {
        let resolved = ResolvedProviderConfig {
            enabled: true,
            default_adapter: "openai".to_owned(),
            default_model: "gpt-5".to_owned(),
            auth: ProviderAuthConfig::None,
            adapters: BTreeMap::new(),
            models: BTreeMap::new(),
        };

        let error =
            saved_provider_adapter_models_target("gateway", &resolved, &["gitlab".to_owned()])
                .expect_err("unsupported adapter should fail");

        assert!(
            error
                .to_string()
                .contains("adapter model listing does not support `gitlab`")
        );
    }
}
