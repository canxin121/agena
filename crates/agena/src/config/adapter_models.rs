use std::collections::BTreeMap;

use crate::error::AppError;
use crate::provider::auth::{AuthData, CredentialIssuer};

use super::{
    AmazonBedrockProviderOptions, AnthropicProviderOptions, BedrockSigv4AuthConfig,
    ConfigEnvironment, ConfigError, GeminiProviderOptions, HttpProviderAdapterConfig,
    OllamaProviderOptions, OpenAiApiModeConfig, OpenAiBackendConfig, OpenAiProviderOptions,
    ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig,
    ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig, ProviderGitlabAuthConfig,
    ProviderProtocolPathsConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, StreamTransportMode, list_provider_adapter_models,
};
pub const HTTP_ADAPTER_MODEL_LIST_ADAPTER_IDS: [&str; 3] = ["openai", "anthropic", "gemini"];

const DEFAULT_BEDROCK_BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";
const DEFAULT_BEDROCK_REGION: &str = "us-east-1";

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

pub fn draft_none_provider_adapter_models_target(
    provider_id: Option<&str>,
    adapter_ids: &[String],
) -> Result<ProviderAdapterModelsTarget, ConfigError> {
    let adapter_ids = required_adapter_ids(
        adapter_ids,
        "draft adapter model listing requires explicit adapter_ids",
    )?;
    Ok(ProviderAdapterModelsTarget {
        provider_id: optional_trimmed(provider_id).unwrap_or("draft").to_owned(),
        auth: ProviderAuthConfig::None,
        adapters: default_adapter_model_list_adapters(
            adapter_ids.as_slice(),
            AdapterModelListDefaults::None,
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn draft_credential_provider_adapter_models_target(
    provider_id: Option<&str>,
    issuer: CredentialIssuer,
    credential: Option<AuthData>,
    base_url: Option<&str>,
    protocol_paths: ProviderProtocolPathsConfig,
    service_key_env: Option<&str>,
    instance_url: Option<&str>,
    adapter_ids: &[String],
) -> Result<ProviderAdapterModelsTarget, ConfigError> {
    let adapter_ids = required_adapter_ids(
        adapter_ids,
        "draft adapter model listing requires explicit adapter_ids",
    )?;
    Ok(ProviderAdapterModelsTarget {
        provider_id: optional_trimmed(provider_id).unwrap_or("draft").to_owned(),
        auth: ProviderAuthConfig::Credential(ProviderCredentialAuthConfig {
            issuer,
            credential,
            base_url: optional_trimmed(base_url).map(ToOwned::to_owned),
            protocol_paths,
            service_key_env: optional_trimmed(service_key_env).map(ToOwned::to_owned),
            instance_url: optional_trimmed(instance_url).map(ToOwned::to_owned),
            ai_gateway_url: None,
            ai_gateway_headers: BTreeMap::new(),
            feature_flags: BTreeMap::new(),
        }),
        adapters: default_adapter_model_list_adapters(
            adapter_ids.as_slice(),
            AdapterModelListDefaults::Credential(issuer),
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn draft_bedrock_sigv4_provider_adapter_models_target(
    provider_id: Option<&str>,
    base_url: Option<&str>,
    region: Option<&str>,
    profile: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
    session_token: Option<&str>,
    adapter_ids: &[String],
) -> Result<ProviderAdapterModelsTarget, ConfigError> {
    let adapter_ids = required_adapter_ids(
        adapter_ids,
        "draft adapter model listing requires explicit adapter_ids",
    )?;
    Ok(ProviderAdapterModelsTarget {
        provider_id: optional_trimmed(provider_id).unwrap_or("draft").to_owned(),
        auth: ProviderAuthConfig::BedrockSigv4(BedrockSigv4AuthConfig {
            base_url: optional_trimmed(base_url)
                .unwrap_or(DEFAULT_BEDROCK_BASE_URL)
                .to_owned(),
            region: optional_trimmed(region)
                .unwrap_or(DEFAULT_BEDROCK_REGION)
                .to_owned(),
            profile: optional_trimmed(profile).map(ToOwned::to_owned),
            access_key_id: optional_trimmed(access_key_id).map(ToOwned::to_owned),
            secret_access_key: optional_trimmed(secret_access_key).map(ToOwned::to_owned),
            session_token: optional_trimmed(session_token).map(ToOwned::to_owned),
        }),
        adapters: default_adapter_model_list_adapters(
            adapter_ids.as_slice(),
            AdapterModelListDefaults::BedrockSigv4,
        )?,
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
    let default_kind = adapter_model_list_defaults_for_auth(&resolved.auth);
    let mut adapters = BTreeMap::new();
    for adapter_id in adapter_ids {
        let adapter = match resolved.adapters.get(adapter_id.as_str()).cloned() {
            Some(adapter) => adapter,
            None => {
                let mut default_adapters = default_adapter_model_list_adapters(
                    std::slice::from_ref(&adapter_id),
                    default_kind,
                )?;
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
    default_adapter_model_list_adapters(adapter_ids, AdapterModelListDefaults::Api)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterModelListDefaults {
    None,
    Api,
    Credential(CredentialIssuer),
    BedrockSigv4,
}

fn adapter_model_list_defaults_for_auth(auth: &ProviderAuthConfig) -> AdapterModelListDefaults {
    match auth {
        ProviderAuthConfig::None => AdapterModelListDefaults::None,
        ProviderAuthConfig::Credential(config) => {
            AdapterModelListDefaults::Credential(config.issuer)
        }
        ProviderAuthConfig::BedrockSigv4(_) => AdapterModelListDefaults::BedrockSigv4,
        ProviderAuthConfig::Api(_) | ProviderAuthConfig::Gitlab(_) => AdapterModelListDefaults::Api,
    }
}

fn default_adapter_model_list_adapters(
    adapter_ids: &[String],
    defaults: AdapterModelListDefaults,
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
                        backend: openai_backend_for_listing(defaults),
                        api_mode: openai_api_mode_for_listing(defaults),
                        api_mode_explicit: matches!(
                            defaults,
                            AdapterModelListDefaults::Credential(CredentialIssuer::OpenaiChatgpt)
                        ),
                        stream_mode: StreamTransportMode::Sse,
                        realtime_ws_url: None,
                        models_url: None,
                        auth_header: "authorization".to_owned(),
                        auth_scheme: Some("Bearer".to_owned()),
                        capability_family: openai_capability_family_for_listing(defaults),
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
                        auth_header: anthropic_auth_header_for_listing(defaults).to_owned(),
                        auth_scheme: anthropic_auth_scheme_for_listing(defaults)
                            .map(ToOwned::to_owned),
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
            "ollama" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::Ollama(OllamaProviderOptions {
                    base_url: None,
                }),
            },
            "amazon_bedrock" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::AmazonBedrock(AmazonBedrockProviderOptions),
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

fn openai_backend_for_listing(defaults: AdapterModelListDefaults) -> OpenAiBackendConfig {
    match defaults {
        AdapterModelListDefaults::Credential(CredentialIssuer::OpenaiChatgpt) => {
            OpenAiBackendConfig::ChatgptCodex
        }
        AdapterModelListDefaults::None
        | AdapterModelListDefaults::Api
        | AdapterModelListDefaults::Credential(_)
        | AdapterModelListDefaults::BedrockSigv4 => OpenAiBackendConfig::Api,
    }
}

fn openai_api_mode_for_listing(defaults: AdapterModelListDefaults) -> OpenAiApiModeConfig {
    match defaults {
        AdapterModelListDefaults::Credential(CredentialIssuer::OpenaiChatgpt) => {
            OpenAiApiModeConfig::Responses
        }
        AdapterModelListDefaults::None
        | AdapterModelListDefaults::Api
        | AdapterModelListDefaults::Credential(_)
        | AdapterModelListDefaults::BedrockSigv4 => OpenAiApiModeConfig::Auto,
    }
}

fn openai_capability_family_for_listing(
    defaults: AdapterModelListDefaults,
) -> Option<ProviderCapabilityFamilyConfig> {
    match defaults {
        AdapterModelListDefaults::Credential(CredentialIssuer::GoogleAdc) => {
            Some(ProviderCapabilityFamilyConfig::Gemini)
        }
        AdapterModelListDefaults::None
        | AdapterModelListDefaults::Api
        | AdapterModelListDefaults::Credential(_)
        | AdapterModelListDefaults::BedrockSigv4 => None,
    }
}

fn anthropic_auth_header_for_listing(defaults: AdapterModelListDefaults) -> &'static str {
    match defaults {
        AdapterModelListDefaults::Credential(CredentialIssuer::GithubCopilot) => "authorization",
        AdapterModelListDefaults::None
        | AdapterModelListDefaults::Api
        | AdapterModelListDefaults::Credential(_)
        | AdapterModelListDefaults::BedrockSigv4 => "x-api-key",
    }
}

fn anthropic_auth_scheme_for_listing(defaults: AdapterModelListDefaults) -> Option<&'static str> {
    match defaults {
        AdapterModelListDefaults::Credential(CredentialIssuer::GithubCopilot) => Some("Bearer"),
        AdapterModelListDefaults::None
        | AdapterModelListDefaults::Api
        | AdapterModelListDefaults::Credential(_)
        | AdapterModelListDefaults::BedrockSigv4 => None,
    }
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

    #[test]
    fn draft_credential_listing_uses_issuer_specific_openai_defaults() {
        let target = draft_credential_provider_adapter_models_target(
            Some("chatgpt"),
            CredentialIssuer::OpenaiChatgpt,
            None,
            None,
            ProviderProtocolPathsConfig::default(),
            None,
            None,
            &["openai".to_owned()],
        )
        .expect("target should build");

        let adapter = target.adapters.get("openai").expect("openai adapter");
        let ProviderAdapterDefinition::OpenAi(config) = &adapter.definition else {
            panic!("expected openai adapter");
        };
        assert_eq!(config.options.backend, OpenAiBackendConfig::ChatgptCodex);
        assert_eq!(config.options.api_mode, OpenAiApiModeConfig::Responses);
    }

    #[test]
    fn saved_listing_fallback_uses_provider_auth_defaults() {
        let resolved = ResolvedProviderConfig {
            enabled: true,
            defaults: Default::default(),
            auth: ProviderAuthConfig::Credential(ProviderCredentialAuthConfig {
                issuer: CredentialIssuer::GithubCopilot,
                credential: None,
                base_url: None,
                protocol_paths: ProviderProtocolPathsConfig::default(),
                service_key_env: None,
                instance_url: None,
                ai_gateway_url: None,
                ai_gateway_headers: BTreeMap::new(),
                feature_flags: BTreeMap::new(),
            }),
            adapters: BTreeMap::new(),
            models: BTreeMap::new(),
        };

        let target =
            saved_provider_adapter_models_target("copilot", &resolved, &["anthropic".to_owned()])
                .expect("target should build");

        let adapter = target.adapters.get("anthropic").expect("anthropic adapter");
        let ProviderAdapterDefinition::Anthropic(config) = &adapter.definition else {
            panic!("expected anthropic adapter");
        };
        assert_eq!(config.options.auth_header, "authorization");
        assert_eq!(config.options.auth_scheme.as_deref(), Some("Bearer"));
    }
}
