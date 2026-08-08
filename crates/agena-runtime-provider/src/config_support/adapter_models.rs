use std::collections::BTreeMap;

use crate::ProviderError;
use agena_provider::{
    AuthData, CredentialIssuer, OpenAiResponsesBackendConfig, ProviderCapabilityFamilyConfig,
    ProviderCredentialAuthConfig, ProviderGitlabApiAccessConfig,
    ProviderGitlabCredentialAuthConfig, ProviderHttpCredentialAuthConfig,
    ProviderInlineCredentialAuthConfig, ProviderProtocolPathsConfig,
    ProviderSapAiCoreCredentialAuthConfig, ProviderSecretSourceConfig, StreamTransportMode,
};

#[derive(Debug, Clone)]
/// Model probe result for one adapter.
pub struct ProviderAdapterModelsResult {
    pub adapter_id: String,
    pub enabled: bool,
    pub resolved_base_url: Option<String>,
    pub models: Vec<agena_domain::Model>,
    pub failure: Option<agena_failure::Failure>,
}

use super::{
    AmazonBedrockProviderOptions, AnthropicProviderOptions, ConfigEnvironment, ConfigError,
    GeminiProviderOptions, HttpProviderAdapterConfig, OllamaProviderOptions,
    OpenAiChatCompletionsProviderOptions, OpenAiRealtimeProviderOptions,
    OpenAiResponsesProviderOptions, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ResolvedProviderAdapterConfig, ResolvedProviderConfig,
};
const DEFAULT_BEDROCK_BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";
const DEFAULT_BEDROCK_REGION: &str = "us-east-1";

#[derive(Debug, Clone)]
/// Target of an adapter model probe.
pub struct ProviderAdapterModelsTarget {
    pub provider_id: String,
    pub auth: ProviderAuthConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
}

#[derive(Debug, Clone)]
/// Adapter model probes for a provider.
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
        auth: ProviderAuthConfig::Api(ProviderApiAuthConfig::custom(
            Some(base_url.to_owned()),
            protocol_paths,
            draft_secret_source(api_key, api_key_env),
        )),
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
        auth: ProviderAuthConfig::Api(ProviderApiAuthConfig::Gitlab {
            access: ProviderGitlabApiAccessConfig::ApiKey {
                source: draft_secret_source(api_key, api_key_env).ok_or_else(|| {
                    ConfigError::Validation(
                        "gitlab draft adapter model listing requires an api key source".to_owned(),
                    )
                })?,
            },
            instance_url: None,
            ai_gateway_url: None,
            ai_gateway_headers: BTreeMap::new(),
            feature_flags: BTreeMap::new(),
        }),
        adapters: default_http_adapter_model_list_adapters(adapter_ids.as_slice())?,
    })
}

pub fn draft_cline_api_provider_adapter_models_target(
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
        auth: ProviderAuthConfig::Api(ProviderApiAuthConfig::ClineApi {
            api_key: draft_secret_source(api_key, api_key_env),
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
        auth: ProviderAuthConfig::Credential(match issuer {
            CredentialIssuer::OpenaiChatgpt => ProviderCredentialAuthConfig::OpenaiChatgpt {
                config: ProviderInlineCredentialAuthConfig { credential },
            },
            CredentialIssuer::GithubCopilot => ProviderCredentialAuthConfig::GithubCopilot {
                config: ProviderInlineCredentialAuthConfig { credential },
            },
            CredentialIssuer::Gitlab => ProviderCredentialAuthConfig::Gitlab {
                config: ProviderGitlabCredentialAuthConfig {
                    credential,
                    instance_url: optional_trimmed(instance_url).map(ToOwned::to_owned),
                    ai_gateway_url: None,
                    ai_gateway_headers: BTreeMap::new(),
                    feature_flags: BTreeMap::new(),
                },
            },
            CredentialIssuer::GoogleAdc => ProviderCredentialAuthConfig::GoogleAdc {
                config: ProviderHttpCredentialAuthConfig {
                    base_url: required_trimmed(
                        base_url.unwrap_or_default(),
                        "google_adc adapter model listing requires base_url",
                    )?
                    .to_owned(),
                    protocol_paths,
                },
            },
            CredentialIssuer::SapAiCore => ProviderCredentialAuthConfig::SapAiCore {
                config: ProviderSapAiCoreCredentialAuthConfig {
                    base_url: required_trimmed(
                        base_url.unwrap_or_default(),
                        "sap_ai_core adapter model listing requires base_url",
                    )?
                    .to_owned(),
                    protocol_paths,
                    service_key_env: required_trimmed(
                        service_key_env.unwrap_or_default(),
                        "sap_ai_core adapter model listing requires service_key_env",
                    )?
                    .to_owned(),
                },
            },
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
        auth: ProviderAuthConfig::Api(ProviderApiAuthConfig::BedrockSigv4 {
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

pub async fn list_provider_adapter_models_with_providers(
    providers: &BTreeMap<String, ResolvedProviderConfig>,
    target: &ProviderAdapterModelsTarget,
    env: &dyn ConfigEnvironment,
) -> Result<ProviderAdapterModelsListing, ProviderError> {
    let network = providers
        .get(target.provider_id.as_str())
        .map(|provider| provider.network)
        .unwrap_or_default();
    let client = crate::provider::ProviderRegistry::build_http_client(
        agena_provider::ProviderHttpClientConfig {
            timeout: std::time::Duration::from_secs(network.request_timeout_secs),
            connect_timeout: std::time::Duration::from_secs(network.connect_timeout_secs),
        },
    )?;
    let _ = (client, env);
    Err(ProviderError::Internal(
        "adapter model listing is owned by the runtime adapter composition".to_owned(),
    ))
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
            AdapterModelListDefaults::Credential(config.issuer())
        }
        ProviderAuthConfig::Api(ProviderApiAuthConfig::BedrockSigv4 { .. }) => {
            AdapterModelListDefaults::BedrockSigv4
        }
        ProviderAuthConfig::Api(_) => AdapterModelListDefaults::Api,
    }
}

fn default_adapter_model_list_adapters(
    adapter_ids: &[String],
    defaults: AdapterModelListDefaults,
) -> Result<BTreeMap<String, ResolvedProviderAdapterConfig>, ConfigError> {
    let mut adapters = BTreeMap::new();
    for adapter_id in adapter_ids {
        let config = match adapter_id.as_str() {
            "openai_responses" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::OpenAiResponses(HttpProviderAdapterConfig {
                    user_agent: None,
                    extra_headers: BTreeMap::new(),
                    options: OpenAiResponsesProviderOptions {
                        backend: openai_backend_for_listing(defaults),
                        models_url: None,
                        auth_header: "authorization".to_owned(),
                        auth_scheme: Some("Bearer".to_owned()),
                        capability_family: openai_capability_family_for_listing(defaults),
                    },
                }),
            },
            "openai_chat_completions" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::OpenAiChatCompletions(
                    HttpProviderAdapterConfig {
                        user_agent: None,
                        extra_headers: BTreeMap::new(),
                        options: OpenAiChatCompletionsProviderOptions {
                            models_url: None,
                            auth_header: "authorization".to_owned(),
                            auth_scheme: Some("Bearer".to_owned()),
                            capability_family: openai_capability_family_for_listing(defaults),
                        },
                    },
                ),
            },
            "openai_realtime" => ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::OpenAiRealtime(HttpProviderAdapterConfig {
                    user_agent: None,
                    extra_headers: BTreeMap::new(),
                    options: OpenAiRealtimeProviderOptions {
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
                        auth_header: Some("x-goog-api-key".to_owned()),
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

fn openai_backend_for_listing(defaults: AdapterModelListDefaults) -> OpenAiResponsesBackendConfig {
    match defaults {
        AdapterModelListDefaults::Credential(CredentialIssuer::OpenaiChatgpt) => {
            OpenAiResponsesBackendConfig::ChatgptCodex
        }
        AdapterModelListDefaults::None
        | AdapterModelListDefaults::Api
        | AdapterModelListDefaults::Credential(_)
        | AdapterModelListDefaults::BedrockSigv4 => OpenAiResponsesBackendConfig::Api,
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

fn draft_secret_source(
    api_key: Option<&str>,
    api_key_env: Option<&str>,
) -> Option<ProviderSecretSourceConfig> {
    optional_trimmed(api_key)
        .map(|value| ProviderSecretSourceConfig::Inline(value.to_owned()))
        .or_else(|| {
            optional_trimmed(api_key_env)
                .map(|value| ProviderSecretSourceConfig::Env(value.to_owned()))
        })
}

fn required_trimmed<'a>(value: &'a str, message: &str) -> Result<&'a str, ConfigError> {
    optional_trimmed(Some(value)).ok_or_else(|| ConfigError::Validation(message.to_owned()))
}
