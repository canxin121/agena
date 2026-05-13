use std::{collections::HashMap, path::PathBuf, sync::Arc};

use aws_credential_types::Credentials;
use tokio::sync::Mutex;

use crate::{
    model::ModelId,
    plugin::{PluginHost, PluginHostBuilder},
    provider::{
        AmazonBedrockProvider, AnthropicProvider, AuthRefreshStrategy, AuthSecretSelector,
        CloudflareAiGatewayProvider, CopilotProvider,
        CopilotProviderOptions as RuntimeCopilotProviderOptions, GeminiProvider, GitlabProvider,
        GitlabProviderConfig, GoogleVertexProvider, ManagedCredential, ModelProvider,
        MultiAdapterProvider, OllamaProvider, OpenAiCompatibleProvider, OpenAiProvider,
        OpencodeProvider, ProviderModelRoute, ProviderRegistry, auth::AuthData,
        parse_sap_ai_core_service_key,
    },
};

use super::{
    CloudflareAiGatewayProviderOptions, ConfigEnvironment, ConfigError, HttpProviderAdapterConfig,
    ProcessEnvironment, ProviderAdapterDefinition, ProviderAuthConfig, ProviderSecretAuthConfig,
    ResolvedConfig, ResolvedProviderAdapterConfig, ResolvedProviderConfig,
};

impl ResolvedConfig {
    pub fn build_provider_registry(&self) -> Result<ProviderRegistry, ConfigError> {
        self.build_provider_registry_with_env(&ProcessEnvironment)
    }

    pub fn build_provider_registry_with_env(
        &self,
        env: &dyn ConfigEnvironment,
    ) -> Result<ProviderRegistry, ConfigError> {
        let client = ProviderRegistry::build_http_client(self.provider_http_client_config())?;
        let mut registry = ProviderRegistry::with_runtime_config(self.provider_runtime_config());

        for (provider_id, resolved) in &self.providers {
            if !resolved.enabled {
                continue;
            }

            let provider = build_provider(provider_id.as_str(), resolved, client.clone(), env)?;
            registry.register_arc(provider);
        }

        Ok(registry)
    }
}

impl super::ConfigResolution {
    pub async fn build_provider_registry_with_plugins(
        &self,
        plugins: &PluginHost,
    ) -> Result<ProviderRegistry, ConfigError> {
        let mut registry = self.config.build_provider_registry()?;
        if plugins.is_empty() {
            return Ok(registry);
        }

        let current = registry
            .provider_ids()
            .into_iter()
            .map(|id| crate::plugin::ProviderDescriptor {
                display_name: id.clone(),
                id,
                models: Vec::new(),
                endpoint: None,
                kind: crate::plugin::ProviderKind::Custom,
            })
            .collect();
        let patch = plugins
            .dispatch_provider_list(crate::plugin::ProviderListInput { current })
            .await
            .map_err(|err| ConfigError::Validation(format!("plugin provider.list: {err}")))?;
        for provider_id in patch.remove {
            registry.remove(provider_id.as_str());
        }
        for descriptor in patch.add {
            registry.register_plugin_provider(descriptor)?;
        }
        Ok(registry)
    }

    pub async fn build_plugin_host(&self) -> Result<Arc<PluginHost>, ConfigError> {
        self.build_plugin_host_with_previous_and_mcp(None, None, None)
            .await
    }

    /// Hot-reload-aware build: when a previous plugin host (and its config)
    /// is available, transports for byte-identical entries are reused, so
    /// stdio subprocesses and HTTP plugins survive a config reload that
    /// didn't touch them.
    pub async fn build_plugin_host_with_previous(
        &self,
        previous_host: Option<Arc<PluginHost>>,
        previous_config: Option<&agena_plugin_host::PluginsConfig>,
    ) -> Result<Arc<PluginHost>, ConfigError> {
        self.build_plugin_host_with_previous_and_mcp(previous_host, previous_config, None)
            .await
    }

    pub async fn build_plugin_host_with_previous_and_mcp(
        &self,
        previous_host: Option<Arc<PluginHost>>,
        previous_config: Option<&agena_plugin_host::PluginsConfig>,
        mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    ) -> Result<Arc<PluginHost>, ConfigError> {
        let workspace_root = self
            .meta
            .config_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let agena_version = env!("CARGO_PKG_VERSION").to_string();
        let mut plugin_config = self.config.plugins.clone();
        if mcp_manager.is_some() {
            plugin_config
                .list
                .entry(crate::tool::mcp_plugin_id().to_string())
                .or_insert_with(|| crate::plugin::PluginEntry::Static {
                    options: serde_json::to_value(&self.config.mcp)
                        .unwrap_or(serde_json::Value::Null),
                    timeouts: Default::default(),
                });
        }
        let mut builder = PluginHostBuilder::new(workspace_root, agena_version)
            .with_config(plugin_config)
            .register_static(crate::tool::lsp_plugin_id(), crate::tool::new_lsp_plugin())
            .register_static(
                crate::tool::cron_plugin_id(),
                crate::tool::new_cron_plugin(),
            )
            .register_static(crate::tool::fs_plugin_id(), crate::tool::new_fs_plugin())
            .register_static(
                crate::tool::shell_plugin_id(),
                crate::tool::new_shell_plugin(),
            )
            .register_static(crate::tool::web_plugin_id(), crate::tool::new_web_plugin())
            .register_static(
                crate::tool::workflow_plugin_id(),
                crate::tool::new_workflow_plugin(),
            )
            .register_static(
                crate::memory::memory_plugin_id(),
                crate::memory::new_memory_plugin(self.config.memory.clone()),
            )
            .register_static(
                crate::hooks::ShellHookPlugin::id(),
                crate::hooks::ShellHookPlugin::new(self.config.hooks.clone()),
            );
        if let Some(manager) = mcp_manager {
            builder = builder.register_static(
                crate::tool::mcp_plugin_id(),
                crate::tool::new_mcp_plugin(manager),
            );
        }
        builder = builder.register_static(
            crate::tool::skills_fs_plugin_id(),
            crate::tool::new_skills_fs_plugin(),
        );
        if let (Some(prev_host), Some(prev_cfg)) = (previous_host, previous_config) {
            builder = builder.with_previous(prev_host, prev_cfg);
        }
        builder
            .build()
            .await
            .map_err(|e| ConfigError::Validation(format!("plugin host: {e}")))
    }
}

fn build_provider(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
) -> Result<Arc<dyn ModelProvider>, ConfigError> {
    let adapters = resolved
        .adapters
        .iter()
        .map(|(adapter_id, adapter)| {
            Ok((
                adapter_id.clone(),
                build_adapter_provider(
                    provider_id,
                    adapter_id.as_str(),
                    adapter,
                    &resolved.auth,
                    client.clone(),
                    env,
                )?,
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, ConfigError>>()?;

    let routes = resolved
        .models
        .iter()
        .map(|(model_id, config)| {
            Ok((
                model_id.clone(),
                ProviderModelRoute {
                    adapter_id: config.adapter.clone(),
                    target_model: ModelId::new(config.target_model.clone()),
                    definition: config.definition.clone(),
                },
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, ConfigError>>()?;

    let passthrough_adapter_id = (resolved.adapters.len() == 1)
        .then(|| resolved.adapters.keys().next().cloned())
        .flatten();

    Ok(Arc::new(MultiAdapterProvider::new(
        provider_id,
        resolved.default_model.clone(),
        adapters,
        routes,
        passthrough_adapter_id,
    )))
}

fn build_adapter_provider(
    provider_id: &str,
    adapter_id: &str,
    config: &ResolvedProviderAdapterConfig,
    auth: &ProviderAuthConfig,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
) -> Result<Arc<dyn ModelProvider>, ConfigError> {
    let runtime_provider_id = runtime_adapter_provider_id(provider_id, adapter_id);
    let provider: Arc<dyn ModelProvider> = match &config.definition {
        ProviderAdapterDefinition::Ollama(adapter) => Arc::new(OllamaProvider::new(
            runtime_provider_id.as_str(),
            client,
            adapter.base_url.clone(),
            config.default_model.clone(),
        )),
        ProviderAdapterDefinition::OpenAi(adapter) => {
            let credential = match adapter.options.backend {
                super::OpenAiBackendConfig::Api => secret_auth_managed_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::OpenAiOAuth,
                    env,
                    true,
                )?,
                super::OpenAiBackendConfig::ChatgptCodex => require_provider_auth_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::OpenAiOAuth,
                    env,
                )?,
            };

            let mut provider = OpenAiProvider::new_managed_with_id(
                runtime_provider_id.as_str(),
                client,
                credential.credential,
                adapter.base_url.clone(),
                config.default_model.clone(),
            )
            .with_backend(adapter.options.backend.into())
            .with_extra_headers(to_hash_map(&adapter.extra_headers))
            .with_api_mode(adapter.options.api_mode.into())
            .with_stream_mode(adapter.options.stream_mode.into())
            .with_realtime_ws_url(adapter.options.realtime_ws_url.clone());

            if let Some(auth_data) = credential.auth_data {
                provider = provider.with_auth_data(auth_data);
            }

            Arc::new(provider)
        }
        ProviderAdapterDefinition::OpenAiCompatible(adapter) => {
            let credential = secret_auth_managed_credential(
                provider_id,
                "api_key",
                auth,
                AuthSecretSelector::AccessOrApiKey,
                AuthRefreshStrategy::ReloadFromStore,
                env,
                true,
            )?
            .credential;
            let extra_headers = to_hash_map(&adapter.extra_headers);
            if matches!(provider_id, "opencode" | "opencode-go")
                || matches!(adapter_id, "opencode" | "opencode-go")
            {
                Arc::new(OpencodeProvider::new(
                    runtime_provider_id.as_str(),
                    client,
                    credential,
                    adapter.base_url.clone(),
                    config.default_model.clone(),
                    adapter.options.auth_header.clone(),
                    adapter.options.auth_scheme.clone(),
                    extra_headers,
                    adapter.options.stream_mode.into(),
                    adapter.options.realtime_ws_url.clone(),
                ))
            } else {
                Arc::new(
                    OpenAiCompatibleProvider::new_managed(
                        runtime_provider_id.as_str(),
                        client,
                        credential,
                        adapter.base_url.clone(),
                        config.default_model.clone(),
                    )
                    .with_auth_header(
                        adapter.options.auth_header.clone(),
                        adapter.options.auth_scheme.clone(),
                    )
                    .with_extra_headers(extra_headers)
                    .with_stream_mode(adapter.options.stream_mode.into())
                    .with_realtime_ws_url(adapter.options.realtime_ws_url.clone()),
                )
            }
        }
        ProviderAdapterDefinition::SapAiCore(adapter) => Arc::new(build_sap_ai_core_provider(
            provider_id,
            runtime_provider_id.as_str(),
            client,
            config,
            adapter,
            auth,
            env,
        )?),
        ProviderAdapterDefinition::Anthropic(adapter) => Arc::new(
            AnthropicProvider::new_managed(
                client,
                secret_auth_managed_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::ReloadFromStore,
                    env,
                    true,
                )?
                .credential,
                adapter.base_url.clone(),
                config.default_model.clone(),
            )
            .with_auth_header(
                adapter.options.auth_header.clone(),
                adapter.options.auth_scheme.clone(),
            )
            .with_extra_headers(to_hash_map(&adapter.extra_headers)),
        ),
        ProviderAdapterDefinition::Gemini(adapter) => Arc::new(
            GeminiProvider::new_managed(
                client,
                secret_auth_managed_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::ReloadFromStore,
                    env,
                    true,
                )?
                .credential,
                adapter.base_url.clone(),
                config.default_model.clone(),
            )
            .with_extra_headers(to_hash_map(&adapter.extra_headers)),
        ),
        ProviderAdapterDefinition::Gitlab(adapter) => {
            let runtime_config = GitlabProviderConfig {
                instance_url: adapter.instance_url.clone(),
                ai_gateway_url: adapter.ai_gateway_url.clone(),
                default_model: config.default_model.clone(),
                ai_gateway_headers: to_hash_map(&adapter.ai_gateway_headers),
                feature_flags: to_hash_map(&adapter.feature_flags),
            };
            let secret = secret_auth(auth, provider_id)?;
            let credential = if secret_has_direct_source(secret, env) {
                required_secret_auth_credential(provider_id, "api_key", secret, env)?
            } else {
                require_provider_auth_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::GitlabOAuth {
                        instance_url: adapter.instance_url.clone(),
                    },
                    env,
                )?
                .credential
            };
            Arc::new(GitlabProvider::from_managed_token_with_config(
                client,
                credential,
                runtime_config,
            )?)
        }
        ProviderAdapterDefinition::Copilot(adapter) => {
            let credential = require_provider_auth_credential(
                provider_id,
                "bearer_token",
                auth,
                AuthSecretSelector::RefreshOrAccess,
                AuthRefreshStrategy::ReloadFromStore,
                env,
            )?;
            let enterprise_url = credential
                .auth_data
                .as_ref()
                .and_then(current_enterprise_url);

            let mut provider = CopilotProvider::with_bearer_credential(
                runtime_provider_id.as_str(),
                client,
                credential.credential,
                enterprise_url,
                RuntimeCopilotProviderOptions {
                    base_url: copilot_base_url(credential.auth_data.as_ref(), adapter),
                    default_model: Some(ModelId::new(config.default_model.clone())),
                    models_url: adapter.models_url.clone(),
                },
            )?;
            if let Some(auth_data) = credential.auth_data {
                provider = provider.with_auth_data(auth_data);
            }
            Arc::new(provider)
        }
        ProviderAdapterDefinition::AmazonBedrock(adapter) => Arc::new(match auth {
            ProviderAuthConfig::Secret(_) => AmazonBedrockProvider::new_managed_bearer(
                client,
                secret_auth_managed_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::ReloadFromStore,
                    env,
                    true,
                )?
                .credential,
                adapter.base_url.clone(),
                config.default_model.clone(),
                adapter.region.clone(),
            ),
            ProviderAuthConfig::BedrockSigv4(sigv4) => AmazonBedrockProvider::new_sigv4(
                client,
                adapter.base_url.clone(),
                config.default_model.clone(),
                adapter.region.clone(),
                sigv4.profile.clone(),
                static_bedrock_credentials(
                    sigv4.access_key_id.clone(),
                    sigv4.secret_access_key.clone(),
                    sigv4.session_token.clone(),
                    provider_id,
                )?,
            ),
            _ => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "amazon_bedrock adapter requires secret or bedrock_sigv4 auth"
                        .to_owned(),
                });
            }
        }),
        ProviderAdapterDefinition::GoogleVertex(adapter) => Arc::new(match auth {
            ProviderAuthConfig::Secret(_) => GoogleVertexProvider::new_managed_token(
                runtime_provider_id.as_str(),
                client,
                adapter.base_url.clone(),
                config.default_model.clone(),
                secret_auth_managed_credential(
                    provider_id,
                    "access_token",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::ReloadFromStore,
                    env,
                    true,
                )?
                .credential,
            ),
            ProviderAuthConfig::GoogleAdc => GoogleVertexProvider::new_adc(
                runtime_provider_id.as_str(),
                client,
                adapter.base_url.clone(),
                config.default_model.clone(),
            ),
            _ => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "google_vertex adapter requires secret or google_adc auth".to_owned(),
                });
            }
        }),
        ProviderAdapterDefinition::CloudflareAiGateway(adapter) => {
            Arc::new(build_cloudflare_provider(
                provider_id,
                runtime_provider_id.as_str(),
                client,
                config,
                adapter,
                auth,
                env,
            )?)
        }
    };

    Ok(provider)
}

fn build_cloudflare_provider(
    provider_id: &str,
    runtime_provider_id: &str,
    client: reqwest::Client,
    config: &ResolvedProviderAdapterConfig,
    adapter: &CloudflareAiGatewayProviderOptions,
    auth: &ProviderAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<CloudflareAiGatewayProvider, ConfigError> {
    let inner = OpenAiCompatibleProvider::new_managed(
        runtime_provider_id,
        client,
        secret_auth_managed_credential(
            provider_id,
            "api_key",
            auth,
            AuthSecretSelector::AccessOrApiKey,
            AuthRefreshStrategy::ReloadFromStore,
            env,
            true,
        )?
        .credential,
        adapter.base_url.clone(),
        config.default_model.clone(),
    );
    Ok(CloudflareAiGatewayProvider::new(inner))
}

fn build_sap_ai_core_provider(
    provider_id: &str,
    runtime_provider_id: &str,
    client: reqwest::Client,
    config: &ResolvedProviderAdapterConfig,
    adapter: &HttpProviderAdapterConfig<super::OpenAiCompatibleProviderOptions>,
    auth: &ProviderAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<OpenAiCompatibleProvider, ConfigError> {
    let credential = match sap_ai_core_managed_credential(provider_id, client.clone(), auth, env) {
        Ok(credential) => credential,
        Err(err) => return Err(err),
    };

    Ok(OpenAiCompatibleProvider::new_managed(
        runtime_provider_id,
        client,
        credential,
        adapter.base_url.clone(),
        config.default_model.clone(),
    )
    .with_auth_header(
        adapter.options.auth_header.clone(),
        adapter.options.auth_scheme.clone(),
    )
    .with_extra_headers(to_hash_map(&adapter.extra_headers))
    .with_stream_mode(adapter.options.stream_mode.into())
    .with_realtime_ws_url(adapter.options.realtime_ws_url.clone()))
}

fn secret_auth<'a>(
    auth: &'a ProviderAuthConfig,
    provider_id: &str,
) -> Result<&'a ProviderSecretAuthConfig, ConfigError> {
    match auth {
        ProviderAuthConfig::Secret(secret) => Ok(secret),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "adapter requires secret-based auth".to_owned(),
        }),
    }
}

struct ResolvedManagedCredential {
    credential: ManagedCredential,
    auth_data: Option<Arc<Mutex<AuthData>>>,
}

fn secret_has_direct_source(
    secret: &ProviderSecretAuthConfig,
    env: &dyn ConfigEnvironment,
) -> bool {
    match (
        secret
            .secret
            .as_ref()
            .and_then(|value| normalize_text(value.as_str())),
        secret
            .secret_env
            .as_ref()
            .and_then(|value| normalize_text(value.as_str())),
    ) {
        (Some(_), _) => true,
        (None, Some(env_key)) => env
            .var(env_key.as_str())
            .and_then(|value| normalize_text(&value))
            .is_some(),
        (None, None) => false,
    }
}

fn required_secret_auth_credential(
    provider_id: &str,
    field: &'static str,
    secret: &ProviderSecretAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<ManagedCredential, ConfigError> {
    if let Some(value) = secret
        .secret
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()))
    {
        return Ok(ManagedCredential::static_value(
            format!("{provider_id} {field}"),
            value,
        ));
    }

    let Some(env_key) = secret
        .secret_env
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()))
    else {
        return Err(ConfigError::MissingProviderField {
            provider_id: provider_id.to_owned(),
            field,
        });
    };

    if env
        .var(env_key.as_str())
        .and_then(|value| normalize_text(&value))
        .is_none()
    {
        return Err(ConfigError::MissingEnvironmentVariable {
            provider_id: provider_id.to_owned(),
            field,
            env_key,
        });
    }

    Ok(ManagedCredential::environment(
        format!("{provider_id} {field}"),
        provider_id.to_owned(),
        field,
        env_key,
    ))
}

fn secret_auth_managed_credential(
    provider_id: &str,
    field: &'static str,
    auth: &ProviderAuthConfig,
    selector: AuthSecretSelector,
    refresh: AuthRefreshStrategy,
    env: &dyn ConfigEnvironment,
    allow_deferred_env: bool,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let secret = secret_auth(auth, provider_id)?;
    if let Some(value) = secret
        .secret
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()))
    {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::static_value(format!("{provider_id} {field}"), value),
            auth_data: None,
        });
    }

    let env_key = secret
        .secret_env
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()));

    if let Some(env_key) = env_key.as_ref() {
        if env
            .var(env_key.as_str())
            .and_then(|value| normalize_text(&value))
            .is_some()
        {
            return Ok(ResolvedManagedCredential {
                credential: ManagedCredential::environment(
                    format!("{provider_id} {field}"),
                    provider_id.to_owned(),
                    field,
                    env_key.clone(),
                ),
                auth_data: None,
            });
        }
    }

    if let Some(auth_data) = secret.credential.clone() {
        if !auth_supports_selector(&auth_data, selector) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "configured inline credential does not satisfy `{field}` requirements"
                ),
            });
        }

        let auth_data = Arc::new(Mutex::new(auth_data));
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::auth_data_shared(
                format!("{provider_id} {field}"),
                provider_id.to_owned(),
                auth_data.clone(),
                selector,
                refresh,
            ),
            auth_data: Some(auth_data),
        });
    }

    if allow_deferred_env && let Some(env_key) = env_key {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::environment(
                format!("{provider_id} {field}"),
                provider_id.to_owned(),
                field,
                env_key,
            ),
            auth_data: None,
        });
    }

    Err(ConfigError::MissingProviderField {
        provider_id: provider_id.to_owned(),
        field,
    })
}

fn require_provider_auth_credential(
    provider_id: &str,
    field: &'static str,
    auth: &ProviderAuthConfig,
    selector: AuthSecretSelector,
    refresh: AuthRefreshStrategy,
    env: &dyn ConfigEnvironment,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let secret = secret_auth(auth, provider_id)?;
    if secret_has_direct_source(secret, env) {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("{field} must come from provider credential auth"),
        });
    }

    if let Some(auth_data) = secret.credential.clone() {
        if !auth_supports_selector(&auth_data, selector) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "configured inline credential does not satisfy `{field}` requirements"
                ),
            });
        }

        let auth_data = Arc::new(Mutex::new(auth_data));
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::auth_data_shared(
                format!("{provider_id} {field}"),
                provider_id.to_owned(),
                auth_data.clone(),
                selector,
                refresh,
            ),
            auth_data: Some(auth_data),
        });
    }

    Err(ConfigError::MissingProviderField {
        provider_id: provider_id.to_owned(),
        field,
    })
}

fn sap_ai_core_managed_credential(
    provider_id: &str,
    client: reqwest::Client,
    auth: &ProviderAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<ManagedCredential, ConfigError> {
    match auth {
        ProviderAuthConfig::SapAiCore(config) => {
            let secret_auth = ProviderAuthConfig::Secret(config.secret.clone());
            match secret_auth_managed_credential(
                provider_id,
                "api_key",
                &secret_auth,
                AuthSecretSelector::AccessOrApiKey,
                AuthRefreshStrategy::ReloadFromStore,
                env,
                false,
            ) {
                Ok(credential) => Ok(credential.credential),
                Err(ConfigError::MissingProviderField { .. }) => {
                    let service_key_raw = env
                        .var(config.service_key_env.as_str())
                        .and_then(|value| normalize_text(&value))
                        .ok_or_else(|| ConfigError::InvalidProviderConfig {
                            provider_id: provider_id.to_owned(),
                            message: format!(
                                "sap-ai-core requires `{}` when secret auth is not configured",
                                config.service_key_env
                            ),
                        })?;
                    let service_key = parse_sap_ai_core_service_key(service_key_raw.as_str())
                        .map_err(|err| ConfigError::InvalidProviderConfig {
                            provider_id: provider_id.to_owned(),
                            message: format!("failed to parse `{}`: {err}", config.service_key_env),
                        })?;
                    Ok(ManagedCredential::sap_ai_core(
                        format!("{provider_id} sap ai core token"),
                        client.clone(),
                        provider_id.to_owned(),
                        service_key,
                    ))
                }
                Err(err) => Err(err),
            }
        }
        ProviderAuthConfig::Secret(_) => secret_auth_managed_credential(
            provider_id,
            "api_key",
            auth,
            AuthSecretSelector::AccessOrApiKey,
            AuthRefreshStrategy::ReloadFromStore,
            env,
            true,
        )
        .map(|credential| credential.credential),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "sap_ai_core adapter requires secret or sap_ai_core auth".to_owned(),
        }),
    }
}

fn static_bedrock_credentials(
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    provider_id: &str,
) -> Result<Option<Credentials>, ConfigError> {
    match (
        access_key_id.and_then(|value| normalize_text(&value)),
        secret_access_key.and_then(|value| normalize_text(&value)),
    ) {
        (Some(access_key_id), Some(secret_access_key)) => Ok(Some(Credentials::new(
            access_key_id,
            secret_access_key,
            session_token.and_then(|value| normalize_text(&value)),
            None,
            "agena-config",
        ))),
        (None, None) => Ok(None),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "access_key_id and secret_access_key must be provided together".to_owned(),
        }),
    }
}

fn runtime_adapter_provider_id(provider_id: &str, adapter_id: &str) -> String {
    if adapter_id == "default" {
        provider_id.to_owned()
    } else {
        format!("{provider_id}::{adapter_id}")
    }
}

fn copilot_base_url(
    auth_data: Option<&Arc<Mutex<AuthData>>>,
    config: &super::CopilotProviderOptions,
) -> Option<String> {
    if config.base_url == "https://api.githubcopilot.com"
        && auth_data.and_then(current_enterprise_url).is_some()
    {
        None
    } else {
        Some(config.base_url.clone())
    }
}

fn current_enterprise_url(auth_data: &Arc<Mutex<AuthData>>) -> Option<String> {
    auth_data
        .try_lock()
        .ok()
        .as_deref()
        .and_then(AuthData::enterprise_url)
        .map(ToOwned::to_owned)
}

fn normalize_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn auth_supports_selector(auth: &AuthData, selector: AuthSecretSelector) -> bool {
    match selector {
        AuthSecretSelector::AccessOrApiKey => match auth {
            AuthData::Api { key } | AuthData::WellKnown { key, .. } => !key.trim().is_empty(),
            AuthData::OAuth { access, .. } => !access.trim().is_empty(),
        },
        AuthSecretSelector::RefreshOrAccess => match auth {
            AuthData::Api { key } | AuthData::WellKnown { key, .. } => !key.trim().is_empty(),
            AuthData::OAuth {
                refresh, access, ..
            } => !refresh.trim().is_empty() || !access.trim().is_empty(),
        },
    }
}

fn to_hash_map<K, V>(map: &std::collections::BTreeMap<K, V>) -> HashMap<K, V>
where
    K: Clone + Eq + std::hash::Hash,
    V: Clone,
{
    map.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::config::{ConfigEnvironment, ConfigError, ConfigLoader, LoadConfigRequest};
    use crate::provider::CapabilitySupport;

    #[derive(Clone, Default)]
    struct TestEnvironment {
        vars: BTreeMap<String, String>,
    }

    impl ConfigEnvironment for TestEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }

        fn vars(&self) -> Vec<(String, String)> {
            self.vars
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        }
    }

    #[test]
    fn registry_builder_resolves_env_secret_for_concrete_provider() {
        let path = write_temp_file(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"
"#,
        );

        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");
        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "openai"));
    }

    #[test]
    fn registry_builder_applies_configured_models_to_provider_models() {
        let path = write_temp_file(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"

[providers.openai.models."gpt-4.1-mini"]
input = { unsupported = ["image"] }
"#,
        );

        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");
        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build");

        let capabilities = registry
            .model_capabilities(&crate::model::ModelRef::new("openai", "gpt-4.1-mini"))
            .expect("provider capabilities should resolve");
        assert_eq!(capabilities.image_input, CapabilitySupport::Unsupported);
        assert_eq!(capabilities.document_input, CapabilitySupport::Supported);
    }

    #[test]
    fn registry_builder_accepts_inline_api_key_for_openai_without_config_secret() {
        let path = write_temp_file(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[providers.openai.auth]
credential = { type = "api", key = "sk-from-config" }
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build from inline api key");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "openai"));
    }

    #[test]
    fn registry_builder_accepts_inline_oauth_for_openai_without_config_secret() {
        let path = write_temp_file(
            r#"
[providers.openai]
default_model = "gpt-5.3-codex"

[providers.openai.auth]
credential = { type = "oauth", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.openai.adapters.codex]
kind = "openai"
backend = "chatgpt_codex"
default_model = "gpt-5.3-codex"
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build from inline oauth");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "openai"));
    }

    #[test]
    fn registry_builder_prefers_inline_credential_when_api_key_env_is_configured_but_missing() {
        let path = write_temp_file(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"

[providers.openai.auth]
credential = { type = "api", key = "sk-from-config" }
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build from inline credential fallback");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "openai"));
    }

    #[test]
    fn registry_builder_prefers_inline_gitlab_oauth_when_api_key_env_is_configured_but_missing() {
        let path = write_temp_file(
            r#"
[providers.gitlab]
default_model = "claude-sonnet-4-5"

[providers.gitlab.auth]
secret_env = "GITLAB_TOKEN"
credential = { type = "oauth", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000 }

[providers.gitlab.adapters.duo]
kind = "gitlab"
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build from inline gitlab oauth fallback");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "gitlab"));
    }

    #[test]
    fn registry_builder_requires_chatgpt_codex_auth() {
        let path = write_temp_file(
            r#"
[providers.openai_chatgpt]
kind = "openai"
backend = "chatgpt_codex"
default_model = "gpt-5.3-codex"
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let err = match resolution.config.build_provider_registry_with_env(&env) {
            Ok(_) => panic!("registry should require chatgpt codex auth"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            ConfigError::MissingProviderField { provider_id, field }
                if provider_id == "openai_chatgpt" && field == "api_key"
        ));
    }

    #[test]
    fn registry_builder_registers_chatgpt_codex_backend_with_inline_oauth() {
        let path = write_temp_file(
            r#"
[providers.openai_chatgpt]
default_model = "gpt-5.3-codex"

[providers.openai_chatgpt.auth]
credential = { type = "oauth", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.openai_chatgpt.adapters.codex]
kind = "openai"
backend = "chatgpt_codex"
default_model = "gpt-5.3-codex"
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build with inline chatgpt codex oauth");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "openai_chatgpt"));
    }

    #[test]
    fn registry_builder_requires_github_copilot_auth() {
        let path = write_temp_file(
            r#"
[providers."github-copilot"]
kind = "copilot"
default_model = "gpt-4o-mini"
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let err = match resolution.config.build_provider_registry_with_env(&env) {
            Ok(_) => panic!("registry should require copilot auth"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            ConfigError::MissingProviderField { provider_id, field }
                if provider_id == "github-copilot" && field == "bearer_token"
        ));
    }

    #[test]
    fn registry_builder_requires_gitlab_auth_when_no_direct_secret_exists() {
        let path = write_temp_file(
            r#"
[providers.gitlab]
kind = "gitlab"
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let err = match resolution.config.build_provider_registry_with_env(&env) {
            Ok(_) => panic!("registry should require gitlab auth"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            ConfigError::MissingProviderField { provider_id, field }
                if provider_id == "gitlab" && field == "api_key"
        ));
    }

    #[test]
    fn registry_builder_registers_google_vertex_with_missing_env_until_request_time() {
        let path = write_temp_file(
            r#"
[providers."google-vertex"]
kind = "google_vertex"
base_url = "https://us-central1-aiplatform.googleapis.com/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
default_model = "google/gemini-2.5-flash"
access_token_env = "GOOGLE_VERTEX_ACCESS_TOKEN"
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                overrides: Vec::new(),
            })
            .expect("config should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build without google vertex env");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "google-vertex"));
    }

    #[test]
    fn registry_builder_accepts_config_full_without_credentials_for_first_run_login() {
        let path = write_temp_file(include_str!("../../../../config.full.toml"));

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                overrides: Vec::new(),
            })
            .expect("config.full.toml should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("config.full.toml should build provider registry without credentials");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "openai"));
        assert!(ids.iter().any(|id| id == "openai_chatgpt"));
        assert!(ids.iter().any(|id| id == "github-copilot"));
        assert!(ids.iter().any(|id| id == "gitlab"));
        assert!(ids.iter().any(|id| id == "google-vertex"));
    }

    #[test]
    fn registry_builder_accepts_inline_api_key_for_sap_ai_core_without_config_secret() {
        let path = write_temp_file(
            r#"
[providers.sap]
kind = "sap_ai_core"
base_url = "https://api.example.com/v2"
default_model = "anthropic/claude-sonnet-4"
auth_header = "authorization"
auth_scheme = "Bearer"

[providers.sap.auth]
credential = { type = "api", key = "sap-api-token" }
"#,
        );

        let env = TestEnvironment::default();
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");

        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build from inline sap api key");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "sap"));
    }

    fn write_temp_file(content: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agena-config-registry-{suffix}.toml"));
        fs::write(&path, content).expect("temp file should be written");
        path
    }
}
