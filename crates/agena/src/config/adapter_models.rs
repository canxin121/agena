use std::collections::BTreeMap;

use crate::error::AppError;

use super::{
    AnthropicProviderOptions, ConfigEnvironment, ConfigError, GeminiProviderOptions,
    HttpProviderAdapterConfig, OpenAiApiModeConfig, OpenAiBackendConfig, OpenAiProviderOptions,
    ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig, ProviderGitlabAuthConfig,
    ProviderProtocolPathsConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, StreamTransportMode, list_provider_adapter_models,
};
pub const HTTP_ADAPTER_MODEL_LIST_ADAPTER_IDS: [&str; 3] = ["openai", "anthropic", "gemini"];

#[derive(Debug, Clone)]
pub struct ProviderAdapterModelsTarget {
    pub provider_id: String,
    pub auth: ProviderAuthConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
}

#[derive(Debug, Clone)]
pub struct ProviderAdapterModelsListing {
    pub provider_id: String,
    pub adapters: Vec<super::ProviderAdapterModelsResult>,
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

pub async fn list_provider_adapter_models_with_config(
    config: &ResolvedConfig,
    target: &ProviderAdapterModelsTarget,
    env: &dyn ConfigEnvironment,
) -> Result<ProviderAdapterModelsListing, AppError> {
    let adapters = list_provider_adapter_models(
        target.provider_id.as_str(),
        &target.auth,
        &target.adapters,
        config.build_provider_http_client()?,
        env,
    )
    .await;
    Ok(ProviderAdapterModelsListing {
        provider_id: target.provider_id.clone(),
        adapters,
    })
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
                    user_agent: None,
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
                    user_agent: None,
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
                    user_agent: None,
                    extra_headers: BTreeMap::new(),
                    options: GeminiProviderOptions {
                        auth_header: None,
                        auth_scheme: None,
                        stream_mode: StreamTransportMode::Sse,
                        realtime_ws_url: None,
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
