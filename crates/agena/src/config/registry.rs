use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
};

use aws_credential_types::Credentials;
use tokio::sync::Mutex;

use crate::{
    error::AppError,
    model::{AdapterId, ProviderId},
    model_catalog::ModelCatalogSnapshot,
    plugin::{PluginHost, PluginHostBuilder},
    provider::{
        AmazonBedrockAdapter, AnthropicAdapter, AnthropicProfile, AuthRefreshStrategy,
        AuthSecretSelector, CatalogedModelsProvider, GeminiAdapter, GitlabProvider,
        GitlabProviderConfig, ManagedCredential, ModelCapabilities, ModelId, ModelMetadata,
        ModelRuntime, ModelSpeedMode, ModelThinkingMode, MultiAdapterProvider, OllamaAdapter,
        OpenAiAdapter, OpenAiApiMode, PromptCacheShape, ProviderModelRoute, ProviderRegistry,
        StreamResumePolicy, auth::AuthData, parse_sap_ai_core_service_key,
    },
};

use super::raw::parse_adapter_model_ref;
use super::{
    ConfigEnvironment, ConfigError, HttpProviderAdapterConfig, OpenAiApiModeConfig,
    ProcessEnvironment, ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig,
    ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig, ProviderModelDiscoveryConfig,
    ProviderProtocolPathsConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig,
};

const ATOMGIT_GATEWAY_ROOT: &str = "https://api-ai.gitcode.com";
const ATOMGIT_LLM_BASE_URL: &str = "https://api-ai.gitcode.com/v1";
// AtomGit's LLM gateway is OpenAI-compatible for inference, but model
// catalogue lookup is served by the CodingPlan API.
const ATOMGIT_CODING_PLAN_MODELS_URL: &str = "https://api.gitcode.com/api/v5/coding-plan/models-v2";
const LIST_MODELS_DEFAULT_MODEL_ID: &str = "__list_models__";

#[derive(Debug, Clone)]
pub struct ProviderAdapterModelsResult {
    pub adapter_id: String,
    pub enabled: bool,
    pub resolved_base_url: Option<String>,
    pub models: Vec<crate::provider::ProviderModel>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitlabRoutedBackend {
    OpenAi,
    Anthropic,
}

impl GitlabRoutedBackend {
    fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    fn matches_model(self, model: &ModelId) -> bool {
        let mapped = GitlabProvider::mapped_model(model.as_str());
        GitlabProvider::use_openai_backend(mapped.as_str()) == matches!(self, Self::OpenAi)
    }
}

#[derive(Clone)]
struct GitlabRoutedAdapter {
    inner: Arc<GitlabProvider>,
    backend: GitlabRoutedBackend,
    default_model: ModelId,
}

impl GitlabRoutedAdapter {
    fn supports_model(&self, model: &ModelId) -> bool {
        self.backend.matches_model(model)
    }

    fn backend_mismatch_error(&self, model: &ModelId) -> AppError {
        AppError::Config(format!(
            "gitlab auth routed adapter `{}` does not support model `{}`",
            self.backend.label(),
            model
        ))
    }
}

#[async_trait::async_trait]
impl ModelRuntime for GitlabRoutedAdapter {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        self.inner.capability_family()
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        if self.supports_model(model) {
            self.inner.model_capabilities(model)
        } else {
            ModelCapabilities::default()
        }
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        if self.supports_model(model) {
            self.inner.model_metadata(model)
        } else {
            ModelMetadata::default()
        }
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        if self.supports_model(model) {
            self.inner.model_thinking_modes(model)
        } else {
            BTreeMap::new()
        }
    }

    fn model_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        if self.supports_model(model) {
            self.inner.model_speed_modes(model)
        } else {
            BTreeMap::new()
        }
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.inner.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.supports_model(model) && self.inner.supports_prompt_continuation(model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.supports_model(model)
            .then(|| self.inner.prompt_cache_shape(model))
            .flatten()
    }

    async fn list_models(&self) -> Result<Vec<crate::provider::Model>, AppError> {
        Ok(self
            .inner
            .list_models()
            .await?
            .into_iter()
            .filter(|model| self.supports_model(&model.id))
            .collect())
    }

    async fn complete(
        &self,
        request: crate::provider::CompletionRequest,
    ) -> Result<crate::provider::CompletionResponse, AppError> {
        if !self.supports_model(&request.model) {
            return Err(self.backend_mismatch_error(&request.model));
        }
        self.inner.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: crate::provider::CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_core::Stream<
                        Item = Result<crate::provider::CompletionStreamEvent, AppError>,
                    > + Send,
            >,
        >,
        AppError,
    > {
        if !self.supports_model(&request.model) {
            return Err(self.backend_mismatch_error(&request.model));
        }
        self.inner.complete_stream(request).await
    }
}

impl ResolvedConfig {
    pub fn build_provider_http_client(&self) -> Result<reqwest::Client, AppError> {
        ProviderRegistry::build_http_client(self.provider_http_client_config())
    }

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
        let client = self.build_provider_http_client()?;
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
                    disabled: false,
                });
        }
        let mut builder = PluginHostBuilder::new(workspace_root, agena_version)
            .with_config(plugin_config)
            .register_static(crate::tool::lsp_plugin_id(), crate::tool::new_lsp_plugin())
            .register_static(
                crate::tool::cron_plugin_id(),
                crate::tool::new_cron_plugin(),
            )
            .register_static(
                crate::tool::code_plugin_id(),
                crate::tool::new_code_plugin(),
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
) -> Result<Arc<dyn ModelRuntime>, ConfigError> {
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

    let configured_only_adapters = resolved
        .adapters
        .iter()
        .filter(|(_, adapter)| {
            adapter.enabled
                && matches!(
                    adapter.model_discovery,
                    ProviderModelDiscoveryConfig::ConfiguredOnly
                )
        })
        .map(|(adapter_id, _)| adapter_id.clone())
        .collect();

    let provider: Arc<dyn ModelRuntime> = Arc::new(
        MultiAdapterProvider::new(
            provider_id,
            resolved
                .defaults
                .adapter
                .clone()
                .expect("resolved provider default adapter"),
            resolved
                .defaults
                .model
                .clone()
                .expect("resolved provider default model"),
            adapters,
            routes,
        )
        .with_configured_only_adapters(configured_only_adapters),
    );

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
) -> Result<Arc<dyn ModelRuntime>, ConfigError> {
    let runtime_provider_id = runtime_adapter_provider_id(provider_id, adapter_id);
    let provider: Arc<dyn ModelRuntime> = match &config.definition {
        ProviderAdapterDefinition::Ollama(adapter) => Arc::new(OllamaAdapter::new(
            runtime_provider_id.as_str(),
            client,
            adapter
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_owned()),
            adapter_default_model.to_owned(),
        )),
        ProviderAdapterDefinition::OpenAi(adapter) => match auth {
            ProviderAuthConfig::Gitlab(config) => Arc::new(GitlabRoutedAdapter {
                inner: Arc::new(GitlabProvider::from_managed_token_with_config(
                    client,
                    gitlab_auth_managed_credential(provider_id, auth, env)?.credential,
                    gitlab_runtime_config(config, adapter_default_model),
                )?),
                backend: GitlabRoutedBackend::OpenAi,
                default_model: ModelId::new(adapter_default_model),
            }),
            ProviderAuthConfig::Credential(credential_auth)
                if credential_auth.issuer == crate::provider::auth::CredentialIssuer::Gitlab =>
            {
                Arc::new(GitlabRoutedAdapter {
                    inner: Arc::new(GitlabProvider::from_managed_token_with_config(
                        client,
                        require_provider_auth_credential(
                            provider_id,
                            "api_key",
                            auth,
                            AuthSecretSelector::AccessOrApiKey,
                            AuthRefreshStrategy::GitlabOAuth {
                                instance_url: gitlab_credential_instance_url(credential_auth),
                            },
                            env,
                        )?
                        .credential,
                        gitlab_credential_runtime_config(credential_auth, adapter_default_model),
                    )?),
                    backend: GitlabRoutedBackend::OpenAi,
                    default_model: ModelId::new(adapter_default_model),
                })
            }
            ProviderAuthConfig::Api(_) => {
                let credential = openai_adapter_api_credential(
                    provider_id,
                    auth,
                    client.clone(),
                    adapter.options.capability_family,
                    env,
                )?;
                let mut provider = OpenAiAdapter::new_managed_with_id(
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
                .with_extra_headers(http_adapter_extra_headers(
                    adapter,
                    Some(http_adapter_default_user_agent(
                        auth,
                        HttpAdapterKind::OpenAi,
                        adapter_default_model,
                    )),
                ))
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
                    let mut provider = OpenAiAdapter::new_managed_with_id(
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
                    .with_extra_headers(http_adapter_extra_headers(
                        adapter,
                        Some(http_adapter_default_user_agent(
                            auth,
                            HttpAdapterKind::OpenAi,
                            adapter_default_model,
                        )),
                    ))
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
                    let mut provider = OpenAiAdapter::new_managed_with_id(
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
                    .with_extra_headers(http_adapter_extra_headers(
                        adapter,
                        Some(http_adapter_default_user_agent(
                            auth,
                            HttpAdapterKind::OpenAi,
                            adapter_default_model,
                        )),
                    ));
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
                    let mut provider = OpenAiAdapter::new_managed_with_id(
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
                    .with_extra_headers(http_adapter_extra_headers(
                        adapter,
                        Some(http_adapter_default_user_agent(
                            auth,
                            HttpAdapterKind::OpenAi,
                            adapter_default_model,
                        )),
                    ))
                    .with_api_mode(atomgit_openai_api_mode(
                        adapter.options.api_mode,
                        adapter.options.api_mode_explicit,
                    ))
                    .with_api_mode_explicit(adapter.options.api_mode_explicit)
                    .with_stream_mode(adapter.options.stream_mode.into())
                    .with_models_url(atomgit_model_listing_url(
                        adapter.options.models_url.clone(),
                    ))
                    .with_atomgit_coding_plan_models(adapter.options.models_url.is_none())
                    .with_realtime_ws_url(adapter.options.realtime_ws_url.clone());

                    if let Some(family) = adapter.options.capability_family {
                        provider = provider.with_capability_family(family.into());
                    }

                    if let Some(auth_data) = credential.auth_data {
                        provider = provider.with_auth_data(auth_data);
                    }

                    Arc::new(provider)
                }
                crate::provider::auth::CredentialIssuer::GoogleAdc
                | crate::provider::auth::CredentialIssuer::SapAiCore => {
                    let credential = openai_adapter_api_credential(
                        provider_id,
                        auth,
                        client.clone(),
                        adapter.options.capability_family,
                        env,
                    )?;
                    let mut provider = OpenAiAdapter::new_managed_with_id(
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
                    .with_extra_headers(http_adapter_extra_headers(
                        adapter,
                        Some(http_adapter_default_user_agent(
                            auth,
                            HttpAdapterKind::OpenAi,
                            adapter_default_model,
                        )),
                    ))
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
                    message: "openai adapter requires compatible api or credential auth".to_owned(),
                });
            }
        },
        ProviderAdapterDefinition::Anthropic(adapter) => match auth {
            ProviderAuthConfig::Gitlab(config) => Arc::new(GitlabRoutedAdapter {
                inner: Arc::new(GitlabProvider::from_managed_token_with_config(
                    client,
                    gitlab_auth_managed_credential(provider_id, auth, env)?.credential,
                    gitlab_runtime_config(config, adapter_default_model),
                )?),
                backend: GitlabRoutedBackend::Anthropic,
                default_model: ModelId::new(adapter_default_model),
            }),
            ProviderAuthConfig::Credential(credential_auth)
                if credential_auth.issuer == crate::provider::auth::CredentialIssuer::Gitlab =>
            {
                Arc::new(GitlabRoutedAdapter {
                    inner: Arc::new(GitlabProvider::from_managed_token_with_config(
                        client,
                        require_provider_auth_credential(
                            provider_id,
                            "api_key",
                            auth,
                            AuthSecretSelector::AccessOrApiKey,
                            AuthRefreshStrategy::GitlabOAuth {
                                instance_url: gitlab_credential_instance_url(credential_auth),
                            },
                            env,
                        )?
                        .credential,
                        gitlab_credential_runtime_config(credential_auth, adapter_default_model),
                    )?),
                    backend: GitlabRoutedBackend::Anthropic,
                    default_model: ModelId::new(adapter_default_model),
                })
            }
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

                let mut provider = AnthropicAdapter::new_managed_with_id(
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
                .with_eager_input_streaming_override(adapter.options.eager_input_streaming)
                .with_extra_headers(http_adapter_extra_headers(
                    adapter,
                    Some(http_adapter_default_user_agent(
                        auth,
                        HttpAdapterKind::Anthropic,
                        adapter_default_model,
                    )),
                ));

                if let Some(auth_data) = credential.auth_data {
                    provider = provider.with_auth_data(auth_data);
                }

                Arc::new(provider)
            }
            _ => Arc::new(
                AnthropicAdapter::new_managed_with_id(
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
                .with_eager_input_streaming_override(adapter.options.eager_input_streaming)
                .with_extra_headers(http_adapter_extra_headers(
                    adapter,
                    Some(http_adapter_default_user_agent(
                        auth,
                        HttpAdapterKind::Anthropic,
                        adapter_default_model,
                    )),
                )),
            ),
        },
        ProviderAdapterDefinition::Gemini(adapter) => Arc::new({
            let mut provider = GeminiAdapter::new_managed(
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
            .with_extra_headers(http_adapter_extra_headers(
                adapter,
                Some(http_adapter_default_user_agent(
                    auth,
                    HttpAdapterKind::Gemini,
                    adapter_default_model,
                )),
            ))
            .with_stream_mode(adapter.options.stream_mode.into())
            .with_realtime_ws_url(adapter.options.realtime_ws_url.clone());
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
                ProviderAuthConfig::Gitlab(_) => {
                    gitlab_auth_managed_credential(provider_id, auth, env)?.credential
                }
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
        ProviderAdapterDefinition::AmazonBedrock(_) => Arc::new(match auth {
            ProviderAuthConfig::BedrockSigv4(sigv4) => AmazonBedrockAdapter::new_sigv4(
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

pub async fn list_provider_adapter_models(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapters: &BTreeMap<String, ResolvedProviderAdapterConfig>,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
) -> Vec<ProviderAdapterModelsResult> {
    let mut results = Vec::new();
    for (adapter_id, adapter) in adapters {
        let resolved_base_url =
            resolved_adapter_models_base_url(provider_id, auth, &adapter.definition)
                .ok()
                .flatten();
        if !adapter.enabled {
            results.push(ProviderAdapterModelsResult {
                adapter_id: adapter_id.clone(),
                enabled: false,
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
            LIST_MODELS_DEFAULT_MODEL_ID,
            auth,
            client.clone(),
            env,
        ) {
            Ok(provider) => provider,
            Err(err) => {
                results.push(ProviderAdapterModelsResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
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
                results.push(ProviderAdapterModelsResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
                    resolved_base_url,
                    models,
                    error: None,
                });
            }
            Err(err) => {
                results.push(ProviderAdapterModelsResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
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
        let default_model =
            resolved
                .defaults
                .model
                .clone()
                .ok_or_else(|| ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "provider defaults.model is missing for adapter `{adapter_id}`"
                    ),
                })?;
        defaults.insert(adapter_id.clone(), default_model);
    }

    Ok(defaults)
}

fn resolved_adapter_models_base_url(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    definition: &ProviderAdapterDefinition,
) -> Result<Option<String>, ConfigError> {
    match definition {
        ProviderAdapterDefinition::OpenAi(_) => match auth {
            ProviderAuthConfig::Gitlab(config) => Ok(Some(gitlab_proxy_base_url(
                config,
                GitlabRoutedBackend::OpenAi,
            ))),
            ProviderAuthConfig::Credential(config)
                if config.issuer == crate::provider::auth::CredentialIssuer::Gitlab =>
            {
                Ok(Some(gitlab_credential_proxy_base_url(
                    config,
                    GitlabRoutedBackend::OpenAi,
                )))
            }
            _ => Ok(Some(resolve_http_adapter_base_url(
                provider_id,
                auth,
                HttpAdapterKind::OpenAi,
            )?)),
        },
        ProviderAdapterDefinition::Anthropic(_) => match auth {
            ProviderAuthConfig::Gitlab(config) => Ok(Some(gitlab_proxy_base_url(
                config,
                GitlabRoutedBackend::Anthropic,
            ))),
            ProviderAuthConfig::Credential(config)
                if config.issuer == crate::provider::auth::CredentialIssuer::Gitlab =>
            {
                Ok(Some(gitlab_credential_proxy_base_url(
                    config,
                    GitlabRoutedBackend::Anthropic,
                )))
            }
            _ => Ok(Some(resolve_http_adapter_base_url(
                provider_id,
                auth,
                HttpAdapterKind::Anthropic,
            )?)),
        },
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

fn http_adapter_default_user_agent(
    auth: &ProviderAuthConfig,
    adapter: HttpAdapterKind,
    default_model: &str,
) -> String {
    credential_user_agent(auth, default_model).unwrap_or_else(|| match adapter {
        HttpAdapterKind::OpenAi => crate::provider::CODEX_USER_AGENT.to_owned(),
        HttpAdapterKind::Anthropic => crate::provider::CLAUDE_CODE_API_USER_AGENT.to_owned(),
        HttpAdapterKind::Gemini => crate::provider::gemini_cli_user_agent(default_model),
    })
}

fn credential_user_agent(auth: &ProviderAuthConfig, default_model: &str) -> Option<String> {
    let ProviderAuthConfig::Credential(config) = auth else {
        return None;
    };

    match config.issuer {
        crate::provider::auth::CredentialIssuer::AtomGit => {
            Some(crate::provider::ATOMCODE_USER_AGENT.to_owned())
        }
        crate::provider::auth::CredentialIssuer::OpenaiChatgpt => {
            Some(crate::provider::CODEX_USER_AGENT.to_owned())
        }
        crate::provider::auth::CredentialIssuer::GoogleAdc => {
            Some(crate::provider::gemini_cli_user_agent(default_model))
        }
        _ => None,
    }
}

fn resolve_http_adapter_base_url(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapter: HttpAdapterKind,
) -> Result<String, ConfigError> {
    let (base_url, protocol_paths) = provider_endpoint_root(auth, provider_id)?;
    let normalized = normalize_base_url(base_url)?;
    let protocol_path = http_adapter_protocol_path(protocol_paths, adapter);
    if protocol_path.is_empty() {
        Ok(normalized)
    } else {
        Ok(format!("{normalized}{protocol_path}"))
    }
}

fn atomgit_model_listing_url(configured: Option<String>) -> Option<String> {
    let configured = configured.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    });
    Some(configured.unwrap_or_else(|| ATOMGIT_CODING_PLAN_MODELS_URL.to_owned()))
}

fn atomgit_openai_api_mode(configured: OpenAiApiModeConfig, explicit: bool) -> OpenAiApiMode {
    if explicit {
        return configured.into();
    }

    // AtomCode sends AtomGit LLM traffic through the OpenAI-compatible
    // Chat Completions endpoint. Keep custom explicit overrides possible,
    // but make the credential-backed default match that gateway contract.
    OpenAiApiMode::Chat
}

fn provider_endpoint_root<'a>(
    auth: &'a ProviderAuthConfig,
    provider_id: &str,
) -> Result<(&'a str, &'a ProviderProtocolPathsConfig), ConfigError> {
    match auth {
        ProviderAuthConfig::Api(config) if config.base_url.is_some() => Ok((
            config
                .base_url
                .as_deref()
                .expect("guard ensures api base_url exists"),
            &config.protocol_paths,
        )),
        ProviderAuthConfig::Credential(config)
            if config.issuer.uses_http_endpoint() && config.base_url.is_some() =>
        {
            Ok((
                config
                    .base_url
                    .as_deref()
                    .expect("guard ensures credential base_url exists"),
                &config.protocol_paths,
            ))
        }
        ProviderAuthConfig::Credential(config)
            if config.issuer == crate::provider::auth::CredentialIssuer::AtomGit =>
        {
            Ok((ATOMGIT_GATEWAY_ROOT, &config.protocol_paths))
        }
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "provider auth does not define an api base_url".to_owned(),
        }),
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

fn http_adapter_protocol_path(
    protocol_paths: &ProviderProtocolPathsConfig,
    adapter: HttpAdapterKind,
) -> &str {
    match adapter {
        HttpAdapterKind::OpenAi => protocol_paths.openai.as_str(),
        HttpAdapterKind::Anthropic => protocol_paths.anthropic.as_str(),
        HttpAdapterKind::Gemini => protocol_paths.gemini.as_str(),
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
        ProviderAuthConfig::Credential(config)
            if config.issuer == crate::provider::auth::CredentialIssuer::GoogleAdc =>
        {
            if !matches!(
                capability_family,
                Some(ProviderCapabilityFamilyConfig::Gemini)
            ) {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "credential issuer `google_adc` only supports Vertex-style `openai` adapters"
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
        ProviderAuthConfig::Credential(config)
            if config.issuer == crate::provider::auth::CredentialIssuer::SapAiCore =>
        {
            Ok(ResolvedManagedCredential {
                credential: sap_ai_core_managed_credential(provider_id, client, config, env)?,
                auth_data: None,
            })
        }
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "openai adapter requires compatible api or credential auth".to_owned(),
        }),
    }
}

struct ResolvedManagedCredential {
    credential: ManagedCredential,
    auth_data: Option<Arc<Mutex<AuthData>>>,
}

fn gitlab_auth<'a>(
    auth: &'a ProviderAuthConfig,
    provider_id: &str,
) -> Result<&'a super::ProviderGitlabAuthConfig, ConfigError> {
    match auth {
        ProviderAuthConfig::Gitlab(config) => Ok(config),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "adapter requires gitlab auth".to_owned(),
        }),
    }
}

fn gitlab_instance_url(config: &super::ProviderGitlabAuthConfig) -> String {
    config
        .instance_url
        .clone()
        .unwrap_or_else(|| "https://gitlab.com".to_owned())
}

fn gitlab_ai_gateway_url(config: &super::ProviderGitlabAuthConfig) -> String {
    config
        .ai_gateway_url
        .clone()
        .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned())
}

fn gitlab_proxy_base_url(
    config: &super::ProviderGitlabAuthConfig,
    backend: GitlabRoutedBackend,
) -> String {
    let gateway = gitlab_ai_gateway_url(config);
    match backend {
        GitlabRoutedBackend::OpenAi => format!("{gateway}/ai/v1/proxy/openai/v1"),
        GitlabRoutedBackend::Anthropic => format!("{gateway}/ai/v1/proxy/anthropic/v1"),
    }
}

fn gitlab_runtime_config(
    config: &super::ProviderGitlabAuthConfig,
    default_model: &str,
) -> GitlabProviderConfig {
    let defaults = GitlabProviderConfig::default();
    GitlabProviderConfig {
        instance_url: gitlab_instance_url(config),
        ai_gateway_url: gitlab_ai_gateway_url(config),
        default_model: default_model.to_owned(),
        ai_gateway_headers: if config.ai_gateway_headers.is_empty() {
            defaults.ai_gateway_headers
        } else {
            to_hash_map(&config.ai_gateway_headers)
        },
        feature_flags: if config.feature_flags.is_empty() {
            defaults.feature_flags
        } else {
            to_hash_map(&config.feature_flags)
        },
    }
}

fn gitlab_auth_managed_credential(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let config = gitlab_auth(auth, provider_id)?;

    if let Some(value) = config
        .api_key
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()))
    {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::static_value(format!("{provider_id} api_key"), value),
            auth_data: None,
        });
    }

    if let Some(env_key) = config
        .api_key_env
        .as_ref()
        .and_then(|value| normalize_text(value.as_str()))
    {
        if env
            .var(env_key.as_str())
            .and_then(|value| normalize_text(&value))
            .is_some()
        {
            return Ok(ResolvedManagedCredential {
                credential: ManagedCredential::environment(
                    format!("{provider_id} api_key"),
                    provider_id.to_owned(),
                    "api_key",
                    env_key,
                ),
                auth_data: None,
            });
        }

        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::environment(
                format!("{provider_id} api_key"),
                provider_id.to_owned(),
                "api_key",
                env_key,
            ),
            auth_data: None,
        });
    }

    if let Some(auth_data) = config.credential.clone() {
        let auth_data = Arc::new(Mutex::new(auth_data));
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::auth_data_shared(
                format!("{provider_id} api_key"),
                provider_id.to_owned(),
                auth_data.clone(),
                AuthSecretSelector::AccessOrApiKey,
                AuthRefreshStrategy::GitlabOAuth {
                    instance_url: gitlab_instance_url(config),
                },
            ),
            auth_data: Some(auth_data),
        });
    }

    Err(ConfigError::MissingProviderField {
        provider_id: provider_id.to_owned(),
        field: "api_key",
    })
}

fn gitlab_credential_instance_url(config: &ProviderCredentialAuthConfig) -> String {
    config
        .instance_url
        .clone()
        .unwrap_or_else(|| "https://gitlab.com".to_owned())
}

fn gitlab_credential_ai_gateway_url(config: &ProviderCredentialAuthConfig) -> String {
    config
        .ai_gateway_url
        .clone()
        .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned())
}

fn gitlab_credential_proxy_base_url(
    config: &ProviderCredentialAuthConfig,
    backend: GitlabRoutedBackend,
) -> String {
    let gateway = gitlab_credential_ai_gateway_url(config);
    match backend {
        GitlabRoutedBackend::OpenAi => format!("{gateway}/ai/v1/proxy/openai/v1"),
        GitlabRoutedBackend::Anthropic => format!("{gateway}/ai/v1/proxy/anthropic/v1"),
    }
}

fn gitlab_credential_runtime_config(
    config: &ProviderCredentialAuthConfig,
    default_model: &str,
) -> GitlabProviderConfig {
    let defaults = GitlabProviderConfig::default();
    GitlabProviderConfig {
        instance_url: gitlab_credential_instance_url(config),
        ai_gateway_url: gitlab_credential_ai_gateway_url(config),
        default_model: default_model.to_owned(),
        ai_gateway_headers: if config.ai_gateway_headers.is_empty() {
            defaults.ai_gateway_headers
        } else {
            to_hash_map(&config.ai_gateway_headers)
        },
        feature_flags: if config.feature_flags.is_empty() {
            defaults.feature_flags
        } else {
            to_hash_map(&config.feature_flags)
        },
    }
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

    if let Some(env_key) = env_key.as_ref()
        && env
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
    config: &ProviderCredentialAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<ManagedCredential, ConfigError> {
    let service_key_env =
        config
            .service_key_env
            .as_deref()
            .ok_or_else(|| ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: "credential issuer `sap_ai_core` requires `service_key_env`".to_owned(),
            })?;
    let service_key_raw = env
        .var(service_key_env)
        .and_then(|value| normalize_text(&value))
        .ok_or_else(|| ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("sap-ai-core requires `{service_key_env}`"),
        })?;
    let service_key = parse_sap_ai_core_service_key(service_key_raw.as_str()).map_err(|err| {
        ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("failed to parse `{service_key_env}`: {err}"),
        }
    })?;
    Ok(ManagedCredential::sap_ai_core(
        format!("{provider_id} sap ai core token"),
        client.clone(),
        provider_id.to_owned(),
        service_key,
    ))
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

fn http_adapter_extra_headers<T>(
    adapter: &HttpProviderAdapterConfig<T>,
    default_user_agent: Option<String>,
) -> HashMap<String, String> {
    let mut headers = to_hash_map(&adapter.extra_headers);
    if let Some(user_agent) = adapter.user_agent.as_deref().and_then(normalize_text) {
        set_user_agent_header(&mut headers, user_agent);
    } else if !has_user_agent_header(&headers)
        && let Some(user_agent) = default_user_agent.as_deref().and_then(normalize_text)
    {
        set_user_agent_header(&mut headers, user_agent);
    }
    headers
}

fn has_user_agent_header(headers: &HashMap<String, String>) -> bool {
    headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
}

fn set_user_agent_header(headers: &mut HashMap<String, String>, user_agent: String) {
    if let Some(existing) = headers
        .keys()
        .find(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
        .cloned()
    {
        headers.remove(&existing);
    }
    headers.insert(reqwest::header::USER_AGENT.as_str().to_owned(), user_agent);
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
