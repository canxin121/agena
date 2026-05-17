use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
};

use aws_credential_types::Credentials;
use tokio::sync::Mutex;

use crate::{
    model::{AdapterId, ProviderId},
    model_catalog::ModelCatalogSnapshot,
    plugin::{PluginHost, PluginHostBuilder},
    provider::{
        AmazonBedrockProvider, AnthropicProfile, AnthropicProvider, AuthRefreshStrategy,
        AuthSecretSelector, CatalogedModelsProvider, GeminiProvider, GitlabProvider,
        GitlabProviderConfig, ManagedCredential, ModelProvider, MultiAdapterProvider,
        OllamaProvider, OpenAiProvider, ProviderModelRoute, ProviderRegistry, auth::AuthData,
        parse_sap_ai_core_service_key,
    },
};

use super::raw::parse_adapter_model_ref;
use super::{
    ConfigEnvironment, ConfigError, ProcessEnvironment, ProviderAdapterDefinition,
    ProviderApiAuthConfig, ProviderAuthConfig, ProviderCapabilityFamilyConfig, ResolvedConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, SharedGatewayEndpointLayout,
};

const ATOMGIT_LLM_BASE_URL: &str = "https://api-ai.gitcode.com/v1";
const PROBE_DEFAULT_MODEL_ID: &str = "__probe__";

#[derive(Debug, Clone)]
pub struct ProviderAdapterProbeResult {
    pub adapter_id: String,
    pub enabled: bool,
    pub supported: bool,
    pub resolved_base_url: Option<String>,
    pub models: Vec<crate::provider::ProviderModel>,
    pub error: Option<String>,
}

impl ResolvedConfig {
    pub fn build_provider_registry(&self) -> Result<ProviderRegistry, ConfigError> {
        self.build_provider_registry_with_env(&ProcessEnvironment)
    }

    pub fn build_provider_registry_with_catalog(
        &self,
        catalog: Option<&ModelCatalogSnapshot>,
    ) -> Result<ProviderRegistry, ConfigError> {
        self.build_provider_registry_with_catalog_and_env(catalog, &ProcessEnvironment)
    }

    pub fn build_provider_registry_with_env(
        &self,
        env: &dyn ConfigEnvironment,
    ) -> Result<ProviderRegistry, ConfigError> {
        self.build_provider_registry_with_catalog_and_env(None, env)
    }

    pub fn build_provider_registry_with_catalog_and_env(
        &self,
        catalog: Option<&ModelCatalogSnapshot>,
        env: &dyn ConfigEnvironment,
    ) -> Result<ProviderRegistry, ConfigError> {
        let client = ProviderRegistry::build_http_client(self.provider_http_client_config())?;
        let mut registry = ProviderRegistry::with_runtime_config(self.provider_runtime_config());

        for (provider_id, resolved) in &self.providers {
            if !resolved.enabled {
                continue;
            }

            let provider =
                build_provider(provider_id.as_str(), resolved, client.clone(), env, catalog)?;
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
        self.build_provider_registry_with_plugins_and_catalog(plugins, None)
            .await
    }

    pub async fn build_provider_registry_with_plugins_and_catalog(
        &self,
        plugins: &PluginHost,
        catalog: Option<&ModelCatalogSnapshot>,
    ) -> Result<ProviderRegistry, ConfigError> {
        let mut registry = self.config.build_provider_registry_with_catalog(catalog)?;
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
                crate::tool::settings_plugin_id(),
                crate::tool::new_settings_plugin(),
            )
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
            crate::tool::skills_plugin_id(),
            crate::tool::new_skills_plugin(),
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
    catalog: Option<&ModelCatalogSnapshot>,
) -> Result<Arc<dyn ModelProvider>, ConfigError> {
    let adapter_defaults = resolve_adapter_default_models(provider_id, resolved)?;
    let adapters = resolved
        .adapters
        .iter()
        .filter(|(_, adapter)| adapter.enabled)
        .map(|(adapter_id, adapter)| {
            Ok((
                adapter_id.clone(),
                build_adapter_provider(
                    provider_id,
                    adapter_id.as_str(),
                    adapter,
                    adapter_defaults
                        .get(adapter_id.as_str())
                        .expect("adapter default should exist")
                        .as_str(),
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
        .filter(|(model_id, _)| {
            parse_adapter_model_ref(provider_id, model_id)
                .ok()
                .and_then(|(adapter_id, _)| resolved.adapters.get(adapter_id.as_str()))
                .map(|adapter| adapter.enabled)
                .unwrap_or(false)
        })
        .map(|(model_id, config)| {
            let (adapter_id, target_model_id) = parse_adapter_model_ref(provider_id, model_id)?;
            Ok((
                (adapter_id, target_model_id),
                ProviderModelRoute {
                    enabled: config.enabled,
                    definition: config.definition.clone(),
                },
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, ConfigError>>()?;

    let provider: Arc<dyn ModelProvider> = Arc::new(MultiAdapterProvider::new(
        provider_id,
        resolved.default_adapter.clone(),
        resolved.default_model.clone(),
        adapters,
        routes,
    ));

    if let Some(provider_record) = catalog.map(|snapshot| snapshot.merged_models()) {
        Ok(CatalogedModelsProvider::new(provider, provider_record))
    } else {
        Ok(provider)
    }
}

fn build_adapter_provider(
    provider_id: &str,
    adapter_id: &str,
    config: &ResolvedProviderAdapterConfig,
    adapter_default_model: &str,
    auth: &ProviderAuthConfig,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
) -> Result<Arc<dyn ModelProvider>, ConfigError> {
    let runtime_provider_id = runtime_adapter_provider_id(provider_id, adapter_id);
    let provider: Arc<dyn ModelProvider> = match &config.definition {
        ProviderAdapterDefinition::Ollama(adapter) => Arc::new(OllamaProvider::new(
            runtime_provider_id.as_str(),
            client,
            adapter
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_owned()),
            adapter_default_model.to_owned(),
        )),
        ProviderAdapterDefinition::OpenAi(adapter) => match auth {
            ProviderAuthConfig::Api(_)
            | ProviderAuthConfig::GoogleAdc(_)
            | ProviderAuthConfig::SapAiCore(_) => {
                let credential = openai_adapter_api_credential(
                    provider_id,
                    auth,
                    client.clone(),
                    adapter.options.capability_family,
                    env,
                )?;
                let mut provider = OpenAiProvider::new_managed_with_id(
                    runtime_provider_id.as_str(),
                    client,
                    credential.credential,
                    resolve_http_adapter_base_url(provider_id, auth, HttpAdapterKind::OpenAi)?,
                    adapter_default_model.to_owned(),
                )
                .with_backend(adapter.options.backend.into())
                .with_auth_header(
                    adapter.options.auth_header.clone(),
                    adapter.options.auth_scheme.clone(),
                )
                .with_extra_headers(to_hash_map(&adapter.extra_headers))
                .with_api_mode(adapter.options.api_mode.into())
                .with_stream_mode(adapter.options.stream_mode.into())
                .with_realtime_ws_url(adapter.options.realtime_ws_url.clone())
                .with_models_url(adapter.options.models_url.clone());

                if let Some(family) = adapter.options.capability_family {
                    provider = provider.with_capability_family(family.into());
                }

                if let Some(auth_data) = credential.auth_data {
                    provider = provider.with_auth_data(auth_data);
                }

                Arc::new(provider)
            }
            ProviderAuthConfig::Credential(credential_auth) => match credential_auth.issuer {
                crate::provider::auth::CredentialIssuer::OpenaiChatgpt => {
                    let credential = require_provider_auth_credential(
                        provider_id,
                        "api_key",
                        auth,
                        AuthSecretSelector::AccessOrApiKey,
                        AuthRefreshStrategy::OpenAiOAuth,
                        env,
                    )?;
                    let mut provider = OpenAiProvider::new_managed_with_id(
                        runtime_provider_id.as_str(),
                        client,
                        credential.credential,
                        "https://chatgpt.com/backend-api/codex".to_owned(),
                        adapter_default_model.to_owned(),
                    )
                    .with_backend(adapter.options.backend.into())
                    .with_auth_header(
                        adapter.options.auth_header.clone(),
                        adapter.options.auth_scheme.clone(),
                    )
                    .with_extra_headers(to_hash_map(&adapter.extra_headers))
                    .with_api_mode(adapter.options.api_mode.into())
                    .with_stream_mode(adapter.options.stream_mode.into())
                    .with_realtime_ws_url(adapter.options.realtime_ws_url.clone());

                    if let Some(auth_data) = credential.auth_data {
                        provider = provider.with_auth_data(auth_data);
                    }

                    Arc::new(provider)
                }
                crate::provider::auth::CredentialIssuer::GithubCopilot => {
                    let credential = require_provider_auth_credential(
                        provider_id,
                        "bearer_token",
                        auth,
                        AuthSecretSelector::RefreshOrAccess,
                        AuthRefreshStrategy::ReloadFromStore,
                        env,
                    )?;
                    let mut provider = OpenAiProvider::new_managed_with_id(
                        runtime_provider_id.as_str(),
                        client,
                        credential.credential,
                        "https://api.githubcopilot.com".to_owned(),
                        adapter_default_model.to_owned(),
                    )
                    .with_profile(crate::provider::OpenAiProfile::GithubCopilot)
                    .with_backend(adapter.options.backend.into())
                    .with_auth_header(
                        adapter.options.auth_header.clone(),
                        adapter.options.auth_scheme.clone(),
                    )
                    .with_api_mode(adapter.options.api_mode.into())
                    .with_api_mode_explicit(adapter.options.api_mode_explicit)
                    .with_stream_mode(adapter.options.stream_mode.into())
                    .with_models_url(adapter.options.models_url.clone())
                    .with_realtime_ws_url(adapter.options.realtime_ws_url.clone())
                    .with_extra_headers(to_hash_map(&adapter.extra_headers));
                    if let Some(auth_data) = credential.auth_data {
                        provider = provider.with_auth_data(auth_data);
                    }
                    Arc::new(provider)
                }
                crate::provider::auth::CredentialIssuer::AtomGit => {
                    let credential = require_provider_auth_credential(
                        provider_id,
                        "api_key",
                        auth,
                        AuthSecretSelector::AccessOrApiKey,
                        AuthRefreshStrategy::AtomGitOAuth,
                        env,
                    )?;
                    let mut provider = OpenAiProvider::new_managed_with_id(
                        runtime_provider_id.as_str(),
                        client,
                        credential.credential,
                        ATOMGIT_LLM_BASE_URL.to_owned(),
                        adapter_default_model.to_owned(),
                    )
                    .with_backend(adapter.options.backend.into())
                    .with_auth_header(
                        adapter.options.auth_header.clone(),
                        adapter.options.auth_scheme.clone(),
                    )
                    .with_extra_headers(to_hash_map(&adapter.extra_headers))
                    .with_api_mode(adapter.options.api_mode.into())
                    .with_api_mode_explicit(adapter.options.api_mode_explicit)
                    .with_stream_mode(adapter.options.stream_mode.into())
                    .with_models_url(adapter.options.models_url.clone())
                    .with_realtime_ws_url(adapter.options.realtime_ws_url.clone());

                    if let Some(family) = adapter.options.capability_family {
                        provider = provider.with_capability_family(family.into());
                    }

                    if let Some(auth_data) = credential.auth_data {
                        provider = provider.with_auth_data(auth_data);
                    }

                    Arc::new(provider)
                }
                _ => {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "credential issuer is not supported by openai adapter".to_owned(),
                    });
                }
            },
            _ => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "openai adapter requires api, google_adc, credential, or sap_ai_core auth"
                            .to_owned(),
                });
            }
        },
        ProviderAdapterDefinition::Anthropic(adapter) => match auth {
            ProviderAuthConfig::Credential(credential_auth)
                if matches!(
                    credential_auth.issuer,
                    crate::provider::auth::CredentialIssuer::GithubCopilot
                ) =>
            {
                let credential = require_provider_auth_credential(
                    provider_id,
                    "bearer_token",
                    auth,
                    AuthSecretSelector::RefreshOrAccess,
                    AuthRefreshStrategy::ReloadFromStore,
                    env,
                )?;
                let base_url = copilot_base_url(credential.auth_data.as_ref(), None)
                    .unwrap_or_else(|| "https://api.githubcopilot.com".to_owned());

                let mut provider = AnthropicProvider::new_managed_with_id(
                    runtime_provider_id.as_str(),
                    client,
                    credential.credential,
                    base_url,
                    adapter_default_model.to_owned(),
                )
                .with_profile(AnthropicProfile::GithubCopilot)
                .with_models_url(adapter.options.models_url.clone())
                .with_messages_url(adapter.options.messages_url.clone())
                .with_auth_header(
                    adapter.options.auth_header.clone(),
                    adapter.options.auth_scheme.clone(),
                )
                .with_beta_header(adapter.options.extra_beta_header.clone())
                .with_extra_headers(to_hash_map(&adapter.extra_headers));

                if let Some(auth_data) = credential.auth_data {
                    provider = provider.with_auth_data(auth_data);
                }

                Arc::new(provider)
            }
            _ => Arc::new(
                AnthropicProvider::new_managed_with_id(
                    runtime_provider_id.as_str(),
                    client,
                    api_auth_managed_credential(
                        provider_id,
                        "api_key",
                        auth,
                        AuthSecretSelector::AccessOrApiKey,
                        AuthRefreshStrategy::ReloadFromStore,
                        env,
                        true,
                    )?
                    .credential,
                    resolve_http_adapter_base_url(provider_id, auth, HttpAdapterKind::Anthropic)?,
                    adapter_default_model.to_owned(),
                )
                .with_models_url(adapter.options.models_url.clone())
                .with_messages_url(adapter.options.messages_url.clone())
                .with_auth_header(
                    adapter.options.auth_header.clone(),
                    adapter.options.auth_scheme.clone(),
                )
                .with_beta_header(adapter.options.extra_beta_header.clone())
                .with_extra_headers(to_hash_map(&adapter.extra_headers)),
            ),
        },
        ProviderAdapterDefinition::Gemini(adapter) => Arc::new({
            let mut provider = GeminiProvider::new_managed(
                client,
                api_auth_managed_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::ReloadFromStore,
                    env,
                    true,
                )?
                .credential,
                resolve_http_adapter_base_url(provider_id, auth, HttpAdapterKind::Gemini)?,
                adapter_default_model.to_owned(),
            )
            .with_extra_headers(to_hash_map(&adapter.extra_headers));
            if let Some(header) = adapter.options.auth_header.clone() {
                provider = provider.with_auth_header(header, adapter.options.auth_scheme.clone());
            }
            provider
        }),
        ProviderAdapterDefinition::Gitlab(adapter) => {
            let runtime_config = GitlabProviderConfig {
                instance_url: adapter
                    .instance_url
                    .clone()
                    .unwrap_or_else(|| "https://gitlab.com".to_owned()),
                ai_gateway_url: adapter
                    .ai_gateway_url
                    .clone()
                    .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned()),
                default_model: adapter_default_model.to_owned(),
                ai_gateway_headers: to_hash_map(&adapter.ai_gateway_headers),
                feature_flags: to_hash_map(&adapter.feature_flags),
            };
            let credential = match auth {
                ProviderAuthConfig::Api(api) => {
                    if api_auth_has_direct_source(api, env) {
                        required_api_auth_credential(provider_id, "api_key", api, env)?
                    } else {
                        return Err(ConfigError::MissingProviderField {
                            provider_id: provider_id.to_owned(),
                            field: "api_key",
                        });
                    }
                }
                ProviderAuthConfig::Credential(_) => {
                    require_provider_auth_credential(
                        provider_id,
                        "api_key",
                        auth,
                        AuthSecretSelector::AccessOrApiKey,
                        AuthRefreshStrategy::GitlabOAuth {
                            instance_url: adapter
                                .instance_url
                                .clone()
                                .unwrap_or_else(|| "https://gitlab.com".to_owned()),
                        },
                        env,
                    )?
                    .credential
                }
                _ => {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "gitlab adapter requires api or credential auth".to_owned(),
                    });
                }
            };
            Arc::new(GitlabProvider::from_managed_token_with_config(
                client,
                credential,
                runtime_config,
            )?)
        }
        ProviderAdapterDefinition::AmazonBedrock(_adapter) => Arc::new(match auth {
            ProviderAuthConfig::BedrockSigv4(sigv4) => AmazonBedrockProvider::new_sigv4(
                client,
                sigv4.base_url.clone(),
                adapter_default_model.to_owned(),
                sigv4.region.clone(),
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
                    message: "amazon_bedrock adapter requires bedrock_sigv4 auth".to_owned(),
                });
            }
        }),
    };

    Ok(provider)
}

pub async fn probe_provider_adapters(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapters: &BTreeMap<String, ResolvedProviderAdapterConfig>,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
) -> Vec<ProviderAdapterProbeResult> {
    let mut results = Vec::new();
    for (adapter_id, adapter) in adapters {
        let resolved_base_url =
            resolved_adapter_probe_base_url(provider_id, auth, &adapter.definition)
                .ok()
                .flatten();
        if !adapter.enabled {
            results.push(ProviderAdapterProbeResult {
                adapter_id: adapter_id.clone(),
                enabled: false,
                supported: false,
                resolved_base_url,
                models: Vec::new(),
                error: Some("adapter is disabled".to_owned()),
            });
            continue;
        }

        let provider = match build_adapter_provider(
            provider_id,
            adapter_id.as_str(),
            adapter,
            PROBE_DEFAULT_MODEL_ID,
            auth,
            client.clone(),
            env,
        ) {
            Ok(provider) => provider,
            Err(err) => {
                results.push(ProviderAdapterProbeResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
                    supported: false,
                    resolved_base_url,
                    models: Vec::new(),
                    error: Some(err.to_string()),
                });
                continue;
            }
        };

        match provider.list_models().await {
            Ok(mut models) => {
                for model in &mut models {
                    model.provider_id = ProviderId::new(provider_id.to_owned());
                    model.adapter_id = Some(AdapterId::new(adapter_id.clone()));
                    let catalog_model_id =
                        crate::model_catalog::canonical_model_catalog_id(model.id.as_str());
                    if !catalog_model_id.is_empty() {
                        model.catalog_model_id = Some(crate::model::ModelId::new(catalog_model_id));
                    }
                    let fallback = provider.model_capabilities_for_adapter(None, &model.id);
                    model.capabilities = model.capabilities.clone().with_fallbacks_from(&fallback);
                    let metadata_fallback = provider.model_metadata_for_adapter(None, &model.id);
                    model.metadata = model
                        .metadata
                        .clone()
                        .with_fallbacks_from(&metadata_fallback);
                    if model.thinking_modes.is_empty() {
                        model.thinking_modes =
                            provider.model_thinking_modes_for_adapter(None, &model.id);
                    }
                    if model.speed_modes.is_empty() {
                        model.speed_modes = provider.model_speed_modes_for_adapter(None, &model.id);
                    }
                }
                results.push(ProviderAdapterProbeResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
                    supported: true,
                    resolved_base_url,
                    models,
                    error: None,
                });
            }
            Err(err) => {
                results.push(ProviderAdapterProbeResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
                    supported: false,
                    resolved_base_url,
                    models: Vec::new(),
                    error: Some(err.to_string()),
                });
            }
        }
    }
    results.sort_by(|left, right| left.adapter_id.cmp(&right.adapter_id));
    results
}

fn resolve_adapter_default_models(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Result<std::collections::BTreeMap<String, String>, ConfigError> {
    let mut defaults = std::collections::BTreeMap::new();
    for adapter_id in resolved
        .adapters
        .iter()
        .filter(|(_, adapter)| adapter.enabled)
        .map(|(adapter_id, _)| adapter_id)
    {
        if resolved.default_model.trim().is_empty() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!("provider default_model is empty for adapter `{adapter_id}`"),
            });
        }
        defaults.insert(adapter_id.clone(), resolved.default_model.clone());
    }

    Ok(defaults)
}

fn resolved_adapter_probe_base_url(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    definition: &ProviderAdapterDefinition,
) -> Result<Option<String>, ConfigError> {
    match definition {
        ProviderAdapterDefinition::OpenAi(_) => Ok(Some(resolve_http_adapter_base_url(
            provider_id,
            auth,
            HttpAdapterKind::OpenAi,
        )?)),
        ProviderAdapterDefinition::Anthropic(_) => Ok(Some(resolve_http_adapter_base_url(
            provider_id,
            auth,
            HttpAdapterKind::Anthropic,
        )?)),
        ProviderAdapterDefinition::Gemini(_) => Ok(Some(resolve_http_adapter_base_url(
            provider_id,
            auth,
            HttpAdapterKind::Gemini,
        )?)),
        ProviderAdapterDefinition::Ollama(adapter) => Ok(Some(
            adapter
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_owned()),
        )),
        ProviderAdapterDefinition::Gitlab(adapter) => Ok(Some(
            adapter
                .ai_gateway_url
                .clone()
                .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned()),
        )),
        ProviderAdapterDefinition::AmazonBedrock(_) => Ok(Some(
            provider_endpoint_root(auth, provider_id)?.0.to_owned(),
        )),
    }
}

fn api_auth<'a>(
    auth: &'a ProviderAuthConfig,
    provider_id: &str,
) -> Result<&'a ProviderApiAuthConfig, ConfigError> {
    match auth {
        ProviderAuthConfig::Api(api) => Ok(api),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "adapter requires api auth".to_owned(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpAdapterKind {
    OpenAi,
    Anthropic,
    Gemini,
}

fn resolve_http_adapter_base_url(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapter: HttpAdapterKind,
) -> Result<String, ConfigError> {
    let (base_url, layout) = provider_endpoint_root(auth, provider_id)?;
    let normalized = normalize_base_url(base_url)?;
    let resolved_layout = resolve_endpoint_layout(layout, normalized.as_str());
    let gateway_root = normalize_gateway_root(normalized.as_str(), resolved_layout)?;
    Ok(match resolved_layout {
        SharedGatewayEndpointLayout::Direct => normalized,
        SharedGatewayEndpointLayout::ProtocolRoot => {
            protocol_root_adapter_base(gateway_root.as_str(), adapter)
        }
        SharedGatewayEndpointLayout::ProviderRouted => {
            provider_routed_adapter_base(gateway_root.as_str(), adapter)
        }
        SharedGatewayEndpointLayout::Auto => unreachable!("auto layout must be resolved"),
    })
}

fn provider_endpoint_root<'a>(
    auth: &'a ProviderAuthConfig,
    provider_id: &str,
) -> Result<(&'a str, SharedGatewayEndpointLayout), ConfigError> {
    match auth {
        ProviderAuthConfig::Api(config) => Ok((config.base_url.as_str(), config.endpoint_layout)),
        ProviderAuthConfig::GoogleAdc(config) => {
            Ok((config.base_url.as_str(), config.endpoint_layout))
        }
        ProviderAuthConfig::SapAiCore(config) => {
            Ok((config.api.base_url.as_str(), config.api.endpoint_layout))
        }
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "provider auth does not define an api base_url".to_owned(),
        }),
    }
}

fn resolve_endpoint_layout(
    layout: SharedGatewayEndpointLayout,
    normalized_base_url: &str,
) -> SharedGatewayEndpointLayout {
    match layout {
        SharedGatewayEndpointLayout::Auto => {
            if normalized_base_url.contains("/api/provider/") {
                SharedGatewayEndpointLayout::ProviderRouted
            } else {
                let path = url::Url::parse(normalized_base_url)
                    .ok()
                    .map(|url| url.path().trim_end_matches('/').to_owned())
                    .unwrap_or_default();
                if path.is_empty()
                    || path.ends_with("/v1")
                    || path.ends_with("/v1beta")
                    || path.ends_with("/chat/completions")
                    || path.ends_with("/responses")
                    || path.ends_with("/messages")
                    || path.ends_with("/models")
                    || path.contains(":generateContent")
                    || path.contains(":streamGenerateContent")
                {
                    SharedGatewayEndpointLayout::ProtocolRoot
                } else {
                    SharedGatewayEndpointLayout::Direct
                }
            }
        }
        other => other,
    }
}

fn normalize_base_url(value: &str) -> Result<String, ConfigError> {
    let mut url = url::Url::parse(value).map_err(|err| {
        ConfigError::Validation(format!(
            "provider auth base_url `{value}` is invalid: {err}"
        ))
    })?;
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if path.is_empty() { "/" } else { path.as_str() });
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn normalize_gateway_root(
    normalized_base_url: &str,
    layout: SharedGatewayEndpointLayout,
) -> Result<String, ConfigError> {
    let mut url = url::Url::parse(normalized_base_url).map_err(|err| {
        ConfigError::Validation(format!(
            "provider auth base_url `{normalized_base_url}` is invalid: {err}"
        ))
    })?;
    let segments = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let keep = match layout {
        SharedGatewayEndpointLayout::Direct => segments.len(),
        SharedGatewayEndpointLayout::ProtocolRoot => {
            trim_protocol_root_segments(segments.as_slice())
        }
        SharedGatewayEndpointLayout::ProviderRouted => {
            trim_provider_routed_segments(segments.as_slice())
        }
        SharedGatewayEndpointLayout::Auto => unreachable!("auto layout must be resolved"),
    };

    let path = if keep == 0 {
        "/".to_owned()
    } else {
        format!("/{}", segments[..keep].join("/"))
    };
    url.set_path(path.as_str());
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn trim_protocol_root_segments(segments: &[String]) -> usize {
    if let Some(mut len) = trim_known_endpoint_suffix(segments) {
        if len > 0
            && matches!(
                segments.get(len - 1).map(String::as_str),
                Some("v1" | "v1beta")
            )
        {
            len -= 1;
        }
        return len;
    }
    if matches!(segments.last().map(String::as_str), Some("v1" | "v1beta")) {
        return segments.len().saturating_sub(1);
    }
    segments.len()
}

fn trim_provider_routed_segments(segments: &[String]) -> usize {
    if let Some(index) = segments
        .windows(3)
        .position(|window| window[0] == "api" && window[1] == "provider")
    {
        return index;
    }
    trim_protocol_root_segments(segments)
}

fn trim_known_endpoint_suffix(segments: &[String]) -> Option<usize> {
    if segments.len() >= 2
        && segments[segments.len() - 2] == "chat"
        && segments[segments.len() - 1] == "completions"
    {
        return Some(segments.len() - 2);
    }
    if segments.len() >= 2
        && segments[segments.len() - 2] == "models"
        && matches!(
            segments.last().map(String::as_str),
            Some(last)
                if last.contains(":generateContent") || last.contains(":streamGenerateContent")
        )
    {
        return Some(segments.len() - 2);
    }
    if matches!(
        segments.last().map(String::as_str),
        Some("responses" | "messages" | "models")
    ) {
        return Some(segments.len() - 1);
    }
    if let Some(last) = segments.last()
        && (last.contains(":generateContent") || last.contains(":streamGenerateContent"))
    {
        return Some(segments.len().saturating_sub(1));
    }
    None
}

fn protocol_root_adapter_base(root: &str, adapter: HttpAdapterKind) -> String {
    match adapter {
        HttpAdapterKind::OpenAi | HttpAdapterKind::Anthropic => format!("{root}/v1"),
        HttpAdapterKind::Gemini => format!("{root}/v1beta"),
    }
}

fn provider_routed_adapter_base(root: &str, adapter: HttpAdapterKind) -> String {
    match adapter {
        HttpAdapterKind::OpenAi => format!("{root}/api/provider/openai/v1"),
        HttpAdapterKind::Anthropic => format!("{root}/api/provider/anthropic/v1"),
        HttpAdapterKind::Gemini => format!("{root}/api/provider/google/v1beta"),
    }
}

fn openai_adapter_api_credential(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    client: reqwest::Client,
    capability_family: Option<ProviderCapabilityFamilyConfig>,
    env: &dyn ConfigEnvironment,
) -> Result<ResolvedManagedCredential, ConfigError> {
    match auth {
        ProviderAuthConfig::Api(_) => api_auth_managed_credential(
            provider_id,
            "api_key",
            auth,
            AuthSecretSelector::AccessOrApiKey,
            AuthRefreshStrategy::OpenAiOAuth,
            env,
            true,
        ),
        ProviderAuthConfig::GoogleAdc(_) => {
            if !matches!(
                capability_family,
                Some(ProviderCapabilityFamilyConfig::Gemini)
            ) {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "google_adc auth only supports Vertex-style `openai` adapters"
                        .to_owned(),
                });
            }
            Ok(ResolvedManagedCredential {
                credential: ManagedCredential::google_adc(
                    format!("{provider_id} google adc"),
                    provider_id.to_owned(),
                ),
                auth_data: None,
            })
        }
        ProviderAuthConfig::SapAiCore(_) => Ok(ResolvedManagedCredential {
            credential: sap_ai_core_managed_credential(provider_id, client, auth, env)?,
            auth_data: None,
        }),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "openai adapter requires api/google_adc/credential/sap_ai_core auth"
                .to_owned(),
        }),
    }
}

struct ResolvedManagedCredential {
    credential: ManagedCredential,
    auth_data: Option<Arc<Mutex<AuthData>>>,
}

fn api_auth_has_direct_source(api: &ProviderApiAuthConfig, env: &dyn ConfigEnvironment) -> bool {
    match (
        api.api_key
            .as_ref()
            .and_then(|value| normalize_text(value.as_str())),
        api.api_key_env
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

fn required_api_auth_credential(
    provider_id: &str,
    field: &'static str,
    api: &ProviderApiAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<ManagedCredential, ConfigError> {
    if let Some(value) = api
        .api_key
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()))
    {
        return Ok(ManagedCredential::static_value(
            format!("{provider_id} {field}"),
            value,
        ));
    }

    let Some(env_key) = api
        .api_key_env
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

fn api_auth_managed_credential(
    provider_id: &str,
    field: &'static str,
    auth: &ProviderAuthConfig,
    _selector: AuthSecretSelector,
    _refresh: AuthRefreshStrategy,
    env: &dyn ConfigEnvironment,
    allow_deferred_env: bool,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let api = api_auth(auth, provider_id)?;
    if let Some(value) = api
        .api_key
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()))
    {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::static_value(format!("{provider_id} {field}"), value),
            auth_data: None,
        });
    }

    let env_key = api
        .api_key_env
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
    _env: &dyn ConfigEnvironment,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let ProviderAuthConfig::Credential(config) = auth else {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("{field} must come from provider credential auth"),
        });
    };
    if let Some(auth_data) = config.credential.clone() {
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
            let api_auth = ProviderAuthConfig::Api(config.api.clone());
            match api_auth_managed_credential(
                provider_id,
                "api_key",
                &api_auth,
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
        ProviderAuthConfig::Api(_) => api_auth_managed_credential(
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
            message: "sap_ai_core token resolution requires api or sap_ai_core auth".to_owned(),
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
    _models_url: Option<&str>,
) -> Option<String> {
    let base_url = "https://api.githubcopilot.com";
    if base_url == "https://api.githubcopilot.com"
        && auth_data.and_then(current_enterprise_url).is_some()
    {
        None
    } else {
        Some(base_url.to_owned())
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

    use crate::config::SharedGatewayEndpointLayout;
    use crate::config::{
        ConfigEnvironment, ConfigError, ConfigLoader, LoadConfigRequest, ProviderAuthConfig,
    };
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
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true
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
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-4.1-mini"]
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
            .model_capabilities(&crate::model::ModelRef::new_with_adapter(
                "openai",
                "openai",
                "gpt-4.1-mini",
            ))
            .expect("provider capabilities should resolve");
        assert_eq!(capabilities.image_input, CapabilitySupport::Unsupported);
        assert_eq!(capabilities.document_input, CapabilitySupport::Supported);
    }

    #[test]
    fn registry_builder_accepts_inline_api_key_for_openai_without_config_secret() {
        let path = write_temp_file(
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "sk-from-config"

[providers.openai.adapters.openai]
enabled = true
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
default_model = "openai/gpt-5.3-codex"

[providers.openai.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.openai.adapters.openai]
enabled = true
backend = "chatgpt_codex"
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
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "sk-from-config"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true
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
default_model = "gitlab/claude-sonnet-4-5"

[providers.gitlab.auth]
mode = "credential"
issuer = "gitlab"
credential = { type = "oauth", issuer = "gitlab", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000 }

[providers.gitlab.adapters.gitlab]
enabled = true
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
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
default_model = "openai/gpt-5.3-codex"

[providers.openai_chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"

[providers.openai_chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"
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
default_model = "openai/gpt-5.3-codex"

[providers.openai_chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.openai_chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"
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
default_model = "openai/gpt-4o-mini"

[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"

[providers."github-copilot".adapters.openai]
enabled = true
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
    fn registry_builder_registers_github_copilot_via_openai_adapter_with_inline_oauth() {
        let path = write_temp_file(
            r#"
[providers."github-copilot"]
default_model = "openai/gpt-4o-mini"

[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "copilot-refresh-token", access = "copilot-access-token", expires_at_ms = 4102444800000 }

[providers."github-copilot".adapters.openai]
enabled = true
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
            .expect("registry should build with inline copilot oauth");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "github-copilot"));
    }

    #[test]
    fn registry_builder_registers_atomgit_via_openai_adapter_with_inline_oauth() {
        let path = write_temp_file(
            r#"
[providers.atomgit]
default_model = "openai/Kimi-K2-Instruct"

[providers.atomgit.auth]
mode = "credential"
issuer = "atomgit"
credential = { type = "oauth", issuer = "atomgit", refresh = "atomgit-refresh-token", access = "atomgit-access-token", expires_at_ms = 4102444800000, account_id = "atomgit-user" }

[providers.atomgit.adapters.openai]
enabled = true
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
            .expect("registry should build with inline atomgit oauth");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "atomgit"));
    }

    #[test]
    fn registry_builder_registers_github_copilot_via_anthropic_adapter_with_inline_oauth() {
        let path = write_temp_file(
            r#"
[providers."github-copilot-claude"]
default_model = "anthropic/claude-sonnet-4"

[providers."github-copilot-claude".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "copilot-refresh-token", access = "copilot-access-token", expires_at_ms = 4102444800000 }

[providers."github-copilot-claude".adapters.anthropic]
enabled = true
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
            .expect("registry should build with inline copilot oauth");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "github-copilot-claude"));
    }

    #[test]
    fn registry_builder_requires_gitlab_auth_when_no_direct_secret_exists() {
        let path = write_temp_file(
            r#"
[providers.gitlab]
default_model = "gitlab/claude-sonnet-4-5"

[providers.gitlab.auth]
mode = "credential"
issuer = "gitlab"

[providers.gitlab.adapters.gitlab]
enabled = true
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
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
default_model = "openai/google/gemini-2.5-flash"

[providers."google-vertex".auth]
mode = "api"
base_url = "https://us-central1-aiplatform.googleapis.com/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
api_key_env = "GOOGLE_VERTEX_ACCESS_TOKEN"

[providers."google-vertex".adapters.openai]
enabled = true
capability_family = "gemini"
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
default_model = "openai/anthropic/claude-sonnet-4"

[providers.sap.auth]
mode = "sap_ai_core"
base_url = "https://api.example.com/v2"
api_key = "sap-api-token"

[providers.sap.adapters.openai]
enabled = true
auth_header = "authorization"
auth_scheme = "Bearer"
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

    #[test]
    fn shared_gateway_protocol_root_derives_adapter_bases() {
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
                    base_url: "https://api.cxits.cn/v1".to_owned(),
                    endpoint_layout: SharedGatewayEndpointLayout::ProtocolRoot,
                    api_key: None,
                    api_key_env: None,
                }),
                super::HttpAdapterKind::OpenAi,
            )
            .expect("openai base should resolve"),
            "https://api.cxits.cn/v1"
        );
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
                    base_url: "https://api.cxits.cn/v1".to_owned(),
                    endpoint_layout: SharedGatewayEndpointLayout::ProtocolRoot,
                    api_key: None,
                    api_key_env: None,
                }),
                super::HttpAdapterKind::Anthropic,
            )
            .expect("anthropic base should resolve"),
            "https://api.cxits.cn/v1"
        );
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
                    base_url: "https://api.cxits.cn/v1".to_owned(),
                    endpoint_layout: SharedGatewayEndpointLayout::ProtocolRoot,
                    api_key: None,
                    api_key_env: None,
                }),
                super::HttpAdapterKind::Gemini,
            )
            .expect("gemini base should resolve"),
            "https://api.cxits.cn/v1beta"
        );
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
                    base_url: "https://api.cxits.cn".to_owned(),
                    endpoint_layout: SharedGatewayEndpointLayout::ProtocolRoot,
                    api_key: None,
                    api_key_env: None,
                }),
                super::HttpAdapterKind::OpenAi,
            )
            .expect("openai root base should resolve"),
            "https://api.cxits.cn/v1"
        );
    }

    #[test]
    fn shared_gateway_provider_routed_derives_adapter_bases() {
        let auth = ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
            base_url: "https://api.cxits.cn/api/provider/openai/v1".to_owned(),
            endpoint_layout: SharedGatewayEndpointLayout::ProviderRouted,
            api_key: None,
            api_key_env: None,
        });
        assert_eq!(
            super::resolve_http_adapter_base_url("shared", &auth, super::HttpAdapterKind::OpenAi)
                .expect("openai base should resolve"),
            "https://api.cxits.cn/api/provider/openai/v1"
        );
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &auth,
                super::HttpAdapterKind::Anthropic,
            )
            .expect("anthropic base should resolve"),
            "https://api.cxits.cn/api/provider/anthropic/v1"
        );
        assert_eq!(
            super::resolve_http_adapter_base_url("shared", &auth, super::HttpAdapterKind::Gemini)
                .expect("gemini base should resolve"),
            "https://api.cxits.cn/api/provider/google/v1beta"
        );
    }

    #[test]
    fn shared_gateway_auto_detects_routed_and_protocol_layouts() {
        let routed = ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
            base_url: "https://api.cxits.cn/api/provider/openai/v1".to_owned(),
            endpoint_layout: SharedGatewayEndpointLayout::Auto,
            api_key: None,
            api_key_env: None,
        });
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &routed,
                super::HttpAdapterKind::Anthropic,
            )
            .expect("anthropic routed base should resolve"),
            "https://api.cxits.cn/api/provider/anthropic/v1"
        );

        let protocol_root = ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
            base_url: "https://api.cxits.cn/v1/messages".to_owned(),
            endpoint_layout: SharedGatewayEndpointLayout::Auto,
            api_key: None,
            api_key_env: None,
        });
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &protocol_root,
                super::HttpAdapterKind::Gemini,
            )
            .expect("gemini protocol-root base should resolve"),
            "https://api.cxits.cn/v1beta"
        );

        let gemini_endpoint = ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
            base_url: "https://api.cxits.cn/v1beta/models/gemini-2.5-pro:generateContent"
                .to_owned(),
            endpoint_layout: SharedGatewayEndpointLayout::Auto,
            api_key: None,
            api_key_env: None,
        });
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &gemini_endpoint,
                super::HttpAdapterKind::OpenAi,
            )
            .expect("openai base should normalize from gemini endpoint"),
            "https://api.cxits.cn/v1"
        );
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "shared",
                &gemini_endpoint,
                super::HttpAdapterKind::Gemini,
            )
            .expect("gemini base should normalize from gemini endpoint"),
            "https://api.cxits.cn/v1beta"
        );

        let direct = ProviderAuthConfig::Api(crate::config::ProviderApiAuthConfig {
            base_url:
                "https://aiplatform.googleapis.com/v1/projects/p/locations/l/endpoints/openapi"
                    .to_owned(),
            endpoint_layout: SharedGatewayEndpointLayout::Auto,
            api_key: None,
            api_key_env: None,
        });
        assert_eq!(
            super::resolve_http_adapter_base_url(
                "vertex",
                &direct,
                super::HttpAdapterKind::OpenAi,
            )
                .expect("direct base should resolve"),
            "https://aiplatform.googleapis.com/v1/projects/p/locations/l/endpoints/openapi"
        );
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
