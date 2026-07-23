use std::{collections::HashMap, path::Path, sync::Arc};

use agena_storage::MemoryStore;
use tokio_util::sync::CancellationToken;

use crate::{
    AppError,
    agent::Agent,
    config::{ConfigLoader, LoadConfigRequest, ProcessEnvironment},
    permission::{PermissionPolicy, ToolPermissionPolicy},
    session::SessionManager,
    tool::ToolExecutor,
};

use agena_runtime::{
    ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig, RuntimeBackgroundTask,
    RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOrigin,
    RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskSpec, RuntimeBackgroundTaskStart,
    RuntimeCompositionConfig, RuntimeControlState, RuntimeReloadCause, RuntimeReloadReport,
    TaskControl, TuiColorSchemeConfig, TuiGraphicsModeConfig,
};

use super::{RuntimeSnapshot, reload};

/// Compose the concrete Runtime from a schema-neutral process request and
/// return its stable application capability bundle.
pub async fn bootstrap_application_services(
    request: agena_runtime::RuntimeBootstrapRequest,
) -> Result<agena_runtime::RuntimeBootstrapResult, agena_runtime::RuntimeBootstrapError> {
    agena_runtime::compose_runtime_bootstrap(request, |config| async move {
        let runtime = AgenaRuntime::new(config)
            .await
            .map_err(runtime_bootstrap_error)?;
        Ok(agena_runtime::RuntimeBootstrapComposition::new(
            runtime.application_services(),
            Arc::new(runtime),
        ))
    })
    .await
}

impl AgenaRuntime {
    /// Runtime-private composition entrypoint. External consumers use the
    /// stable `bootstrap_application_services` capability result instead.
    pub(crate) async fn new(config: RuntimeCompositionConfig) -> Result<Self, AppError> {
        let mut config = config;
        let workspace_root = config.resolve_workspace_root()?;
        let RuntimeCompositionConfig {
            load_request,
            database_connection,
            database_url,
            database_path,
            initialize_schema,
            tracing_reload_handle,
            bootstrap_preflight,
            workspace_root: _,
        } = config;
        let loader = ConfigLoader::new(ProcessEnvironment);
        let initial_resolution = bootstrap_preflight
            .is_none()
            .then(|| loader.load(&load_request));
        let initial_resolution = match initial_resolution {
            Some(result) => Some(result?),
            None => None,
        };
        let tracing = match (&bootstrap_preflight, &initial_resolution) {
            (Some(preflight), _) => &preflight.tracing,
            (None, Some(resolution)) => &resolution.config.tracing,
            (None, None) => {
                return Err(AppError::Internal(
                    "runtime bootstrap tracing preflight was not resolved".to_owned(),
                ));
            }
        };
        let database =
            agena_runtime::connect_runtime_database(agena_runtime::DatabaseCompositionInputs {
                database_connection,
                database_url,
                database_path,
                initialize_schema,
                tracing,
            })
            .await
            .map_err(runtime_database_error)?;
        let initial_snapshot = Arc::new(
            RuntimeSnapshot::build(
                1,
                &loader,
                &load_request,
                workspace_root.as_path(),
                database.clone(),
                None,
            )
            .await?,
        );

        let runtime = AgenaRuntime {
            inner: Arc::new(agena_runtime::RuntimeProcessState::new(
                loader,
                load_request,
                workspace_root,
                database,
                RuntimeControlState::new(initial_snapshot.clone(), tracing_reload_handle),
            )),
        };

        // Install the runtime-backed HostClient into the plugin host so
        // plugin → host callbacks (log/read_config/etc.) actually do work.
        {
            let host_handle = initial_snapshot.plugin_manager().host_handle();
            let client = super::host_client::host_client_for(runtime.clone());
            agena_runtime::install_plugin_host_client(Arc::clone(&host_handle), client).await;
            super::host_client::install_plugin_host_event_publisher(host_handle, runtime.clone());
        }

        runtime.apply_tracing_filter(initial_snapshot.tracing_config());
        runtime.spawn_background_tasks();
        Ok(runtime)
    }
}

#[derive(Clone, Copy)]
enum AuthOAuthPurpose {
    BrowserLogin,
    CredentialRefresh,
    DeviceLogin,
}

fn auth_bad_request(message: impl Into<String>) -> agena_runtime::RuntimeAuthenticationError {
    agena_runtime::RuntimeAuthenticationError::bad_request(message)
}

fn auth_internal(error: impl std::fmt::Display) -> agena_runtime::RuntimeAuthenticationError {
    agena_runtime::RuntimeAuthenticationError::internal(error.to_string())
}

fn auth_manager_for_runtime(
    runtime: &AgenaRuntime,
) -> crate::provider::auth::AuthManager<crate::config::ProviderConfigCredentialStore> {
    crate::provider::auth::AuthManager::new(crate::config::ProviderConfigCredentialStore::new(
        runtime.current_snapshot().config_path().to_path_buf(),
    ))
}

fn auth_provider_is_configured(resolved: &agena_runtime::ResolvedProviderConfig) -> bool {
    match &resolved.auth {
        ProviderAuthConfig::None => false,
        ProviderAuthConfig::Api(api) => api.bedrock_sigv4().is_none(),
        ProviderAuthConfig::Credential(config) => !matches!(
            config.issuer(),
            agena_provider::CredentialIssuer::GoogleAdc
                | agena_provider::CredentialIssuer::SapAiCore
        ),
    }
}

fn auth_resolved_provider(
    runtime: &AgenaRuntime,
    provider_id: &str,
) -> Result<agena_runtime::ResolvedProviderConfig, agena_runtime::RuntimeAuthenticationError> {
    runtime
        .current_snapshot()
        .provider_configs()
        .get(provider_id)
        .cloned()
        .filter(auth_provider_is_configured)
        .ok_or_else(|| {
            agena_runtime::RuntimeAuthenticationError::not_found(format!(
                "auth provider not found: {provider_id}"
            ))
        })
}

fn auth_provider_projection(
    provider_id: &str,
    resolved: &agena_runtime::ResolvedProviderConfig,
) -> Result<agena_runtime::RuntimeAuthProvider, agena_runtime::RuntimeAuthenticationError> {
    let auth = crate::config::provider_auth_data(resolved);
    let credential_present = auth.is_some();
    let (
        credential_type,
        credential_issuer,
        key_preview,
        expires_at,
        account_id,
        enterprise_url,
        username,
        display_name,
        email,
        avatar_url,
    ) = match auth {
        Some(agena_provider::AuthData::Api { key }) => (
            Some(agena_runtime::RuntimeAuthCredentialType::Api),
            None,
            auth_secret_preview(&key),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        Some(agena_provider::AuthData::OAuth {
            issuer,
            expires_at_ms,
            account_id,
            enterprise_url,
            user,
            ..
        }) => {
            let expires_at = if expires_at_ms > 0 {
                Some(
                    chrono::DateTime::from_timestamp_millis(expires_at_ms).ok_or_else(|| {
                        auth_internal(format!("invalid oauth expiry millis: {expires_at_ms}"))
                    })?,
                )
            } else {
                None
            };
            let (account_id, username, display_name, email, avatar_url) = match user {
                Some(user) => (
                    account_id.or(Some(user.id)),
                    Some(user.username),
                    user.name,
                    user.email,
                    user.avatar_url,
                ),
                None => (account_id, None, None, None, None),
            };
            (
                Some(agena_runtime::RuntimeAuthCredentialType::Oauth),
                issuer.map(auth_issuer),
                None,
                expires_at,
                account_id,
                enterprise_url,
                username,
                display_name,
                email,
                avatar_url,
            )
        }
        Some(agena_provider::AuthData::WellKnown { key, .. }) => (
            Some(agena_runtime::RuntimeAuthCredentialType::WellKnown),
            None,
            Some(key),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        None => (None, None, None, None, None, None, None, None, None, None),
    };
    let (browser_login_kind, browser_login_instance_url) =
        match crate::config::resolve_provider_oauth_target(resolved) {
            Ok(Some(crate::config::ProviderOAuthTarget::OpenAi)) => (
                Some(agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt),
                None,
            ),
            Ok(Some(crate::config::ProviderOAuthTarget::Gitlab { instance_url })) => (
                Some(agena_runtime::RuntimeAuthLoginKind::Gitlab),
                Some(instance_url),
            ),
            Err(crate::config::ProviderAuthTargetError::AmbiguousGitlab) => {
                (None, crate::config::provider_gitlab_instance_url(resolved))
            }
            Ok(None) | Err(crate::config::ProviderAuthTargetError::AmbiguousProvider) => {
                (None, None)
            }
        };
    let device_login_kind = match crate::config::resolve_provider_device_auth_target(resolved) {
        Ok(Some(crate::config::ProviderDeviceAuthTarget::OpenAi)) => {
            Some(agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt)
        }
        Ok(Some(crate::config::ProviderDeviceAuthTarget::Copilot)) => {
            Some(agena_runtime::RuntimeAuthLoginKind::GithubCopilot)
        }
        Ok(None) | Err(_) => None,
    };
    Ok(agena_runtime::RuntimeAuthProvider {
        provider_id: provider_id.to_owned(),
        credential_present,
        credential_type,
        credential_issuer,
        key_preview,
        expires_at,
        account_id,
        enterprise_url,
        username,
        display_name,
        email,
        avatar_url,
        api_key_write_supported: crate::config::provider_supports_api_key_write(resolved),
        browser_login_kind,
        browser_login_instance_url,
        device_login_kind,
    })
}

fn auth_issuer(
    issuer: agena_provider::CredentialIssuer,
) -> agena_runtime::RuntimeAuthCredentialIssuer {
    match issuer {
        agena_provider::CredentialIssuer::OpenaiChatgpt => {
            agena_runtime::RuntimeAuthCredentialIssuer::OpenaiChatgpt
        }
        agena_provider::CredentialIssuer::GithubCopilot => {
            agena_runtime::RuntimeAuthCredentialIssuer::GithubCopilot
        }
        agena_provider::CredentialIssuer::Gitlab => {
            agena_runtime::RuntimeAuthCredentialIssuer::Gitlab
        }
        agena_provider::CredentialIssuer::GoogleAdc => {
            agena_runtime::RuntimeAuthCredentialIssuer::GoogleAdc
        }
        agena_provider::CredentialIssuer::SapAiCore => {
            agena_runtime::RuntimeAuthCredentialIssuer::SapAiCore
        }
    }
}

fn auth_secret_preview(secret: &str) -> Option<String> {
    let value = secret.trim();
    (!value.is_empty()).then(|| {
        if value.len() <= 8 {
            "*".repeat(value.len())
        } else {
            format!("{}...{}", &value[..4], &value[value.len() - 4..])
        }
    })
}

fn auth_oauth_target(
    runtime: &AgenaRuntime,
    provider_id: &str,
    purpose: AuthOAuthPurpose,
) -> Result<crate::config::ProviderOAuthTarget, agena_runtime::RuntimeAuthenticationError> {
    let resolved = auth_resolved_provider(runtime, provider_id)?;
    crate::config::resolve_provider_oauth_target(&resolved)
        .map_err(|error| auth_target_error(provider_id, purpose, error))?
        .ok_or_else(|| match purpose {
            AuthOAuthPurpose::BrowserLogin => {
                auth_bad_request(format!("{provider_id} does not support browser login"))
            }
            AuthOAuthPurpose::CredentialRefresh => auth_bad_request(format!(
                "credential refresh is not supported for provider '{provider_id}'"
            )),
            AuthOAuthPurpose::DeviceLogin => {
                auth_bad_request(format!("{provider_id} does not support device login"))
            }
        })
}

fn auth_device_target(
    runtime: &AgenaRuntime,
    provider_id: &str,
) -> Result<crate::config::ProviderDeviceAuthTarget, agena_runtime::RuntimeAuthenticationError> {
    let resolved = auth_resolved_provider(runtime, provider_id)?;
    crate::config::resolve_provider_device_auth_target(&resolved)
        .map_err(|error| auth_target_error(provider_id, AuthOAuthPurpose::DeviceLogin, error))?
        .ok_or_else(|| auth_bad_request(format!("{provider_id} does not support device login")))
}

fn auth_target_error(
    provider_id: &str,
    purpose: AuthOAuthPurpose,
    error: crate::config::ProviderAuthTargetError,
) -> agena_runtime::RuntimeAuthenticationError {
    let text = match (purpose, error) {
        (
            AuthOAuthPurpose::BrowserLogin,
            crate::config::ProviderAuthTargetError::AmbiguousProvider,
        ) => format!("{provider_id} has ambiguous browser auth providers"),
        (
            AuthOAuthPurpose::BrowserLogin,
            crate::config::ProviderAuthTargetError::AmbiguousGitlab,
        ) => format!("{provider_id} has ambiguous gitlab browser auth adapters"),
        (
            AuthOAuthPurpose::CredentialRefresh,
            crate::config::ProviderAuthTargetError::AmbiguousProvider,
        ) => format!("{provider_id} has ambiguous credential refresh handlers"),
        (
            AuthOAuthPurpose::CredentialRefresh,
            crate::config::ProviderAuthTargetError::AmbiguousGitlab,
        ) => format!("{provider_id} has ambiguous gitlab refresh adapters"),
        (
            AuthOAuthPurpose::DeviceLogin,
            crate::config::ProviderAuthTargetError::AmbiguousProvider,
        ) => format!("{provider_id} has ambiguous device auth providers"),
        (
            AuthOAuthPurpose::DeviceLogin,
            crate::config::ProviderAuthTargetError::AmbiguousGitlab,
        ) => format!("{provider_id} has ambiguous device auth providers"),
    };
    auth_bad_request(text)
}

fn auth_kind_name(kind: agena_runtime::RuntimeAuthLoginKind) -> &'static str {
    match kind {
        agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt => "openai",
        agena_runtime::RuntimeAuthLoginKind::GithubCopilot => "copilot",
        agena_runtime::RuntimeAuthLoginKind::Gitlab => "gitlab",
    }
}

fn auth_copilot_deployment(domain: Option<String>) -> agena_provider::CopilotDeployment {
    match domain
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        Some(domain) => agena_provider::CopilotDeployment::Enterprise { domain },
        None => agena_provider::CopilotDeployment::GitHubCom,
    }
}

#[async_trait::async_trait]
impl agena_provider::ProviderCatalog for AgenaRuntime {
    fn list_providers(&self) -> Vec<agena_provider::ProviderCatalogEntry> {
        let snapshot = self.current_snapshot();
        let registry = snapshot.provider_registry();
        let mut providers = registry
            .provider_ids()
            .into_iter()
            .filter_map(|provider_id| {
                registry.get(provider_id.as_ref()).map(|provider| {
                    let provider_config = snapshot.provider_configs().get(provider_id.as_str());
                    let adapters = provider_config
                        .map(|provider_config| {
                            provider_config
                                .adapters
                                .iter()
                                .map(|(adapter_id, adapter)| {
                                    agena_provider::ProviderAdapterSummary {
                                        adapter_id: adapter_id.clone(),
                                        enabled: adapter.enabled,
                                        configured_model_count: provider_config
                                            .models
                                            .keys()
                                            .filter(|model_id| {
                                                model_id
                                                    .split_once('/')
                                                    .map(|(route_adapter_id, _)| {
                                                        route_adapter_id == adapter_id
                                                    })
                                                    .unwrap_or(false)
                                            })
                                            .count(),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let provider_native_tools = provider_config.map(|provider_config| {
                        let (active, bindings) = provider_config
                            .defaults
                            .adapter
                            .as_ref()
                            .zip(provider_config.defaults.model.as_ref())
                            .and_then(|(adapter_id, model_id)| {
                                provider_config
                                    .models
                                    .get(format!("{adapter_id}/{model_id}").as_str())
                            })
                            .map(|model| {
                                let bindings = model.provider_native_tool_bindings();
                                (!bindings.is_empty(), bindings)
                            })
                            .unwrap_or((false, Vec::new()));
                        agena_provider::ProviderNativeToolsSummary {
                            active,
                            model_count: provider_config
                                .models
                                .values()
                                .filter(|model| !model.provider_native_tool_bindings().is_empty())
                                .count(),
                            bindings: bindings
                                .into_iter()
                                .map(|binding| agena_provider::ProviderNativeToolBindingSummary {
                                    tool: binding.tool.config_key().to_owned(),
                                    route: serde_json::to_string(&binding.route)
                                        .unwrap_or_default()
                                        .trim_matches('"')
                                        .to_owned(),
                                })
                                .collect(),
                        }
                    });
                    agena_provider::ProviderCatalogEntry {
                        provider_id: agena_domain::ProviderId::new(provider_id),
                        defaults: agena_provider::ProviderDefaults {
                            adapter: provider.default_adapter().map(ToString::to_string),
                            model: provider.default_model().to_string(),
                            thinking_mode: provider_config.and_then(|provider_config| {
                                provider_config.defaults.thinking_mode.clone()
                            }),
                            speed_mode: provider_config.and_then(|provider_config| {
                                provider_config.defaults.speed_mode.clone()
                            }),
                            verbosity: provider_config.and_then(|provider_config| {
                                provider_config.defaults.verbosity.clone()
                            }),
                            parallel_tool_calls: provider_config.and_then(|provider_config| {
                                provider_config.defaults.parallel_tool_calls
                            }),
                        },
                        adapters,
                        provider_native_tools,
                    }
                })
            })
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        providers
    }

    fn contains_provider(&self, provider_id: &agena_domain::ProviderId) -> bool {
        let provider_key: &str = provider_id.as_ref();
        self.current_snapshot()
            .provider_registry()
            .get(provider_key)
            .is_some()
    }

    fn configured_routing(
        &self,
        provider_id: &agena_domain::ProviderId,
    ) -> Option<agena_provider::ProviderConfiguredRouting> {
        let provider_key: &str = provider_id.as_ref();
        let snapshot = self.current_snapshot();
        let provider = snapshot.provider_configs().get(provider_key)?;
        let mut adapters = provider
            .adapters
            .iter()
            .map(|(adapter_id, adapter)| {
                let mut model_ids = provider
                    .models
                    .keys()
                    .filter_map(|route| {
                        route
                            .split_once('/')
                            .filter(|(route_adapter_id, _)| *route_adapter_id == adapter_id)
                            .map(|(_, model_id)| model_id.to_owned())
                    })
                    .collect::<Vec<_>>();
                model_ids.sort();
                agena_provider::ProviderConfiguredAdapterModels {
                    adapter_id: adapter_id.clone(),
                    enabled: adapter.enabled,
                    model_ids,
                }
            })
            .collect::<Vec<_>>();
        adapters.sort_by(|left, right| left.adapter_id.cmp(&right.adapter_id));
        Some(agena_provider::ProviderConfiguredRouting {
            provider_id: provider_id.clone(),
            adapters,
        })
    }

    fn configured_editor(
        &self,
        provider_id: &agena_domain::ProviderId,
    ) -> Option<agena_provider::ProviderConfiguredEditor> {
        fn api_key_source(
            source: Option<&agena_provider::ProviderSecretSourceConfig>,
        ) -> Option<agena_provider::ProviderApiKeySource> {
            match source? {
                agena_provider::ProviderSecretSourceConfig::Inline(value) => {
                    Some(agena_provider::ProviderApiKeySource::Inline(value.clone()))
                }
                agena_provider::ProviderSecretSourceConfig::Env(value) => Some(
                    agena_provider::ProviderApiKeySource::Environment(value.clone()),
                ),
            }
        }

        let provider_key: &str = provider_id.as_ref();
        let snapshot = self.current_snapshot();
        let provider = snapshot.provider_configs().get(provider_key)?;
        let auth = match &provider.auth {
            ProviderAuthConfig::None => agena_provider::ProviderConfiguredEditorAuth::None,
            ProviderAuthConfig::Api(api) => match api {
                ProviderApiAuthConfig::Custom { .. } => {
                    agena_provider::ProviderConfiguredEditorAuth::Api {
                        base_url: api.custom_base_url().unwrap_or_default().to_owned(),
                        api_key: api_key_source(api.api_key_source()),
                    }
                }
                ProviderApiAuthConfig::ClineApi { api_key } => {
                    agena_provider::ProviderConfiguredEditorAuth::ClineApi {
                        api_key: api_key_source(api_key.as_ref()),
                    }
                }
                ProviderApiAuthConfig::Gitlab {
                    access,
                    instance_url,
                    ..
                } => agena_provider::ProviderConfiguredEditorAuth::Gitlab {
                    api_key: api_key_source(access.api_key_source()),
                    instance_url: instance_url.clone(),
                },
                ProviderApiAuthConfig::BedrockSigv4 {
                    base_url,
                    region,
                    profile,
                    access_key_id,
                    secret_access_key,
                    session_token,
                } => agena_provider::ProviderConfiguredEditorAuth::BedrockSigv4 {
                    base_url: base_url.clone(),
                    region: region.clone(),
                    profile: profile.clone(),
                    access_key_id: access_key_id.clone(),
                    secret_access_key: secret_access_key.clone(),
                    session_token: session_token.clone(),
                },
            },
            ProviderAuthConfig::Credential(config) => {
                agena_provider::ProviderConfiguredEditorAuth::Credential {
                    issuer: config.issuer(),
                    credential: config.credential().cloned(),
                    base_url: config.base_url().map(ToOwned::to_owned),
                    instance_url: config
                        .gitlab()
                        .and_then(|gitlab| gitlab.instance_url.clone()),
                    service_key_env: config.service_key_env().map(ToOwned::to_owned),
                }
            }
        };
        Some(agena_provider::ProviderConfiguredEditor {
            provider_id: provider_key.to_owned(),
            auth,
            default_adapter: provider.defaults.adapter.clone(),
            default_model: provider.defaults.model.clone(),
            request_timeout_secs: provider.network.request_timeout_secs,
            connect_timeout_secs: provider.network.connect_timeout_secs,
        })
    }

    fn configured_local_models(
        &self,
        provider_id: &agena_domain::ProviderId,
    ) -> Result<Vec<agena_domain::Model>, agena_provider::ProviderCatalogError> {
        self.current_snapshot()
            .configured_local_models(provider_id.as_ref())
            .map_err(|error| agena_provider::ProviderCatalogError::Operation(error.to_string()))
    }

    fn default_model(
        &self,
    ) -> Result<Option<agena_domain::ModelRef>, agena_provider::ProviderCatalogError> {
        self.current_snapshot()
            .resolve_default_model()
            .map_err(|error| agena_provider::ProviderCatalogError::Operation(error.to_string()))
    }

    fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<agena_domain::ModelRef, agena_provider::ProviderCatalogError> {
        self.current_snapshot()
            .resolve_model_target(target, model)
            .map_err(|error| agena_provider::ProviderCatalogError::Operation(error.to_string()))
    }

    fn model_execution_options(
        &self,
        model: &agena_domain::ModelRef,
    ) -> Result<agena_provider::ProviderModelExecutionOptions, agena_provider::ProviderCatalogError>
    {
        let snapshot = self.current_snapshot();
        let registry = snapshot.provider_registry();
        let provider = registry.get(model.provider_id.as_ref()).ok_or_else(|| {
            agena_provider::ProviderCatalogError::NotFound(model.provider_id.to_string())
        })?;
        Ok(agena_provider::ProviderModelExecutionOptions {
            default_adapter: provider.default_adapter().cloned(),
            capabilities: registry.model_capabilities(model).map_err(|error| {
                agena_provider::ProviderCatalogError::Operation(error.to_string())
            })?,
            thinking_modes: registry.model_thinking_modes(model).map_err(|error| {
                agena_provider::ProviderCatalogError::Operation(error.to_string())
            })?,
            speed_modes: registry.model_speed_modes(model).map_err(|error| {
                agena_provider::ProviderCatalogError::Operation(error.to_string())
            })?,
            metadata: registry.model_metadata(model).map_err(|error| {
                agena_provider::ProviderCatalogError::Operation(error.to_string())
            })?,
        })
    }

    async fn list_models(
        &self,
        provider_id: &agena_domain::ProviderId,
    ) -> Result<Vec<agena_domain::Model>, agena_provider::ProviderCatalogError> {
        self.current_snapshot()
            .list_provider_models(provider_id.as_ref())
            .await
            .map_err(|error| agena_provider::ProviderCatalogError::Operation(error.to_string()))
    }

    async fn list_draft_adapter_models(
        &self,
        request: agena_provider::DraftProviderAdapterModelsRequest,
    ) -> Result<agena_provider::ProviderAdapterModelsListing, agena_provider::ProviderCatalogError>
    {
        fn api_key_parts(
            api_key: &Option<agena_provider::ProviderApiKeySource>,
        ) -> (Option<&str>, Option<&str>) {
            match api_key {
                Some(agena_provider::ProviderApiKeySource::Inline(value)) => {
                    (Some(value.as_str()), None)
                }
                Some(agena_provider::ProviderApiKeySource::Environment(value)) => {
                    (None, Some(value.as_str()))
                }
                None => (None, None),
            }
        }
        let protocol_paths = |paths: agena_provider::ProviderProtocolPaths| {
            agena_provider::ProviderProtocolPathsConfig {
                openai: paths.openai,
                anthropic: paths.anthropic,
                gemini: paths.gemini,
            }
        };
        let target = match request {
            agena_provider::DraftProviderAdapterModelsRequest::Http(request) => {
                let (api_key, api_key_env) = api_key_parts(&request.api_key);
                crate::config::draft_provider_adapter_models_target(
                    request.provider_id.as_deref(),
                    request.base_url.as_str(),
                    protocol_paths(request.protocol_paths),
                    api_key,
                    api_key_env,
                    request.adapter_ids.as_slice(),
                )
            }
            agena_provider::DraftProviderAdapterModelsRequest::None {
                provider_id,
                adapter_ids,
            } => crate::config::draft_none_provider_adapter_models_target(
                provider_id.as_deref(),
                adapter_ids.as_slice(),
            ),
            agena_provider::DraftProviderAdapterModelsRequest::ClineApi {
                provider_id,
                api_key,
                adapter_ids,
                models_url,
            } => {
                let (api_key, api_key_env) = api_key_parts(&api_key);
                crate::config::draft_cline_api_provider_adapter_models_target(
                    provider_id.as_deref(),
                    api_key,
                    api_key_env,
                    adapter_ids.as_slice(),
                )
                .map(|mut target| {
                    if let Some(models_url) = models_url
                        && let Some(adapter) = target.adapters.get_mut("openai_chat_completions")
                        && let ProviderAdapterDefinition::OpenAiChatCompletions(config) =
                            &mut adapter.definition
                    {
                        config.options.models_url = Some(models_url);
                    }
                    target
                })
            }
            agena_provider::DraftProviderAdapterModelsRequest::Gitlab {
                provider_id,
                api_key,
                adapter_ids,
            } => {
                let (api_key, api_key_env) = api_key_parts(&api_key);
                crate::config::draft_gitlab_provider_adapter_models_target(
                    provider_id.as_deref(),
                    api_key,
                    api_key_env,
                    adapter_ids.as_slice(),
                )
            }
            agena_provider::DraftProviderAdapterModelsRequest::Credential {
                provider_id,
                issuer,
                credential,
                base_url,
                protocol_paths: paths,
                service_key_env,
                instance_url,
                adapter_ids,
            } => crate::config::draft_credential_provider_adapter_models_target(
                provider_id.as_deref(),
                issuer,
                credential.map(|credential| *credential),
                base_url.as_deref(),
                protocol_paths(paths),
                service_key_env.as_deref(),
                instance_url.as_deref(),
                adapter_ids.as_slice(),
            ),
            agena_provider::DraftProviderAdapterModelsRequest::BedrockSigv4 {
                provider_id,
                base_url,
                region,
                profile,
                access_key_id,
                secret_access_key,
                session_token,
                adapter_ids,
            } => crate::config::draft_bedrock_sigv4_provider_adapter_models_target(
                provider_id.as_deref(),
                base_url.as_deref(),
                region.as_deref(),
                profile.as_deref(),
                access_key_id.as_deref(),
                secret_access_key.as_deref(),
                session_token.as_deref(),
                adapter_ids.as_slice(),
            ),
        }
        .map_err(|error| agena_provider::ProviderCatalogError::InvalidRequest(error.to_string()))?;
        self.list_adapter_models_target(target).await
    }

    async fn list_saved_adapter_models(
        &self,
        provider_id: &agena_domain::ProviderId,
        adapter_ids: Vec<String>,
    ) -> Result<agena_provider::ProviderAdapterModelsListing, agena_provider::ProviderCatalogError>
    {
        let provider_key: &str = provider_id.as_ref();
        let snapshot = self.current_snapshot();
        let resolved = snapshot
            .provider_configs()
            .get(provider_key)
            .ok_or_else(|| {
                agena_provider::ProviderCatalogError::NotFound(provider_id.to_string())
            })?;
        let target = crate::config::saved_provider_adapter_models_target(
            provider_key,
            resolved,
            adapter_ids.as_slice(),
        )
        .map_err(|error| agena_provider::ProviderCatalogError::InvalidRequest(error.to_string()))?;
        self.list_adapter_models_target(target).await
    }
}

impl agena_runtime::ModelCatalogRuntimeService for AgenaRuntime {
    fn model_catalog_response(&self) -> agena_provider::ModelCatalogResponse {
        self.current_snapshot().model_catalog_response()
    }

    fn model_catalog_refresh_active(&self) -> bool {
        AgenaRuntime::model_catalog_refresh_active(self)
    }

    fn start_model_catalog_refresh(
        &self,
        origin: agena_runtime::RuntimeBackgroundTaskOrigin,
    ) -> Result<agena_runtime::RuntimeBackgroundTaskStart, agena_runtime::ModelCatalogRefreshError>
    {
        AgenaRuntime::start_model_catalog_refresh(self, origin)
            .map_err(|error| agena_runtime::ModelCatalogRefreshError::new(error.to_string()))
    }
}

#[async_trait::async_trait]
impl agena_runtime::PluginRuntimeService for AgenaRuntime {
    fn plugin_statuses(&self) -> Vec<agena_plugin_host::status::PluginStatus> {
        self.current_snapshot().plugin_manager().plugin_statuses()
    }

    fn plugin_status(&self, plugin_id: &str) -> Option<agena_plugin_host::status::PluginStatus> {
        self.current_snapshot()
            .plugin_manager()
            .plugin_status(plugin_id)
    }

    fn plugin_ui_catalog(&self) -> agena_plugin_host::PluginUiCatalog {
        self.current_snapshot().plugin_manager().ui_catalog()
    }

    fn permission_tool_catalog(&self) -> Vec<agena_runtime::RuntimePluginToolCatalogItem> {
        let mut tools = self
            .current_snapshot()
            .plugin_manager()
            .registered_tools()
            .into_iter()
            .map(|tool| {
                let mut tags = tool
                    .effective_tags()
                    .into_iter()
                    .map(|tag| tag.as_ref().to_string())
                    .collect::<Vec<_>>();
                tags.sort();
                tags.dedup();
                agena_runtime::RuntimePluginToolCatalogItem {
                    name: tool.canonical_name(),
                    summary: tool.summary_text().unwrap_or_default().to_string(),
                    tags,
                }
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools.dedup_by(|left, right| left.name == right.name);
        tools
    }

    fn statusline_segments(&self) -> Vec<agena_plugin_host::HostStatuslineSegment> {
        self.current_snapshot()
            .plugin_manager()
            .statusline_segments()
    }

    fn tui_content_blocks(&self) -> Vec<agena_plugin_host::PluginTuiContentBlockCatalogItem> {
        self.current_snapshot()
            .plugin_manager()
            .tui_content_blocks()
    }

    fn theme_palettes(&self) -> Vec<agena_plugin_host::HostThemePalette> {
        self.current_snapshot().plugin_manager().theme_palettes()
    }

    fn studio_commands(&self) -> Vec<agena_plugin_host::PluginCommandCatalogItem> {
        self.current_snapshot().plugin_manager().studio_commands()
    }

    fn tool_registry_generation(&self) -> u64 {
        self.current_snapshot()
            .plugin_manager()
            .tool_registry_generation()
    }

    fn tool_registry_events_since(
        &self,
        after_generation: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent> {
        self.current_snapshot()
            .plugin_manager()
            .tool_registry_events_since(after_generation, limit)
    }

    fn plugin_inspect(&self, plugin_id: &str) -> Option<agena_plugin_host::PluginInspect> {
        self.current_snapshot()
            .plugin_manager()
            .plugin_inspect(plugin_id)
    }

    fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::PluginLogRecord> {
        self.current_snapshot()
            .plugin_manager()
            .plugin_logs(plugin_id, after_seq, limit)
    }

    fn resolve_studio_action(
        &self,
        plugin_id: &str,
        action_id: &str,
    ) -> Option<agena_plugin_host::PluginUiAction> {
        self.current_snapshot()
            .plugin_manager()
            .resolve_studio_action(plugin_id, action_id)
    }

    fn resolve_plugin_tool(
        &self,
        plugin_id: Option<&str>,
        tool_name: &str,
    ) -> Option<agena_runtime::PluginToolDescriptor> {
        let host = self.current_snapshot().plugin_manager();
        let entry = match plugin_id {
            Some(plugin_id) => host.resolve_registered_tool_for_plugin_tool(plugin_id, tool_name),
            None => host.lookup_tool(tool_name),
        }?;
        Some(agena_runtime::PluginToolDescriptor {
            canonical_name: entry.canonical_name(),
            plugin_full_name: entry.plugin_full_name(),
            plugin_id: entry.plugin_key().clone(),
        })
    }

    async fn invoke_plugin_command(
        &self,
        plugin_id: &str,
        input: agena_plugin_host::sdk::PluginCommandInvokeInput,
    ) -> Result<agena_plugin_host::sdk::PluginCommandOutput, String> {
        self.current_snapshot()
            .plugin_manager()
            .invoke_plugin_command(plugin_id, input)
            .map_err(|error| error.to_string())
    }

    async fn plugin_rpc(
        &self,
        plugin_id: &str,
        callback_token: Option<String>,
        request: agena_plugin_host::sdk::rpc::Request,
    ) -> Result<agena_plugin_host::sdk::rpc::Response, agena_runtime::PluginRuntimeRpcError> {
        agena_runtime::dispatch_plugin_rpc(
            self.current_snapshot().plugin_manager(),
            plugin_id,
            callback_token,
            request,
        )
        .await
    }
}

impl agena_runtime::RuntimeToolExecutionService for AgenaRuntime {
    fn available_runtime_tools(&self) -> Vec<agena_runtime::RuntimeToolDescriptor> {
        let tools = self.runtime_tool_executor().available_execution_tools();
        let names = crate::tool::execution_tool_names(&tools);
        tools
            .into_iter()
            .zip(names)
            .map(|(tool, name)| agena_runtime::RuntimeToolDescriptor {
                name,
                summary: tool.summary_text().map(ToOwned::to_owned),
                before_help: tool.before_help_text().map(ToOwned::to_owned),
                after_help: tool.after_help_text().map(ToOwned::to_owned),
                input_schema: tool.input_schema(),
            })
            .collect()
    }

    fn available_tool_api_definitions(&self) -> Vec<agena_provider::ToolApiDefinition> {
        self.runtime_tool_executor()
            .available_tool_api_bindings()
            .into_iter()
            .map(|binding| binding.definition())
            .collect()
    }

    fn execute_runtime_tool(
        &self,
        invocation: &agena_domain::ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<agena_tool::ToolExecutionSummary, agena_runtime::RuntimeToolExecutionError> {
        self.runtime_tool_executor()
            .execute_invocation_summary(invocation, session_id, call_id)
            .map_err(|error| agena_runtime::RuntimeToolExecutionError::new(error.to_string()))
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeAuthenticationService for AgenaRuntime {
    fn auth_providers(
        &self,
    ) -> Result<Vec<agena_runtime::RuntimeAuthProvider>, agena_runtime::RuntimeAuthenticationError>
    {
        self.current_snapshot()
            .provider_configs()
            .iter()
            .filter(|(_, resolved)| auth_provider_is_configured(resolved))
            .map(|(provider_id, resolved)| auth_provider_projection(provider_id, resolved))
            .collect()
    }

    fn auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<agena_runtime::RuntimeAuthProvider, agena_runtime::RuntimeAuthenticationError> {
        let resolved = auth_resolved_provider(self, provider_id)?;
        auth_provider_projection(provider_id, &resolved)
    }

    fn set_auth_api_key(
        &self,
        provider_id: &str,
        api_key: String,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        let resolved = auth_resolved_provider(self, provider_id)?;
        if !crate::config::provider_supports_api_key_write(&resolved) {
            return Err(auth_bad_request(format!(
                "{provider_id} does not support api key login"
            )));
        }
        auth_manager_for_runtime(self)
            .set_api_key(provider_id, api_key)
            .map_err(auth_internal)
    }

    async fn start_auth_browser(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        redirect_uri: String,
    ) -> Result<agena_runtime::RuntimeAuthBrowserStart, agena_runtime::RuntimeAuthenticationError>
    {
        let target = auth_oauth_target(self, provider_id, AuthOAuthPurpose::BrowserLogin)?;
        let manager = auth_manager_for_runtime(self);
        match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderOAuthTarget::OpenAi,
            ) => {
                let start = manager
                    .start_openai_browser_login(redirect_uri)
                    .map_err(auth_internal)?;
                Ok(agena_runtime::RuntimeAuthBrowserStart {
                    instance_url: None,
                    authorize_url: start.authorize_url,
                    state: start.state,
                    pkce_verifier: start.pkce_verifier,
                })
            }
            (
                agena_runtime::RuntimeAuthLoginKind::Gitlab,
                crate::config::ProviderOAuthTarget::Gitlab { instance_url },
            ) => {
                let start = manager
                    .start_gitlab_login(instance_url.clone(), redirect_uri)
                    .map_err(auth_internal)?;
                Ok(agena_runtime::RuntimeAuthBrowserStart {
                    instance_url: Some(instance_url),
                    authorize_url: start.authorize_url,
                    state: start.state,
                    pkce_verifier: start.pkce_verifier,
                })
            }
            (kind, _) => Err(auth_bad_request(format!(
                "{provider_id} does not support {} browser login",
                auth_kind_name(kind)
            ))),
        }
    }

    fn wait_auth_browser_callback(
        &self,
        port: u16,
        expected_state: &str,
        timeout: std::time::Duration,
    ) -> Result<agena_provider::OAuthCallback, agena_runtime::RuntimeAuthenticationError> {
        agena_runtime::wait_for_oauth_callback(port, expected_state, timeout).map_err(|error| {
            agena_runtime::RuntimeAuthenticationError::bad_request(error.to_string())
        })
    }

    async fn finish_auth_browser(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        let target = auth_oauth_target(self, provider_id, AuthOAuthPurpose::BrowserLogin)?;
        let manager = auth_manager_for_runtime(self);
        match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderOAuthTarget::OpenAi,
            ) => manager
                .finish_openai_browser_login(provider_id, code, pkce_verifier, redirect_uri)
                .await
                .map(|_| ())
                .map_err(auth_internal),
            (
                agena_runtime::RuntimeAuthLoginKind::Gitlab,
                crate::config::ProviderOAuthTarget::Gitlab { instance_url },
            ) => manager
                .finish_gitlab_login(provider_id, instance_url, code, pkce_verifier, redirect_uri)
                .await
                .map(|_| ())
                .map_err(auth_internal),
            (kind, _) => Err(auth_bad_request(format!(
                "{provider_id} does not support {} browser login",
                auth_kind_name(kind)
            ))),
        }
    }

    async fn start_auth_device(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        enterprise_domain: Option<String>,
    ) -> Result<agena_runtime::RuntimeAuthDeviceStart, agena_runtime::RuntimeAuthenticationError>
    {
        let target = auth_device_target(self, provider_id)?;
        let manager = auth_manager_for_runtime(self);
        let start = match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderDeviceAuthTarget::OpenAi,
            ) => manager
                .start_openai_headless_login()
                .await
                .map_err(auth_internal)?,
            (
                agena_runtime::RuntimeAuthLoginKind::GithubCopilot,
                crate::config::ProviderDeviceAuthTarget::Copilot,
            ) => manager
                .start_copilot_login(auth_copilot_deployment(enterprise_domain))
                .await
                .map_err(auth_internal)?,
            (kind, _) => {
                return Err(auth_bad_request(format!(
                    "{provider_id} does not support {} device login",
                    auth_kind_name(kind)
                )));
            }
        };
        Ok(agena_runtime::RuntimeAuthDeviceStart {
            verification_url: start.verification_url,
            user_code: start.user_code,
            device_code: start.device_code,
            interval_seconds: start.interval_seconds,
        })
    }

    async fn poll_auth_device(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        device_code: String,
        user_code: Option<String>,
        enterprise_domain: Option<String>,
    ) -> Result<bool, agena_runtime::RuntimeAuthenticationError> {
        let target = auth_device_target(self, provider_id)?;
        let manager = auth_manager_for_runtime(self);
        match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderDeviceAuthTarget::OpenAi,
            ) => manager
                .poll_openai_headless_login(provider_id, device_code, user_code.unwrap_or_default())
                .await
                .map(|result| result.is_some())
                .map_err(auth_internal),
            (
                agena_runtime::RuntimeAuthLoginKind::GithubCopilot,
                crate::config::ProviderDeviceAuthTarget::Copilot,
            ) => manager
                .poll_copilot_login(
                    provider_id,
                    device_code,
                    auth_copilot_deployment(enterprise_domain),
                )
                .await
                .map(|result| result.is_some())
                .map_err(auth_internal),
            (kind, _) => Err(auth_bad_request(format!(
                "{provider_id} does not support {} device login",
                auth_kind_name(kind)
            ))),
        }
    }

    fn remove_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        auth_resolved_provider(self, provider_id)?;
        auth_manager_for_runtime(self)
            .remove(provider_id)
            .map_err(auth_internal)
    }

    async fn refresh_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        let target = auth_oauth_target(self, provider_id, AuthOAuthPurpose::CredentialRefresh)?;
        let manager = auth_manager_for_runtime(self);
        match target {
            crate::config::ProviderOAuthTarget::OpenAi => manager
                .refresh_openai_login(provider_id)
                .await
                .map(|_| ())
                .map_err(auth_internal),
            crate::config::ProviderOAuthTarget::Gitlab { instance_url } => manager
                .refresh_gitlab_login(provider_id, instance_url)
                .await
                .map(|_| ())
                .map_err(auth_internal),
        }
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeDraftAuthenticationService for AgenaRuntime {
    fn start_draft_auth_browser(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        instance_url: Option<String>,
        redirect_uri: String,
    ) -> Result<
        agena_runtime::RuntimeDraftAuthBrowserStart,
        agena_runtime::RuntimeAuthenticationError,
    > {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                agena_runtime::start_openai_draft_auth_browser(redirect_uri.as_str())
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => {
                let instance_url = instance_url
                    .ok_or_else(|| auth_bad_request("GitLab instance URL is required"))?;
                agena_runtime::start_gitlab_draft_auth_browser(
                    instance_url.as_str(),
                    redirect_uri.as_str(),
                )
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => Err(auth_bad_request(
                "GitHub Copilot does not support browser draft login",
            )),
        }
    }

    async fn finish_draft_auth_browser(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        instance_url: Option<String>,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<agena_runtime::RuntimeDraftAuthToken, agena_runtime::RuntimeAuthenticationError>
    {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                let user_agent = agena_runtime::codex_user_agent();
                agena_runtime::finish_openai_draft_auth_browser(
                    user_agent.as_str(),
                    code.as_str(),
                    pkce_verifier.as_str(),
                    redirect_uri.as_str(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => {
                let instance_url = instance_url
                    .ok_or_else(|| auth_bad_request("GitLab instance URL is required"))?;
                agena_runtime::finish_gitlab_draft_auth_browser(
                    instance_url.as_str(),
                    code.as_str(),
                    pkce_verifier.as_str(),
                    redirect_uri.as_str(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => Err(auth_bad_request(
                "GitHub Copilot does not support browser draft login",
            )),
        }
    }

    async fn start_draft_auth_device(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
    ) -> Result<agena_runtime::RuntimeDraftAuthDeviceStart, agena_runtime::RuntimeAuthenticationError>
    {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                let user_agent = agena_runtime::codex_user_agent();
                agena_runtime::start_openai_draft_auth_device(user_agent.as_str()).await
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => {
                agena_runtime::start_copilot_draft_auth_device(
                    enterprise_domain.as_deref().unwrap_or("github.com"),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => Err(auth_bad_request(
                "GitLab does not support device draft login",
            )),
        }
    }

    async fn poll_draft_auth_device(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
        device_code: String,
        user_code: Option<String>,
    ) -> Result<
        Option<agena_runtime::RuntimeDraftAuthToken>,
        agena_runtime::RuntimeAuthenticationError,
    > {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                let user_agent = agena_runtime::codex_user_agent();
                agena_runtime::poll_openai_draft_auth_device(
                    user_agent.as_str(),
                    device_code.as_str(),
                    user_code.as_deref().unwrap_or_default(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => {
                agena_runtime::poll_copilot_draft_auth_device(
                    enterprise_domain.as_deref().unwrap_or("github.com"),
                    device_code.as_str(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => Err(auth_bad_request(
                "GitLab does not support device draft login",
            )),
        }
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeConfigurationService for AgenaRuntime {
    fn runtime_configuration(
        &self,
    ) -> Result<agena_runtime::RuntimeConfigurationSnapshot, agena_runtime::RuntimeConfigurationError>
    {
        let snapshot = self.current_snapshot();
        let effective_config = snapshot
            .resolved_config_value()
            .map_err(|error| agena_runtime::RuntimeConfigurationError::new(error.to_string()))?;
        let effective_config = serde_json::to_value(effective_config)
            .map_err(|error| agena_runtime::RuntimeConfigurationError::new(error.to_string()))?;
        let configuration_document = snapshot
            .config_value()
            .map_err(|error| agena_runtime::RuntimeConfigurationError::new(error.to_string()))?;
        Ok(agena_runtime::RuntimeConfigurationSnapshot {
            config_path: snapshot.config_path().to_path_buf(),
            config_found: snapshot.config_found(),
            project_config_path: snapshot.project_config_path().to_path_buf(),
            project_config_found: snapshot.project_config_found(),
            applied_layers: snapshot.applied_layer_descriptions(),
            default_provider: snapshot.default_provider().map(ToOwned::to_owned),
            default_agent: snapshot.default_agent().map(ToOwned::to_owned),
            ui: agena_runtime::RuntimeUiConfiguration {
                locale: snapshot.ui_config().locale,
                theme: snapshot.ui_config().tui.theme,
                color_scheme: match snapshot.ui_config().tui.color_scheme {
                    TuiColorSchemeConfig::Auto => agena_runtime::RuntimeTuiColorScheme::Auto,
                    TuiColorSchemeConfig::Dark => agena_runtime::RuntimeTuiColorScheme::Dark,
                    TuiColorSchemeConfig::Light => agena_runtime::RuntimeTuiColorScheme::Light,
                },
                graphics: match snapshot.ui_config().tui.graphics {
                    TuiGraphicsModeConfig::Auto => agena_runtime::RuntimeTuiGraphicsMode::Auto,
                    TuiGraphicsModeConfig::Native => agena_runtime::RuntimeTuiGraphicsMode::Native,
                    TuiGraphicsModeConfig::Unicode => {
                        agena_runtime::RuntimeTuiGraphicsMode::Unicode
                    }
                },
            },
            effective_config,
            configuration_document,
        })
    }
}

impl agena_runtime::RuntimeConfigSettingsService for AgenaRuntime {
    fn read_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsGetInput,
    ) -> Result<agena_runtime::ConfigSettingsReadResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::read_runtime_file_setting(
            self.current_snapshot().config_path().to_path_buf(),
            input,
        )
    }

    fn read_project_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsGetInput,
    ) -> Result<agena_runtime::ConfigSettingsReadResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::read_runtime_file_setting(
            self.current_snapshot().project_config_path().to_path_buf(),
            input,
        )
    }

    fn list_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsListInput,
    ) -> Result<agena_runtime::ConfigSettingsListResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::list_runtime_file_settings(
            self.current_snapshot().config_path().to_path_buf(),
            input,
        )
    }

    fn set_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsSetInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::set_runtime_file_setting(
            self.current_snapshot().config_path().to_path_buf(),
            input,
            Some(&runtime_settings_schema_validator),
        )
    }

    fn set_project_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsSetInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::set_runtime_file_setting(
            self.current_snapshot().project_config_path().to_path_buf(),
            input,
            Some(&runtime_settings_schema_validator),
        )
    }

    fn patch_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsPatchInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::patch_runtime_file_settings(
            self.current_snapshot().config_path().to_path_buf(),
            input,
            Some(&runtime_settings_schema_validator),
        )
    }

    fn delete_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsDeleteInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::delete_runtime_file_setting(
            self.current_snapshot().config_path().to_path_buf(),
            input,
            Some(&runtime_settings_schema_validator),
        )
    }

    fn delete_project_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsDeleteInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::delete_runtime_file_setting(
            self.current_snapshot().project_config_path().to_path_buf(),
            input,
            Some(&runtime_settings_schema_validator),
        )
    }

    fn validate_file_settings(
        &self,
        _input: agena_runtime::ConfigSettingsValidateInput,
    ) -> Result<
        agena_runtime::ConfigSettingsValidateResponse,
        agena_runtime::RuntimeConfigSettingsError,
    > {
        agena_runtime::validate_runtime_file_settings(
            self.current_snapshot().config_path().to_path_buf(),
            &runtime_settings_schema_validator,
        )
    }
}

fn runtime_settings_schema_validator(
    config_path: &std::path::Path,
    text: &str,
) -> Result<(), agena_runtime::RuntimeConfigSettingsError> {
    crate::config::validate_config_text(config_path, text, &crate::config::ProcessEnvironment)
        .map_err(agena_runtime::config_error_to_settings_error)
}

fn runtime_agents_status(snapshot: &RuntimeSnapshot) -> agena_runtime::RuntimeAgentsStatus {
    let mut entries = snapshot.agents().list_descriptors();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let default_agent = snapshot
        .default_agent()
        .map(ToOwned::to_owned)
        .filter(|name| entries.iter().any(|entry| entry.name == *name))
        .or_else(|| entries.first().map(|entry| entry.name.clone()))
        .unwrap_or_else(|| "none".to_string());
    agena_runtime::RuntimeAgentsStatus {
        default_agent,
        agents: entries
            .into_iter()
            .map(|entry| agena_runtime::RuntimeAgentStatus {
                name: entry.name,
                description: entry.description,
                permission: entry.permission,
                defaults: agena_runtime::RuntimeAgentSelectionStatus {
                    provider: entry.defaults.provider,
                    adapter: entry.defaults.adapter,
                    model: entry.defaults.model,
                    thinking_mode: entry.defaults.thinking_mode,
                    speed_mode: entry.defaults.speed_mode,
                    verbosity: entry.defaults.verbosity,
                    parallel_tool_calls: entry.defaults.parallel_tool_calls,
                },
                allowed_tools: entry.tools.allow,
                scope: entry.scope,
                source_path: entry.source_path.map(|path| path.display().to_string()),
            })
            .collect(),
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeControlService for AgenaRuntime {
    async fn reload(
        &self,
    ) -> Result<agena_runtime::RuntimeReloadReport, agena_runtime::RuntimeControlServiceError> {
        AgenaRuntime::reload(self)
            .await
            .map_err(|error| agena_runtime::RuntimeControlServiceError::new(error.to_string()))
    }

    async fn fetch_provider_client_versions(
        &self,
    ) -> Result<agena_provider::ProviderClientVersions, agena_runtime::RuntimeControlServiceError>
    {
        agena_runtime::fetch_latest_provider_client_versions()
            .await
            .map_err(|error| agena_runtime::RuntimeControlServiceError::new(error.to_string()))
    }

    fn runtime_metrics(&self) -> agena_runtime::RuntimeMetricsSnapshot {
        agena_runtime::runtime_metrics_snapshot()
    }

    fn snapshot_backend_capabilities(
        &self,
        workspace: &Path,
    ) -> agena_tool::SnapshotBackendCapabilities {
        agena_runtime::snapshot_backend_capabilities(workspace)
    }

    fn start_runtime_reload_task(
        &self,
        cause: agena_runtime::RuntimeReloadCause,
        origin: agena_runtime::RuntimeBackgroundTaskOrigin,
    ) -> Result<
        agena_runtime::RuntimeBackgroundTaskStart,
        agena_runtime::RuntimeBackgroundTaskControlError,
    > {
        AgenaRuntime::start_runtime_reload_task(self, cause, origin)
    }

    fn start_background_task(
        &self,
        kind: agena_runtime::RuntimeBackgroundTaskKind,
        origin: agena_runtime::RuntimeBackgroundTaskOrigin,
        title: String,
        dedupe_key: Option<String>,
        cancellable: bool,
        work: agena_runtime::RuntimeBackgroundTaskWork,
    ) -> Result<
        agena_runtime::RuntimeBackgroundTaskStart,
        agena_runtime::RuntimeBackgroundTaskControlError,
    > {
        AgenaRuntime::spawn_background_task(
            self,
            kind,
            origin,
            title,
            dedupe_key,
            cancellable,
            move |cancel| async move {
                work(cancel)
                    .await
                    .map_err(|error| AppError::Config(error.to_string()))
            },
        )
    }

    fn background_tasks(&self) -> Vec<agena_runtime::RuntimeBackgroundTask> {
        AgenaRuntime::background_tasks(self)
    }

    fn cancel_background_task(
        &self,
        task_id: &str,
    ) -> Result<
        agena_runtime::RuntimeBackgroundTask,
        agena_runtime::RuntimeBackgroundTaskControlError,
    > {
        AgenaRuntime::cancel_background_task(self, task_id)
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeStatusService for AgenaRuntime {
    fn agents_status(&self) -> agena_runtime::RuntimeAgentsStatus {
        runtime_agents_status(self.current_snapshot().as_ref())
    }

    fn agent_profile(&self, name: &str) -> Option<agena_runtime::RuntimeAgentProfile> {
        let profile = self.current_snapshot().agents().get(name.trim())?;
        Some(agena_runtime::RuntimeAgentProfile {
            name: profile.name,
            description: profile.frontmatter.description,
            permission: profile.frontmatter.permission,
            defaults: agena_runtime::RuntimeAgentSelectionStatus {
                provider: profile.frontmatter.defaults.provider,
                adapter: profile.frontmatter.defaults.adapter,
                model: profile.frontmatter.defaults.model,
                thinking_mode: profile.frontmatter.defaults.thinking_mode,
                speed_mode: profile.frontmatter.defaults.speed_mode,
                verbosity: profile.frontmatter.defaults.verbosity,
                parallel_tool_calls: profile.frontmatter.defaults.parallel_tool_calls,
            },
            allowed_tools: profile.frontmatter.tools.allow,
            prompt: profile.prompt,
            scope: profile.scope,
            source_path: profile.source_path.map(|path| path.display().to_string()),
        })
    }

    async fn runtime_status(&self) -> agena_runtime::RuntimeStatusSnapshot {
        let snapshot = self.current_snapshot();
        let mut provider_ids = snapshot.provider_registry().provider_ids();
        provider_ids.sort();

        let mcp = if let Some(manager) = snapshot.mcp_manager() {
            let mut servers = manager.server_names().await;
            servers.sort();
            let all_tools = manager.all_tools().await;
            let mut tool_counts = HashMap::<String, usize>::new();
            for (server_name, _) in &all_tools {
                *tool_counts.entry(server_name.clone()).or_default() += 1;
            }
            agena_runtime::RuntimeMcpStatus {
                servers: servers
                    .into_iter()
                    .map(|name| agena_runtime::RuntimeMcpServerStatus {
                        tool_count: tool_counts.get(&name).copied().unwrap_or(0),
                        name,
                    })
                    .collect(),
            }
        } else {
            agena_runtime::RuntimeMcpStatus::default()
        };

        let lsp = if let Some(registry) = snapshot.lsp_registry() {
            let mut servers = registry.server_specs().await;
            servers.sort_by(|left, right| left.name.cmp(&right.name));
            let diagnostics = registry.collect_diagnostics().await;
            agena_runtime::RuntimeLspStatus {
                diagnostics_count: diagnostics.iter().map(|(_, entries)| entries.len()).sum(),
                files_with_diagnostics: diagnostics.len(),
                servers: servers
                    .into_iter()
                    .map(|server| agena_runtime::RuntimeLspServerStatus {
                        name: server.name,
                        command: server.command,
                        file_extensions: server.file_extensions,
                        root_markers: server.root_markers,
                    })
                    .collect(),
            }
        } else {
            agena_runtime::RuntimeLspStatus::default()
        };

        let skills = {
            let entries = snapshot
                .plugin_manager()
                .registered_tools()
                .into_iter()
                .filter(|entry| entry.plugin_full_name() == "agena.skills")
                .collect::<Vec<_>>();
            let skill_key_for = |entry: &agena_plugin_host::registry::RegisteredTool| {
                entry
                    .definition
                    .permissions
                    .tags
                    .iter()
                    .find_map(|tag| match tag {
                        agena_plugin_host::sdk::ToolTag::Custom(value) => {
                            value.strip_prefix("skill:").map(str::to_string)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| entry.tool_name().to_string())
            };
            let has_custom_tag = |entry: &agena_plugin_host::registry::RegisteredTool,
                                  expected: &str| {
                entry
                    .definition
                    .permissions
                    .tags
                    .iter()
                    .any(|tag| match tag {
                        agena_plugin_host::sdk::ToolTag::Custom(value) => value == expected,
                        _ => false,
                    })
            };
            let mut aliases_by_skill = HashMap::<String, Vec<String>>::new();
            for entry in &entries {
                if !has_custom_tag(entry, "alias") {
                    continue;
                }
                aliases_by_skill
                    .entry(skill_key_for(entry))
                    .or_default()
                    .push(entry.canonical_name());
            }
            let mut skills = Vec::new();
            let mut commands = Vec::new();
            for entry in entries {
                if has_custom_tag(&entry, "alias") {
                    continue;
                }
                let item = agena_runtime::RuntimeSkillStatus {
                    name: entry.canonical_name(),
                    description: entry
                        .definition
                        .summary_text()
                        .unwrap_or_default()
                        .to_owned(),
                    aliases: aliases_by_skill
                        .remove(&skill_key_for(&entry))
                        .unwrap_or_default(),
                    source_path: None,
                };
                if has_custom_tag(&entry, "command") {
                    commands.push(item);
                } else {
                    skills.push(item);
                }
            }
            skills.sort_by(|left, right| left.name.cmp(&right.name));
            commands.sort_by(|left, right| left.name.cmp(&right.name));
            agena_runtime::RuntimeSkillsStatus { skills, commands }
        };

        let agents = runtime_agents_status(snapshot.as_ref());

        let session_manager = snapshot.session_manager();
        let (session_cache, automation_available, scheduled_jobs) = match session_manager {
            Some(manager) => (
                Some(agena_runtime::SessionExecutionControl::cache_stats(
                    manager.as_ref(),
                )),
                agena_runtime::SessionExecutionControl::scheduler_available(manager.as_ref()),
                agena_runtime::SessionExecutionControl::list_scheduled_jobs(manager.as_ref()).await,
            ),
            None => (None, false, Vec::new()),
        };
        let plugin_manager = snapshot.plugin_manager();
        agena_runtime::RuntimeStatusSnapshot {
            generation: snapshot.generation(),
            loaded_at: snapshot.loaded_at(),
            workspace_root: self.workspace_root().to_path_buf(),
            config_path: snapshot.config_path().to_path_buf(),
            config_found: snapshot.config_found(),
            provider_ids,
            plugin_count: plugin_manager.plugins().len(),
            session_runtime_available: session_cache.is_some(),
            watch_paths: snapshot.watch_paths().to_vec(),
            reload_enabled: snapshot.reload_enabled(),
            reload_interval_secs: snapshot.reload_poll_interval().as_secs(),
            session_gc_enabled: snapshot.session_gc_enabled(),
            session_gc_interval_secs: snapshot.session_gc_interval().as_secs(),
            session_cache,
            model_catalog: snapshot.model_catalog_response(),
            model_catalog_refreshing: self.model_catalog_refresh_active(),
            background_tasks: self.background_tasks(),
            automation_available,
            scheduled_jobs,
            mcp,
            lsp,
            skills,
            agents,
            plugin_ui_catalog: plugin_manager.ui_catalog(),
            tool_registry_generation: plugin_manager.tool_registry_generation(),
            tool_registry_last_event: plugin_manager
                .tool_registry_events_since(None, 1)
                .into_iter()
                .next(),
        }
    }
}

#[async_trait::async_trait]
impl agena_provider::ProviderModelSource for AgenaRuntime {
    fn provider_ids(&self) -> Vec<agena_domain::ProviderId> {
        self.current_snapshot()
            .catalog_source_provider_registry()
            .provider_ids()
            .into_iter()
            .map(agena_domain::ProviderId::new)
            .collect()
    }

    async fn list_models(
        &self,
        provider_id: &agena_domain::ProviderId,
    ) -> Result<Vec<agena_domain::Model>, agena_provider::ProviderCatalogError> {
        self.current_snapshot()
            .catalog_source_provider_registry()
            .list_models(provider_id.as_ref())
            .await
            .map_err(|error| agena_provider::ProviderCatalogError::Operation(error.to_string()))
    }
}

impl AgenaRuntime {
    async fn list_adapter_models_target(
        &self,
        target: crate::config::ProviderAdapterModelsTarget,
    ) -> Result<agena_provider::ProviderAdapterModelsListing, agena_provider::ProviderCatalogError>
    {
        let snapshot = self.current_snapshot();
        let listing = crate::config::list_provider_adapter_models_with_providers(
            snapshot.provider_configs(),
            &target,
            &crate::config::ProcessEnvironment,
        )
        .await
        .map_err(|error| agena_provider::ProviderCatalogError::Operation(error.to_string()))?;
        Ok(agena_provider::ProviderAdapterModelsListing {
            provider_id: listing.provider_id,
            adapters: listing
                .adapters
                .into_iter()
                .map(|adapter| agena_provider::ProviderAdapterModelsEntry {
                    adapter_id: adapter.adapter_id,
                    enabled: adapter.enabled,
                    resolved_base_url: adapter.resolved_base_url,
                    models: adapter.models,
                    error: adapter.error,
                })
                .collect(),
        })
    }
}

fn runtime_bootstrap_error(error: AppError) -> agena_runtime::RuntimeBootstrapError {
    let message = error.to_string();
    match error {
        AppError::Config(_) | AppError::ConfigErr(_) => {
            agena_runtime::RuntimeBootstrapError::configuration(message)
        }
        AppError::Database(_) | AppError::StorageConfig(_) => {
            agena_runtime::RuntimeBootstrapError::database(message)
        }
        AppError::Io(_) => agena_runtime::RuntimeBootstrapError::io(message),
        _ => agena_runtime::RuntimeBootstrapError::internal(message),
    }
}

fn runtime_database_error(error: agena_runtime::RuntimeDatabaseCompositionError) -> AppError {
    match error {
        agena_runtime::RuntimeDatabaseCompositionError::StorageConfig(error) => {
            AppError::StorageConfig(error)
        }
        agena_runtime::RuntimeDatabaseCompositionError::Database(error) => {
            AppError::Database(error)
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgenaRuntime {
    pub(crate) inner: Arc<AgenaRuntimeInner>,
}

pub(crate) type AgenaRuntimeInner = agena_runtime::RuntimeProcessState<
    ConfigLoader<ProcessEnvironment>,
    LoadConfigRequest,
    RuntimeSnapshot,
    AppError,
>;

impl agena_runtime::RuntimeBootstrapLifecycle for AgenaRuntime {
    fn shutdown(&self) {
        AgenaRuntime::shutdown(self);
    }
}

impl AgenaRuntime {
    pub(crate) fn current_snapshot(&self) -> Arc<RuntimeSnapshot> {
        self.inner.control_state.current_snapshot()
    }

    pub(crate) fn session_manager(&self) -> Option<Arc<SessionManager>> {
        self.current_snapshot().session_manager()
    }

    /// Assemble application-facing runtime capabilities once at the concrete
    /// composition boundary. Application consumers receive the runtime-owned
    /// result rather than converting this concrete handle independently.
    pub fn application_services(&self) -> agena_runtime::RuntimeApplicationServices {
        self.application_services_with_manager_option(self.session_manager())
    }

    fn application_services_with_manager_option(
        &self,
        session_manager: Option<Arc<SessionManager>>,
    ) -> agena_runtime::RuntimeApplicationServices {
        let repositories = self.inner.database.as_ref().map(|database| {
            agena_runtime::RuntimeApplicationRepositories {
                memory: Arc::new(MemoryStore::for_workspace(self.workspace_root())),
                workspace: Arc::new(agena_storage_sqlite::SeaWorkspaceRepository::new(
                    Arc::clone(database),
                )),
                permission_rules: Arc::new(agena_storage_sqlite::SeaPermissionRuleRepository::new(
                    Arc::clone(database),
                )),
                session_stats: Arc::new(agena_storage_sqlite::SeaSessionStatsRepository::new(
                    Arc::clone(database),
                )),
                session_summary: Arc::new(agena_storage_sqlite::SeaSessionSummaryRepository::new(
                    Arc::clone(database),
                )),
                session_mutation: Arc::new(agena_storage_sqlite::SeaSessionSummaryRepository::new(
                    Arc::clone(database),
                )),
            }
        });
        let session_queries = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionQueryService>);
        let execution_control = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionExecutionControl>);
        let execution_commands = session_manager.as_ref().map(|manager| {
            manager.clone() as Arc<dyn agena_runtime::SessionExecutionCommandService>
        });
        let tool_execution = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionToolExecutionService>);
        let plugin_commands = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionPluginCommandService>);
        agena_runtime::compose_runtime_application_services(
            agena_runtime::RuntimeApplicationServiceCompositionInputs {
                workspace_root: self.workspace_root().to_path_buf(),
                repositories,
                provider_catalog: Arc::new(self.clone()),
                model_catalog: Arc::new(self.clone()),
                plugins: Arc::new(self.clone()),
                configuration: Arc::new(self.clone()),
                config_settings: Arc::new(self.clone()),
                control: Arc::new(self.clone()),
                authentication: Arc::new(self.clone()),
                draft_authentication: Arc::new(self.clone()),
                status: Arc::new(self.clone()),
                tools: Arc::new(self.clone()),
                event_queries: session_manager.as_ref().map(|manager| {
                    manager.clone() as Arc<dyn agena_runtime::RuntimeEventQueryService>
                }),
                event_stream: session_manager.as_ref().map(|manager| {
                    manager.clone() as Arc<dyn agena_runtime::RuntimeEventStreamService>
                }),
                event_publisher: session_manager.as_ref().map(|manager| {
                    manager.clone() as Arc<dyn agena_runtime::RuntimeEventPublishService>
                }),
                session_queries,
                execution_control,
                execution_commands,
                tool_execution,
                plugin_commands,
            },
        )
    }

    fn runtime_tool_executor(&self) -> ToolExecutor {
        if let Some(manager) = self.session_manager() {
            return manager.tool_executor();
        }

        let snapshot = self.current_snapshot();
        ToolExecutor::new(
            self.workspace_root().to_path_buf(),
            Agent::new(
                "runtime-tool-service",
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            crate::agents::SubagentRegistry::default(),
            snapshot.plugin_manager(),
            None,
            None,
            None,
            snapshot.plugin_config().policy.tool_presentation.clone(),
        )
    }

    pub fn workspace_root(&self) -> &Path {
        &self.inner.workspace_root
    }

    pub fn shutdown(&self) {
        if let Some(session_manager) = self.session_manager() {
            match tokio::runtime::Handle::try_current() {
                Ok(_handle) => {
                    agena_runtime::spawn_detached(async move {
                        session_manager
                            .broadcast_active_session_end(
                                agena_plugin_host::SessionEndReason::Other,
                            )
                            .await;
                    });
                }
                Err(_) => {
                    tracing::debug!(
                        target: "agena_plugin_host::session_end",
                        "no tokio runtime available during shutdown; skipping session.end broadcast"
                    );
                }
            }
        }
        self.inner.control_state.shutdown();
    }

    pub async fn reload(&self) -> Result<RuntimeReloadReport, AppError> {
        self.reload_with_cause(RuntimeReloadCause::Manual).await
    }

    pub(crate) async fn reload_with_cause(
        &self,
        cause: RuntimeReloadCause,
    ) -> Result<RuntimeReloadReport, AppError> {
        let _guard = self.inner.control_state.reload_gate().acquire().await;
        let previous = self.current_snapshot();
        let next = Arc::new(
            RuntimeSnapshot::build_with_previous(
                previous.generation() + 1,
                &self.inner.loader,
                &self.inner.load_request,
                self.inner.workspace_root.as_path(),
                self.inner.database.clone(),
                previous.session_manager(),
                Arc::clone(&previous),
            )
            .await?,
        );

        self.apply_tracing_filter(next.tracing_config());
        let previous_generation = previous.generation();
        // Install runtime-backed HostClient into the new snapshot's plugin
        // host so post-reload plugin callbacks keep working.
        {
            let host_handle = next.plugin_manager().host_handle();
            let client = super::host_client::host_client_for(self.clone());
            agena_runtime::install_plugin_host_client(Arc::clone(&host_handle), client).await;
            super::host_client::install_plugin_host_event_publisher(host_handle, self.clone());
        }
        let _ = self.inner.control_state.swap_snapshot(next.clone());
        let _ = self.start_model_catalog_refresh_if_needed(RuntimeBackgroundTaskOrigin::System);

        Ok(RuntimeReloadReport {
            cause,
            previous_generation,
            generation: next.generation(),
            loaded_at: next.loaded_at(),
        })
    }

    pub(crate) fn task_control_handle(&self) -> Arc<TaskControl> {
        self.inner.control_state.task_control_handle()
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.inner.control_state.task_control().is_shutdown()
    }

    pub fn background_tasks(&self) -> Vec<RuntimeBackgroundTask> {
        self.inner.control_state.background_tasks().list()
    }

    pub fn model_catalog_refresh_active(&self) -> bool {
        self.inner
            .control_state
            .background_tasks()
            .is_kind_running(RuntimeBackgroundTaskKind::ModelCatalogRefresh)
    }

    pub fn cancel_background_task(
        &self,
        task_id: &str,
    ) -> Result<RuntimeBackgroundTask, RuntimeBackgroundTaskControlError> {
        self.inner.control_state.background_tasks().cancel(task_id)
    }

    pub fn spawn_background_task<F, Fut>(
        &self,
        kind: RuntimeBackgroundTaskKind,
        origin: RuntimeBackgroundTaskOrigin,
        title: impl Into<String>,
        dedupe_key: Option<String>,
        cancellable: bool,
        work: F,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<RuntimeBackgroundTaskOutcome, AppError>>
            + Send
            + 'static,
    {
        if self.is_shutdown() {
            return Err(RuntimeBackgroundTaskControlError::Shutdown);
        }

        let spec = RuntimeBackgroundTaskSpec::new(kind, origin, title, dedupe_key, cancellable);
        Ok(self
            .inner
            .control_state
            .background_tasks()
            .spawn(spec, work))
    }

    pub fn start_runtime_reload_task(
        &self,
        cause: RuntimeReloadCause,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        let dedupe_key = Some("runtime_reload".to_owned());
        let title = match &cause {
            RuntimeReloadCause::Manual => "Reload runtime".to_owned(),
            RuntimeReloadCause::WatchedPathsChanged { paths } => {
                format!(
                    "Reload runtime after {} watched path change(s)",
                    paths.len()
                )
            }
        };
        let runtime = self.clone();
        self.spawn_background_task(
            RuntimeBackgroundTaskKind::RuntimeReload,
            origin,
            title,
            dedupe_key,
            false,
            move |_| async move {
                let report = runtime.reload_with_cause(cause.clone()).await?;
                let message = match &cause {
                    RuntimeReloadCause::Manual => {
                        format!("Runtime reloaded to generation {}.", report.generation)
                    }
                    RuntimeReloadCause::WatchedPathsChanged { paths } => format!(
                        "Runtime reloaded to generation {} after changes in {} watched path(s).",
                        report.generation,
                        paths.len()
                    ),
                };
                Ok(RuntimeBackgroundTaskOutcome::succeeded(message))
            },
        )
    }

    pub fn start_model_catalog_refresh(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        self.spawn_model_catalog_refresh(origin)
    }

    fn start_model_catalog_refresh_if_needed(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Option<RuntimeBackgroundTaskStart> {
        if self.is_shutdown() {
            return None;
        }

        if !self
            .current_snapshot()
            .model_catalog()
            .needs_startup_refresh()
        {
            return None;
        }

        self.spawn_model_catalog_refresh(origin).ok()
    }

    fn spawn_model_catalog_refresh(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        let runtime = self.clone();
        self.spawn_background_task(
            RuntimeBackgroundTaskKind::ModelCatalogRefresh,
            origin,
            "Refresh model catalog",
            Some("model_catalog_refresh".to_owned()),
            true,
            move |cancel| async move {
                let result: Result<RuntimeBackgroundTaskOutcome, AppError> = async {
                    let snapshot = runtime.current_snapshot();
                    let model_catalog = snapshot.model_catalog();
                    let provider_priorities =
                        agena_runtime::provider_model_catalog_priorities(snapshot.provider_configs());
                    let refreshed = agena_runtime::run_cancellable_refresh(
                        cancel.clone(),
                        || runtime.is_shutdown(),
                        || async {
                            model_catalog
                                .refresh_from_source(
                                    &runtime,
                                    Some(&provider_priorities),
                                )
                                .await
                                .map_err(|error| AppError::Config(error.to_string()))
                        },
                        || async { runtime.reload().await.map(|_| ()) },
                    )
                    .await?;

                    let Some(refreshed) = refreshed else {
                        return Ok(RuntimeBackgroundTaskOutcome::cancelled(
                            "Cancelled before applying the refreshed catalog to the runtime snapshot.",
                        ));
                    };

                    let message = refreshed
                        .last_error
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|warning| format!("Refreshed model catalog with warnings: {warning}"))
                        .unwrap_or_else(|| "Refreshed model catalog.".to_owned());

                    Ok(RuntimeBackgroundTaskOutcome::succeeded(message))
                }
                .await;

                if let Err(error) = &result {
                    runtime
                        .current_snapshot()
                        .model_catalog()
                        .record_refresh_failure(error.to_string());
                    tracing::warn!(
                        error = %error,
                        origin = ?origin,
                        "background model catalog refresh failed"
                    );
                }

                result
            },
        )
    }

    fn spawn_background_tasks(&self) {
        let janitor_runtime = self.clone();
        let reload_runtime = self.clone();
        agena_runtime::spawn_runtime_maintenance_loops(
            self.inner.control_state.task_control(),
            async move {
                let control = janitor_runtime.task_control_handle();
                let interval_runtime = janitor_runtime.clone();
                let tick_runtime = janitor_runtime;
                agena_runtime::run_session_maintenance(
                    control,
                    move || interval_runtime.current_snapshot().session_gc_interval(),
                    move || {
                        let runtime = tick_runtime.clone();
                        async move {
                            if let Some(manager) = runtime.current_snapshot().session_manager() {
                                manager.prune_cache();
                            }
                        }
                    },
                )
                .await;
            },
            async move { reload::run(reload_runtime).await },
        );

        let _ = self.start_model_catalog_refresh_if_needed(RuntimeBackgroundTaskOrigin::System);
    }

    fn apply_tracing_filter(&self, tracing: &agena_runtime::RuntimeTracingConfiguration) {
        match agena_runtime::apply_runtime_tracing_filter(&self.inner.control_state, tracing) {
            Ok(false) => {
                tracing::debug!("tracing filter reload skipped or rejected");
            }
            Ok(true) => {}
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    filter = tracing.filter,
                    database = tracing.database,
                    "invalid tracing filter in runtime config"
                );
            }
        }
    }
}
