use super::Merge;
use super::{
    BTreeMap, ConfigEnvironment, ConfigError, CredentialIssuer, HarnessesConfig,
    HttpProviderAdapterConfig, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderDefaultsConfig, ProviderKind, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, merge_option, normalize_optional, normalize_optional_string,
    required_string, strip_default_protocol_path_from_base_url, validate_configured_models,
    validate_non_empty_strings,
};
use agena_provider::{
    OpenAiResponsesBackendConfig, ProviderAdapterOverlay, ProviderApiSubtype, ProviderAuthMode,
    ProviderAuthOverlay, ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig,
    ProviderGitlabApiAccessConfig, ProviderGitlabApiAccessOverlay,
    ProviderGitlabCredentialAuthConfig, ProviderHostedToolConfigs,
    ProviderHttpCredentialAuthConfig, ProviderInlineCredentialAuthConfig,
    ProviderModelDiscoveryConfig, ProviderNativeToolKind, ProviderNativeToolRoute,
    ProviderNativeToolsConfig, ProviderNetworkConfig, ProviderOverlay, ProviderProtocolPathsConfig,
    ProviderProtocolPathsOverlay, ProviderSapAiCoreCredentialAuthConfig,
    ProviderSecretSourceConfig, ProviderSecretSourceOverlay, ResolvedProviderModelConfig,
    StreamTransportMode,
};
use agena_runtime::McpConfig;
use std::str::FromStr;

pub(super) trait ProviderOverlayExt {
    fn merge_project_from(&mut self, overlay: Self);

    fn resolve(
        self,
        provider_id: String,
        env: &dyn ConfigEnvironment,
        harnesses: &HarnessesConfig,
        mcp: &McpConfig,
    ) -> Result<(String, ResolvedProviderConfig), ConfigError>;
}

impl ProviderOverlayExt for ProviderOverlay {
    fn merge_project_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        if overlay.defaults.is_some() {
            self.defaults = overlay.defaults;
        }
        if overlay.auth.is_some() {
            self.auth = overlay.auth;
        }
        if overlay.network.is_some() {
            self.network = overlay.network;
        }
        for (adapter_id, adapter) in overlay.adapters {
            match self.adapters.get_mut(&adapter_id) {
                Some(existing) => existing.merge_from(adapter),
                None => {
                    self.adapters.insert(adapter_id, adapter);
                }
            }
        }
    }

    fn resolve(
        self,
        provider_id: String,
        _env: &dyn ConfigEnvironment,
        harnesses: &HarnessesConfig,
        mcp: &McpConfig,
    ) -> Result<(String, ResolvedProviderConfig), ConfigError> {
        let enabled = self.enabled.unwrap_or(true);
        if self.adapters.is_empty() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id,
                message: "provider must declare at least one adapter under `providers.<id>.adapters.<kind>`".to_owned(),
            });
        }

        let mut adapters = BTreeMap::new();
        let mut models = BTreeMap::new();
        for (adapter_id, mut adapter_raw) in self.adapters {
            normalize_model_configs(&mut adapter_raw.models);
            validate_configured_models(
                provider_id.as_str(),
                format!("adapter `{adapter_id}`").as_str(),
                &adapter_raw.models,
            )?;
            let adapter = resolve_adapter(provider_id.as_str(), adapter_id.as_str(), adapter_raw)?;
            for (model_id, configured) in &adapter.models {
                let route_id = format!("{adapter_id}/{model_id}");
                if models.contains_key(route_id.as_str()) {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.clone(),
                        message: format!("duplicate routed model `{route_id}` across adapters"),
                    });
                }
                models.insert(
                    route_id,
                    ResolvedProviderModelConfig {
                        enabled: configured.enabled,
                        native_compaction: configured.native_compaction,
                        agena_tools: configured.agena_tools.clone(),
                        definition: configured.definition.clone(),
                    },
                );
            }
            adapters.insert(adapter_id, adapter.config);
        }

        let provider_defaults = self.defaults.unwrap_or_default();
        if let Some(default_provider) =
            normalize_optional_string(provider_defaults.provider.clone())
            && default_provider.as_str() != provider_id.as_str()
        {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider defaults.provider `{default_provider}` must match provider key `{provider_id}`"
                ),
            });
        }
        let auth = resolve_provider_auth(provider_id.as_str(), self.auth, adapters.values())?;
        validate_provider_auth(provider_id.as_str(), &auth, adapters.values())?;
        let network = self.network.unwrap_or_default();
        let network = ProviderNetworkConfig {
            request_timeout_secs: network
                .request_timeout_secs
                .unwrap_or(ProviderNetworkConfig::default().request_timeout_secs),
            connect_timeout_secs: network
                .connect_timeout_secs
                .unwrap_or(ProviderNetworkConfig::default().connect_timeout_secs),
        };
        if network.request_timeout_secs == 0 || network.connect_timeout_secs == 0 {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: "network timeout values must be greater than zero".to_owned(),
            });
        }
        validate_provider_model_provider_native_tools(
            provider_id.as_str(),
            &models,
            harnesses,
            mcp,
        )?;
        let default_adapter = if let Some(default_adapter) = provider_defaults.adapter.clone() {
            default_adapter
        } else {
            let enabled_adapters = adapters
                .iter()
                .filter(|(_, adapter)| adapter.enabled)
                .map(|(adapter_id, _)| adapter_id.clone())
                .collect::<Vec<_>>();
            (enabled_adapters.len() == 1)
                .then(|| enabled_adapters[0].clone())
                .ok_or_else(|| ConfigError::MissingProviderField {
                    provider_id: provider_id.clone(),
                    field: "defaults.adapter",
                })?
        };
        if default_adapter.trim().is_empty() {
            return Err(ConfigError::MissingProviderField {
                provider_id: provider_id.clone(),
                field: "defaults.adapter",
            });
        }
        let default_model = normalize_optional_string(provider_defaults.model.clone());
        let default_adapter_id = default_adapter.trim().to_owned();
        let default_adapter = adapters.get(default_adapter_id.as_str()).ok_or_else(|| {
            ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider defaults.adapter `{default_adapter_id}` references unknown adapter"
                ),
            }
        })?;
        if !default_adapter.enabled {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider defaults.adapter `{default_adapter_id}` references disabled adapter"
                ),
            });
        }
        if let Some(default_model) = default_model.as_deref() {
            let default_route = format!("{default_adapter_id}/{default_model}");
            if matches!(models.get(default_route.as_str()), Some(configured) if !configured.enabled)
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.clone(),
                    message: format!(
                        "provider defaults.model `{default_model}` references disabled model route `{default_route}`"
                    ),
                });
            }
        }

        let resolved_provider_id = provider_id.clone();

        Ok((
            provider_id,
            ResolvedProviderConfig {
                enabled,
                defaults: ProviderDefaultsConfig {
                    provider: Some(resolved_provider_id),
                    adapter: Some(default_adapter_id),
                    model: default_model,
                    thinking_mode: provider_defaults.thinking_mode,
                    speed_mode: provider_defaults.speed_mode,
                    verbosity: provider_defaults.verbosity,
                    parallel_tool_calls: provider_defaults.parallel_tool_calls,
                },
                auth,
                network,
                adapters,
                models,
            },
        ))
    }
}

fn validate_provider_model_provider_native_tools(
    provider_id: &str,
    models: &BTreeMap<String, ResolvedProviderModelConfig>,
    harnesses: &HarnessesConfig,
    mcp: &McpConfig,
) -> Result<(), ConfigError> {
    for (route_id, model) in models {
        if !model.agena_tools.mode.is_provider_protocol()
            && !model.agena_tools.provider_native.is_empty()
        {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "model route `{route_id}` uses agena_tools.mode `{}` and cannot configure provider-native tools; agena_tools.provider_native requires `provider_protocol`",
                    model.agena_tools.mode.as_str()
                ),
            });
        }
        validate_provider_native_tools(
            provider_id,
            Some(route_id.as_str()),
            &model.agena_tools.provider_native,
            harnesses,
            mcp,
        )?;
    }
    Ok(())
}

fn validate_provider_native_tools(
    provider_id: &str,
    route_id: Option<&str>,
    config: &ProviderNativeToolsConfig,
    harnesses: &HarnessesConfig,
    mcp: &McpConfig,
) -> Result<(), ConfigError> {
    validate_hosted_provider_native_tool_config(provider_id, route_id, &config.hosted)?;

    for tool in ProviderNativeToolKind::ALL {
        if let Some(route) = config.routes.route_for(tool) {
            if !tool.supports_route(route) {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} provider-native tool `{}` does not support route `{route:?}`",
                        provider_native_tool_scope(route_id),
                        tool.config_key(),
                    ),
                });
            }
            if route == ProviderNativeToolRoute::ProviderHarness
                && config.harness.binding_for(tool).is_none()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} provider-native tool `{}` routed to `provider_harness` requires a harness binding",
                        provider_native_tool_scope(route_id),
                        tool.config_key(),
                    ),
                });
            }
            if route == ProviderNativeToolRoute::ProviderConnector
                && tool == ProviderNativeToolKind::RemoteMcp
                && config.connectors.is_empty()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} provider-native tool `remote_mcp` routed to `provider_connector` requires at least one connector",
                        provider_native_tool_scope(route_id)
                    ),
                });
            }
        }
    }

    for tool in [
        ProviderNativeToolKind::Computer,
        ProviderNativeToolKind::Bash,
        ProviderNativeToolKind::TextEditor,
    ] {
        if let Some(reference) = config.harness.binding_for(tool) {
            if reference.name.trim().is_empty() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} provider-native tool `{}` references an empty harness name",
                        provider_native_tool_scope(route_id),
                        tool.config_key(),
                    ),
                });
            }
            if !harnesses.contains(reference) {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} provider-native tool `{}` references missing {:?} harness `{}`",
                        provider_native_tool_scope(route_id),
                        tool.config_key(),
                        reference.kind,
                        reference.name
                    ),
                });
            }
        }
    }

    for (name, connector) in &config.connectors {
        if name.trim().is_empty() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "{} connector name cannot be empty",
                    provider_native_tool_scope(route_id)
                ),
            });
        }
        if connector.server.trim().is_empty() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "{} connector `{name}` must set non-empty `server`",
                    provider_native_tool_scope(route_id)
                ),
            });
        }
        if !mcp.servers.contains_key(connector.server.as_str()) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "{} connector `{name}` references unknown MCP server `{}`",
                    provider_native_tool_scope(route_id),
                    connector.server
                ),
            });
        }
        for tool_name in &connector.tool_filter {
            if tool_name.trim().is_empty() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} connector `{name}` contains an empty tool name in `tool_filter`",
                        provider_native_tool_scope(route_id)
                    ),
                });
            }
        }
    }

    Ok(())
}

fn validate_hosted_provider_native_tool_config(
    provider_id: &str,
    route_id: Option<&str>,
    hosted: &ProviderHostedToolConfigs,
) -> Result<(), ConfigError> {
    validate_non_empty_strings(
        provider_id,
        hosted_provider_native_tool_path(route_id, "web_search.allowed_domains").as_str(),
        &hosted.web_search.allowed_domains,
    )?;
    validate_non_empty_strings(
        provider_id,
        hosted_provider_native_tool_path(route_id, "web_search.blocked_domains").as_str(),
        &hosted.web_search.blocked_domains,
    )?;
    validate_non_empty_strings(
        provider_id,
        hosted_provider_native_tool_path(route_id, "file_search.vector_store_ids").as_str(),
        &hosted.file_search.vector_store_ids,
    )?;
    validate_non_empty_strings(
        provider_id,
        hosted_provider_native_tool_path(route_id, "code_execution.container.file_ids").as_str(),
        &hosted.code_execution.container.file_ids,
    )?;
    if matches!(hosted.web_search.max_results, Some(0)) {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!(
                "{} provider-native tool `web_search` hosted `max_results` must be greater than 0",
                provider_native_tool_scope(route_id)
            ),
        });
    }
    if matches!(hosted.file_search.max_results, Some(0)) {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!(
                "{} provider-native tool `file_search` hosted `max_results` must be greater than 0",
                provider_native_tool_scope(route_id)
            ),
        });
    }
    if matches!(hosted.url_context.max_urls, Some(0)) {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!(
                "{} provider-native tool `url_context` hosted `max_urls` must be greater than 0",
                provider_native_tool_scope(route_id)
            ),
        });
    }
    Ok(())
}

fn provider_native_tool_scope(route_id: Option<&str>) -> String {
    route_id
        .map(|route_id| format!("provider model `{route_id}`"))
        .unwrap_or_else(|| "provider".to_owned())
}

fn hosted_provider_native_tool_path(route_id: Option<&str>, suffix: &str) -> String {
    route_id
        .map(|route_id| format!("models.{route_id}.agena_tools.provider_native.hosted.{suffix}"))
        .unwrap_or_else(|| format!("agena_tools.provider_native.hosted.{suffix}"))
}

#[derive(Debug, Clone)]
struct ResolvedAdapterWithModels {
    config: ResolvedProviderAdapterConfig,
    models: BTreeMap<String, ResolvedProviderModelConfig>,
}

const DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV: &str = "AICORE_SERVICE_KEY";

fn normalize_model_configs(models: &mut BTreeMap<String, ResolvedProviderModelConfig>) {
    for configured in models.values_mut() {
        configured.definition.capabilities.normalize_compact_patch();
    }
}

fn resolve_adapter(
    provider_id: &str,
    adapter_id: &str,
    raw: ProviderAdapterOverlay,
) -> Result<ResolvedAdapterWithModels, ConfigError> {
    let kind =
        ProviderKind::from_str(adapter_id).map_err(|error| ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: error.to_string(),
        })?;
    let config = resolve_adapter_config(
        provider_id,
        adapter_id,
        kind,
        raw.backend,
        raw.enabled,
        raw.model_discovery,
        raw.base_url,
        raw.models_url,
        raw.capability_family,
        raw.messages_url,
        raw.auth_header,
        raw.auth_scheme,
        raw.user_agent,
        raw.extra_beta_header,
        raw.eager_input_streaming,
        raw.extra_headers,
        raw.stream_mode,
        raw.realtime_ws_url,
        raw.instance_url,
        raw.ai_gateway_url,
        raw.ai_gateway_headers,
        raw.feature_flags,
    )?;
    let models = raw
        .models
        .into_iter()
        .map(|(model_id, configured)| Ok((model_id.clone(), configured)))
        .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
    Ok(ResolvedAdapterWithModels { config, models })
}

#[allow(clippy::too_many_arguments)]
fn resolve_adapter_config(
    provider_id: &str,
    _adapter_id: &str,
    kind: ProviderKind,
    backend: Option<OpenAiResponsesBackendConfig>,
    enabled: Option<bool>,
    model_discovery: Option<ProviderModelDiscoveryConfig>,
    base_url: Option<String>,
    models_url: Option<String>,
    capability_family: Option<ProviderCapabilityFamilyConfig>,
    messages_url: Option<String>,
    auth_header: Option<String>,
    auth_scheme: Option<String>,
    user_agent: Option<String>,
    extra_beta_header: Option<String>,
    eager_input_streaming: Option<bool>,
    extra_headers: BTreeMap<String, String>,
    stream_mode: Option<StreamTransportMode>,
    realtime_ws_url: Option<String>,
    instance_url: Option<String>,
    ai_gateway_url: Option<String>,
    ai_gateway_headers: BTreeMap<String, String>,
    feature_flags: BTreeMap<String, bool>,
) -> Result<ResolvedProviderAdapterConfig, ConfigError> {
    let definition = match kind {
        ProviderKind::Ollama => {
            ProviderAdapterDefinition::Ollama(agena_runtime::OllamaProviderOptions {
                base_url: normalize_optional(base_url),
            })
        }
        ProviderKind::OpenAiResponses => {
            let backend = backend.unwrap_or_default();
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "openai_responses adapter does not support `base_url`; configure provider auth endpoint instead"
                            .to_owned(),
                });
            }
            if stream_mode.is_some() || normalize_optional(realtime_ws_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai_responses adapter does not accept Realtime transport fields"
                        .to_owned(),
                });
            }
            ProviderAdapterDefinition::OpenAiResponses(HttpProviderAdapterConfig {
                user_agent: normalize_optional(user_agent),
                extra_headers,
                options: agena_runtime::OpenAiResponsesProviderOptions {
                    backend,
                    models_url: normalize_optional(models_url),
                    auth_header: auth_header.unwrap_or_else(|| "authorization".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme)
                        .or_else(|| Some("Bearer".to_owned())),
                    capability_family,
                },
            })
        }
        ProviderKind::OpenAiChatCompletions => {
            if backend.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai_chat_completions adapter does not accept a Responses backend"
                        .to_owned(),
                });
            }
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai_chat_completions adapter does not support `base_url`; configure provider auth endpoint instead".to_owned(),
                });
            }
            if stream_mode.is_some() || normalize_optional(realtime_ws_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "openai_chat_completions adapter does not accept Realtime transport fields"
                            .to_owned(),
                });
            }
            ProviderAdapterDefinition::OpenAiChatCompletions(HttpProviderAdapterConfig {
                user_agent: normalize_optional(user_agent),
                extra_headers,
                options: agena_runtime::OpenAiChatCompletionsProviderOptions {
                    models_url: normalize_optional(models_url),
                    auth_header: auth_header.unwrap_or_else(|| "authorization".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme)
                        .or_else(|| Some("Bearer".to_owned())),
                    capability_family,
                },
            })
        }
        ProviderKind::OpenAiRealtime => {
            if backend.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai_realtime adapter does not accept a Responses backend"
                        .to_owned(),
                });
            }
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai_realtime adapter does not support `base_url`; configure provider auth endpoint instead".to_owned(),
                });
            }
            if stream_mode.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai_realtime is already a WebSocket protocol adapter and does not accept `stream_mode`".to_owned(),
                });
            }
            ProviderAdapterDefinition::OpenAiRealtime(HttpProviderAdapterConfig {
                user_agent: normalize_optional(user_agent),
                extra_headers,
                options: agena_runtime::OpenAiRealtimeProviderOptions {
                    realtime_ws_url: normalize_optional(realtime_ws_url),
                    models_url: normalize_optional(models_url),
                    auth_header: auth_header.unwrap_or_else(|| "authorization".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme)
                        .or_else(|| Some("Bearer".to_owned())),
                    capability_family,
                },
            })
        }
        ProviderKind::Anthropic => {
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "anthropic adapter does not support `base_url`; configure provider auth endpoint instead"
                            .to_owned(),
                });
            }
            ProviderAdapterDefinition::Anthropic(HttpProviderAdapterConfig {
                user_agent: normalize_optional(user_agent),
                extra_headers,
                options: agena_runtime::AnthropicProviderOptions {
                    models_url: normalize_optional(models_url),
                    messages_url: normalize_optional(messages_url),
                    auth_header: auth_header.unwrap_or_else(|| "x-api-key".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme),
                    extra_beta_header: normalize_optional(extra_beta_header),
                    eager_input_streaming,
                },
            })
        }
        ProviderKind::Gemini => {
            let stream_mode = stream_mode.unwrap_or(StreamTransportMode::Sse);
            let realtime_ws_url = normalize_optional(realtime_ws_url);
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "gemini adapter does not support `base_url`; configure provider auth endpoint instead"
                            .to_owned(),
                });
            }
            ProviderAdapterDefinition::Gemini(HttpProviderAdapterConfig {
                user_agent: normalize_optional(user_agent),
                extra_headers,
                options: agena_runtime::GeminiProviderOptions {
                    auth_header: normalize_optional(auth_header)
                        .or_else(|| Some("x-goog-api-key".to_owned())),
                    auth_scheme: normalize_optional(auth_scheme),
                    stream_mode,
                    realtime_ws_url,
                },
            })
        }
        ProviderKind::Gitlab => {
            ProviderAdapterDefinition::Gitlab(agena_runtime::GitlabProviderOptions {
                instance_url: normalize_optional(instance_url),
                ai_gateway_url: normalize_optional(ai_gateway_url),
                ai_gateway_headers,
                feature_flags,
            })
        }
        ProviderKind::AmazonBedrock => {
            ProviderAdapterDefinition::AmazonBedrock(agena_runtime::AmazonBedrockProviderOptions)
        }
    };

    Ok(ResolvedProviderAdapterConfig {
        enabled: enabled.unwrap_or(false),
        model_discovery: model_discovery.unwrap_or_default(),
        definition,
    })
}

fn resolve_provider_auth<'a>(
    provider_id: &str,
    raw_auth: Option<ProviderAuthOverlay>,
    adapters: impl IntoIterator<Item = &'a ResolvedProviderAdapterConfig>,
) -> Result<ProviderAuthConfig, ConfigError> {
    let adapters = adapters.into_iter().collect::<Vec<_>>();
    let raw_auth = raw_auth.unwrap_or_default();
    let mode = raw_auth
        .mode
        .unwrap_or_else(|| infer_provider_auth_mode(&raw_auth, &adapters));
    match mode {
        ProviderAuthMode::None => Ok(ProviderAuthConfig::None),
        ProviderAuthMode::Api => resolve_api_auth(provider_id, raw_auth, &adapters),
        ProviderAuthMode::Credential => {
            let issuer = raw_auth
                .issuer
                .ok_or_else(|| ConfigError::MissingProviderField {
                    provider_id: provider_id.to_owned(),
                    field: "issuer",
                })?;
            resolve_credential_auth(provider_id, raw_auth, issuer)
        }
    }
}

fn resolve_api_auth(
    provider_id: &str,
    raw_auth: ProviderAuthOverlay,
    _adapters: &[&ResolvedProviderAdapterConfig],
) -> Result<ProviderAuthConfig, ConfigError> {
    if raw_auth.issuer.is_some() {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "auth mode `api` does not accept `issuer`; use auth mode `credential`"
                .to_owned(),
        });
    }

    let Some(subtype) = raw_auth.subtype else {
        return Err(ConfigError::MissingProviderField {
            provider_id: provider_id.to_owned(),
            field: "subtype",
        });
    };
    match subtype {
        ProviderApiSubtype::Custom => {
            if raw_auth.service_key_env.is_some()
                || raw_auth.credential.is_some()
                || raw_auth.access.is_some()
                || raw_auth.instance_url.is_some()
                || raw_auth.ai_gateway_url.is_some()
                || !raw_auth.ai_gateway_headers.is_empty()
                || !raw_auth.feature_flags.is_empty()
                || raw_auth.profile.is_some()
                || raw_auth.access_key_id.is_some()
                || raw_auth.secret_access_key.is_some()
                || raw_auth.session_token.is_some()
                || raw_auth.region.is_some()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "api subtype `custom` only accepts `base_url`, `protocol_paths`, and `api_key`"
                            .to_owned(),
                });
            }
            let has_explicit_protocol_paths = raw_auth.protocol_paths.is_some();
            let protocol_paths =
                resolve_protocol_paths(provider_id, raw_auth.protocol_paths, "protocol_paths")?;
            Ok(ProviderAuthConfig::Api(ProviderApiAuthConfig::custom(
                normalize_optional(raw_auth.base_url).map(|base_url| {
                    if has_explicit_protocol_paths {
                        base_url
                    } else {
                        strip_default_protocol_path_from_base_url(base_url)
                    }
                }),
                protocol_paths,
                raw_auth.api_key.map(resolve_secret_source),
            )))
        }
        ProviderApiSubtype::ClineApi => {
            if raw_auth.base_url.is_some()
                || raw_auth.protocol_paths.is_some()
                || raw_auth.service_key_env.is_some()
                || raw_auth.credential.is_some()
                || raw_auth.access.is_some()
                || raw_auth.instance_url.is_some()
                || raw_auth.ai_gateway_url.is_some()
                || !raw_auth.ai_gateway_headers.is_empty()
                || !raw_auth.feature_flags.is_empty()
                || raw_auth.profile.is_some()
                || raw_auth.access_key_id.is_some()
                || raw_auth.secret_access_key.is_some()
                || raw_auth.session_token.is_some()
                || raw_auth.region.is_some()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "api subtype `cline_api` only accepts `api_key`".to_owned(),
                });
            }
            Ok(ProviderAuthConfig::Api(ProviderApiAuthConfig::ClineApi {
                api_key: raw_auth.api_key.map(resolve_secret_source),
            }))
        }
        ProviderApiSubtype::Gitlab => {
            if raw_auth.base_url.is_some()
                || raw_auth.protocol_paths.is_some()
                || raw_auth.service_key_env.is_some()
                || raw_auth.profile.is_some()
                || raw_auth.access_key_id.is_some()
                || raw_auth.secret_access_key.is_some()
                || raw_auth.session_token.is_some()
                || raw_auth.region.is_some()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "api subtype `gitlab_api` does not accept `base_url`, `protocol_paths`, `service_key_env`, `profile`, `access_key_id`, `secret_access_key`, `session_token`, or `region`".to_owned(),
                });
            }
            if raw_auth.api_key.is_some() || raw_auth.credential.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "api subtype `gitlab_api` uses `access`; it does not accept top-level `api_key` or `credential`"
                            .to_owned(),
                });
            }
            let Some(access) = raw_auth.access.map(resolve_gitlab_api_access) else {
                return Err(ConfigError::MissingProviderField {
                    provider_id: provider_id.to_owned(),
                    field: "access",
                });
            };
            Ok(ProviderAuthConfig::Api(ProviderApiAuthConfig::Gitlab {
                access,
                instance_url: normalize_optional(raw_auth.instance_url),
                ai_gateway_url: normalize_optional(raw_auth.ai_gateway_url),
                ai_gateway_headers: raw_auth.ai_gateway_headers,
                feature_flags: raw_auth.feature_flags,
            }))
        }
        ProviderApiSubtype::BedrockSigv4 => {
            let access_key_id = normalize_optional(raw_auth.access_key_id);
            let secret_access_key = normalize_optional(raw_auth.secret_access_key);
            if access_key_id.is_some() ^ secret_access_key.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "access_key_id and secret_access_key must be set together".to_owned(),
                });
            }
            if raw_auth.protocol_paths.is_some()
                || raw_auth.api_key.is_some()
                || raw_auth.access.is_some()
                || raw_auth.service_key_env.is_some()
                || raw_auth.credential.is_some()
                || raw_auth.instance_url.is_some()
                || raw_auth.ai_gateway_url.is_some()
                || !raw_auth.ai_gateway_headers.is_empty()
                || !raw_auth.feature_flags.is_empty()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "api subtype `bedrock_sigv4` does not accept `protocol_paths`, `api_key`, `access`, `service_key_env`, `credential`, `instance_url`, `ai_gateway_url`, `ai_gateway_headers`, or `feature_flags`".to_owned(),
                });
            }
            Ok(ProviderAuthConfig::Api(
                ProviderApiAuthConfig::BedrockSigv4 {
                    base_url: normalize_optional(raw_auth.base_url).unwrap_or_else(|| {
                        "https://bedrock-runtime.us-east-1.amazonaws.com".to_owned()
                    }),
                    region: normalize_optional(raw_auth.region)
                        .unwrap_or_else(|| "us-east-1".to_owned()),
                    profile: normalize_optional(raw_auth.profile),
                    access_key_id,
                    secret_access_key,
                    session_token: normalize_optional(raw_auth.session_token),
                },
            ))
        }
    }
}

fn resolve_secret_source(raw: ProviderSecretSourceOverlay) -> ProviderSecretSourceConfig {
    match raw {
        ProviderSecretSourceOverlay::Inline(value) => {
            ProviderSecretSourceConfig::Inline(value.trim().to_owned())
        }
        ProviderSecretSourceOverlay::Env(value) => {
            ProviderSecretSourceConfig::Env(value.trim().to_owned())
        }
    }
}

fn resolve_gitlab_api_access(raw: ProviderGitlabApiAccessOverlay) -> ProviderGitlabApiAccessConfig {
    match raw {
        ProviderGitlabApiAccessOverlay::ApiKey { source } => {
            ProviderGitlabApiAccessConfig::ApiKey {
                source: resolve_secret_source(source),
            }
        }
        ProviderGitlabApiAccessOverlay::Credential { credential } => {
            ProviderGitlabApiAccessConfig::Credential {
                credential: credential.for_issuer(CredentialIssuer::Gitlab),
            }
        }
    }
}

fn resolve_credential_auth(
    provider_id: &str,
    raw_auth: ProviderAuthOverlay,
    issuer: CredentialIssuer,
) -> Result<ProviderAuthConfig, ConfigError> {
    let credential = raw_auth
        .credential
        .clone()
        .map(|credential| credential.for_issuer(issuer));
    let base_url = normalize_optional(raw_auth.base_url.clone());
    let api_key = raw_auth.api_key.as_ref().and_then(|value| {
        resolve_secret_source(value.clone())
            .inline()
            .map(ToOwned::to_owned)
    });
    let api_key_env = raw_auth.api_key.as_ref().and_then(|value| {
        resolve_secret_source(value.clone())
            .env()
            .map(ToOwned::to_owned)
    });
    let service_key_env = normalize_optional(raw_auth.service_key_env.clone());
    let instance_url = normalize_optional(raw_auth.instance_url.clone());
    let ai_gateway_url = normalize_optional(raw_auth.ai_gateway_url.clone());

    if issuer.uses_http_endpoint() {
        if api_key.is_some() || api_key_env.is_some() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "credential issuer `{}` does not accept `api_key`; use auth mode `api` for direct tokens",
                    issuer_label(issuer)
                ),
            });
        }
        if credential.is_some() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "credential issuer `{}` does not accept inline `credential` data",
                    issuer_label(issuer)
                ),
            });
        }
        let base_url = required_string(provider_id, "base_url", raw_auth.base_url)?;
        let protocol_paths =
            resolve_protocol_paths(provider_id, raw_auth.protocol_paths, "protocol_paths")?;
        return Ok(ProviderAuthConfig::Credential(match issuer {
            CredentialIssuer::GoogleAdc => {
                if service_key_env.is_some() {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: format!(
                            "credential issuer `{}` does not accept `service_key_env`",
                            issuer_label(issuer)
                        ),
                    });
                }
                ProviderCredentialAuthConfig::GoogleAdc {
                    config: ProviderHttpCredentialAuthConfig {
                        base_url,
                        protocol_paths,
                    },
                }
            }
            CredentialIssuer::SapAiCore => ProviderCredentialAuthConfig::SapAiCore {
                config: ProviderSapAiCoreCredentialAuthConfig {
                    base_url,
                    protocol_paths,
                    service_key_env: service_key_env
                        .unwrap_or_else(|| DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV.to_owned()),
                },
            },
            _ => {
                if service_key_env.is_some() {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: format!(
                            "credential issuer `{}` does not accept `service_key_env`",
                            issuer_label(issuer)
                        ),
                    });
                }
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "credential issuer `{}` does not support endpoint auth",
                        issuer_label(issuer)
                    ),
                });
            }
        }));
    }

    if issuer == CredentialIssuer::Gitlab {
        if base_url.is_some()
            || raw_auth.protocol_paths.is_some()
            || api_key.is_some()
            || api_key_env.is_some()
            || raw_auth.access.is_some()
            || service_key_env.is_some()
        {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message:
                    "credential issuer `gitlab` does not accept `base_url`, `protocol_paths`, `api_key`, `access`, or `service_key_env`"
                        .to_owned(),
            });
        }

        return Ok(ProviderAuthConfig::Credential(
            ProviderCredentialAuthConfig::Gitlab {
                config: ProviderGitlabCredentialAuthConfig {
                    credential,
                    instance_url,
                    ai_gateway_url,
                    ai_gateway_headers: raw_auth.ai_gateway_headers,
                    feature_flags: raw_auth.feature_flags,
                },
            },
        ));
    }

    if base_url.is_some()
        || raw_auth.protocol_paths.is_some()
        || api_key.is_some()
        || api_key_env.is_some()
        || raw_auth.access.is_some()
        || instance_url.is_some()
        || ai_gateway_url.is_some()
        || !raw_auth.ai_gateway_headers.is_empty()
        || !raw_auth.feature_flags.is_empty()
        || service_key_env.is_some()
    {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message:
                "auth mode `credential` does not accept `base_url`, `protocol_paths`, `api_key`, `access`, `instance_url`, `ai_gateway_url`, `ai_gateway_headers`, `feature_flags`, or `service_key_env` for this issuer"
                    .to_owned(),
        });
    }

    Ok(ProviderAuthConfig::Credential(match issuer {
        CredentialIssuer::OpenaiChatgpt => ProviderCredentialAuthConfig::OpenaiChatgpt {
            config: ProviderInlineCredentialAuthConfig { credential },
        },
        CredentialIssuer::GithubCopilot => ProviderCredentialAuthConfig::GithubCopilot {
            config: ProviderInlineCredentialAuthConfig { credential },
        },
        CredentialIssuer::Gitlab | CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore => {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "credential issuer `{}` requires issuer-specific auth fields",
                    issuer_label(issuer)
                ),
            });
        }
    }))
}

fn resolve_protocol_paths(
    provider_id: &str,
    raw: Option<ProviderProtocolPathsOverlay>,
    field: &str,
) -> Result<ProviderProtocolPathsConfig, ConfigError> {
    let raw = raw.unwrap_or_default();
    Ok(ProviderProtocolPathsConfig {
        openai: normalize_protocol_path(
            provider_id,
            format!("{field}.openai").as_str(),
            raw.openai.unwrap_or_else(|| "/v1".to_owned()),
        )?,
        anthropic: normalize_protocol_path(
            provider_id,
            format!("{field}.anthropic").as_str(),
            raw.anthropic.unwrap_or_else(|| "/v1".to_owned()),
        )?,
        gemini: normalize_protocol_path(
            provider_id,
            format!("{field}.gemini").as_str(),
            raw.gemini.unwrap_or_else(|| "/v1beta".to_owned()),
        )?,
    })
}

fn normalize_protocol_path(
    provider_id: &str,
    field: &str,
    value: String,
) -> Result<String, ConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok(String::new());
    }
    if trimmed.contains("://") || trimmed.contains('?') || trimmed.contains('#') {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("provider auth {field} must be a relative path, got `{trimmed}`"),
        });
    }
    Ok(format!("/{}", trimmed.trim_matches('/')))
}

fn infer_provider_auth_mode(
    raw_auth: &ProviderAuthOverlay,
    adapters: &[&ResolvedProviderAdapterConfig],
) -> ProviderAuthMode {
    if raw_auth.credential.is_some() || raw_auth.issuer.is_some() {
        return ProviderAuthMode::Credential;
    }
    if adapters
        .iter()
        .all(|adapter| matches!(adapter.definition, ProviderAdapterDefinition::Ollama(_)))
    {
        return ProviderAuthMode::None;
    }
    ProviderAuthMode::Api
}

fn validate_provider_auth<'a>(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapters: impl IntoIterator<Item = &'a ResolvedProviderAdapterConfig>,
) -> Result<(), ConfigError> {
    for adapter in adapters {
        match (auth, &adapter.definition) {
            (ProviderAuthConfig::None, ProviderAdapterDefinition::Ollama(_)) => {}
            (ProviderAuthConfig::None, _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `none` only supports `ollama` adapters".to_owned(),
                });
            }
            (ProviderAuthConfig::Api(_), ProviderAdapterDefinition::Ollama(_)) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "api auth is not supported by `ollama` adapters".to_owned(),
                });
            }
            (ProviderAuthConfig::Api(_), ProviderAdapterDefinition::OpenAiResponses(config))
                if matches!(
                    config.options.backend,
                    OpenAiResponsesBackendConfig::ChatgptCodex
                ) =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai backend `chatgpt_codex` only supports credential auth"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::Api(api), definition) => {
                if matches!(api, ProviderApiAuthConfig::BedrockSigv4 { .. }) {
                    if !matches!(definition, ProviderAdapterDefinition::AmazonBedrock(_)) {
                        return Err(ConfigError::InvalidProviderConfig {
                            provider_id: provider_id.to_owned(),
                            message: "api subtype `bedrock_sigv4` only supports `amazon_bedrock` adapters".to_owned(),
                        });
                    }
                    continue;
                }
                if matches!(api, ProviderApiAuthConfig::Gitlab { .. }) {
                    match definition {
                        ProviderAdapterDefinition::OpenAiResponses(config)
                            if matches!(
                                config.options.backend,
                                OpenAiResponsesBackendConfig::Api
                            ) => {}
                        ProviderAdapterDefinition::OpenAiChatCompletions(_) => {}
                        ProviderAdapterDefinition::Anthropic(_)
                        | ProviderAdapterDefinition::Gitlab(_) => {}
                        ProviderAdapterDefinition::OpenAiResponses(_) => {
                            return Err(ConfigError::InvalidProviderConfig {
                                provider_id: provider_id.to_owned(),
                                message: "api subtype `gitlab` only supports `openai_responses` with backend `api`".to_owned(),
                            });
                        }
                        _ => {
                            return Err(ConfigError::InvalidProviderConfig {
                                provider_id: provider_id.to_owned(),
                                message: "api subtype `gitlab` only supports OpenAI Responses, OpenAI Chat Completions, Anthropic, or GitLab adapters".to_owned(),
                            });
                        }
                    }
                    continue;
                }
                if matches!(api, ProviderApiAuthConfig::ClineApi { .. }) {
                    match definition {
                        ProviderAdapterDefinition::OpenAiChatCompletions(_) => {}
                        _ => {
                            return Err(ConfigError::InvalidProviderConfig {
                                provider_id: provider_id.to_owned(),
                                message: "api subtype `cline_api` only supports the `openai_chat_completions` adapter".to_owned(),
                            });
                        }
                    }
                    continue;
                }
                if api_auth_requires_base_url(definition) && api.custom_base_url().is_none() {
                    let adapter_label = match definition {
                        ProviderAdapterDefinition::OpenAiResponses(_) => "openai_responses",
                        ProviderAdapterDefinition::OpenAiChatCompletions(_) => {
                            "openai_chat_completions"
                        }
                        ProviderAdapterDefinition::OpenAiRealtime(_) => "openai_realtime",
                        ProviderAdapterDefinition::Anthropic(_) => "anthropic",
                        ProviderAdapterDefinition::Gemini(_) => "gemini",
                        ProviderAdapterDefinition::Gitlab(_) => "gitlab",
                        ProviderAdapterDefinition::Ollama(_) => "ollama",
                        ProviderAdapterDefinition::AmazonBedrock(_) => "amazon_bedrock",
                    };
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: format!(
                            "api auth requires `base_url` for `{adapter_label}` adapters"
                        ),
                    });
                }
            }
            (
                ProviderAuthConfig::Credential(config),
                ProviderAdapterDefinition::OpenAiResponses(options),
            ) => match (config.issuer(), options.options.backend) {
                (CredentialIssuer::OpenaiChatgpt, OpenAiResponsesBackendConfig::ChatgptCodex) => {}
                (CredentialIssuer::GithubCopilot, OpenAiResponsesBackendConfig::Api) => {}
                (CredentialIssuer::Gitlab, OpenAiResponsesBackendConfig::Api) => {}
                _ => {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "credential issuer does not match `openai_responses` adapter requirements"
                            .to_owned(),
                    });
                }
            },
            (
                ProviderAuthConfig::Credential(config),
                ProviderAdapterDefinition::OpenAiChatCompletions(options),
            ) => match config.issuer() {
                CredentialIssuer::GithubCopilot | CredentialIssuer::Gitlab => {}
                CredentialIssuer::GoogleAdc
                    if matches!(
                        options.options.capability_family,
                        Some(ProviderCapabilityFamilyConfig::Gemini)
                    ) => {}
                CredentialIssuer::SapAiCore => {}
                _ => {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "credential issuer does not match `openai_chat_completions` adapter requirements".to_owned(),
                    });
                }
            },
            (ProviderAuthConfig::Credential(_), ProviderAdapterDefinition::OpenAiRealtime(_)) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai_realtime requires API authentication".to_owned(),
                });
            }
            (ProviderAuthConfig::Credential(config), ProviderAdapterDefinition::Anthropic(_)) => {
                if !matches!(
                    config.issuer(),
                    CredentialIssuer::GithubCopilot | CredentialIssuer::Gitlab
                ) {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "credential issuer does not match `anthropic` adapter requirements; use `api` auth with a Claude Console API key for first-party Anthropic access"
                            .to_owned(),
                    });
                }
            }
            (ProviderAuthConfig::Credential(config), ProviderAdapterDefinition::Gemini(_))
                if config.issuer() == CredentialIssuer::GithubCopilot =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "github_copilot credential does not support `gemini` adapter; use an OpenAI protocol adapter for Copilot Gemini models"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::Credential(config), ProviderAdapterDefinition::Gitlab(_))
                if config.issuer() == CredentialIssuer::Gitlab => {}
            (ProviderAuthConfig::Credential(config), _)
                if config.issuer() == CredentialIssuer::GoogleAdc =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "credential issuer `google_adc` only supports Vertex-style `openai_chat_completions` adapters"
                            .to_owned(),
                });
            }
            (ProviderAuthConfig::Credential(config), _)
                if config.issuer() == CredentialIssuer::SapAiCore =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "credential issuer `sap_ai_core` only supports `openai_chat_completions` adapters"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::Credential(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "credential auth is not supported by this adapter".to_owned(),
                });
            }
        }
    }

    Ok(())
}

fn api_auth_requires_base_url(definition: &ProviderAdapterDefinition) -> bool {
    matches!(
        definition,
        ProviderAdapterDefinition::OpenAiResponses(_)
            | ProviderAdapterDefinition::OpenAiChatCompletions(_)
            | ProviderAdapterDefinition::OpenAiRealtime(_)
            | ProviderAdapterDefinition::Anthropic(_)
            | ProviderAdapterDefinition::Gemini(_)
    )
}

fn issuer_label(issuer: CredentialIssuer) -> &'static str {
    match issuer {
        CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        CredentialIssuer::GithubCopilot => "github_copilot",
        CredentialIssuer::Gitlab => "gitlab",
        CredentialIssuer::GoogleAdc => "google_adc",
        CredentialIssuer::SapAiCore => "sap_ai_core",
    }
}
