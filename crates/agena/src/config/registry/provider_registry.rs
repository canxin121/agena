use super::{
    AmazonBedrockAdapter, AnthropicAdapter, AnthropicAdapterOptions, AnthropicProfile, Arc,
    AuthData, AuthRefreshStrategy, AuthSecretSelector, CapabilityFamily, CatalogedModelsProvider,
    ConfigEnvironment, ConfigError, GeminiAdapter, GeminiAdapterOptions, GitlabProvider,
    GitlabProviderConfig, GitlabRoutedAdapter, GitlabRoutedBackend, HttpAdapterKind,
    LIST_MODELS_DEFAULT_MODEL_ID, ManagedCredential, ModelCatalogSnapshot, ModelId, ModelRuntime,
    MultiAdapterProvider, OllamaAdapter, OpenAiChatCompletionsAdapter,
    OpenAiChatCompletionsAdapterOptions, OpenAiProfile, OpenAiRealtimeAdapter,
    OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions, Path,
    ProcessEnvironment, ProviderAdapterDefinition, ProviderAuthConfig,
    ProviderModelDiscoveryConfig, ProviderModelRoute, ProviderRegistry, ResolvedConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, api_auth_has_direct_source,
    api_auth_managed_credential, copilot_base_url, gitlab_auth_managed_credential,
    gitlab_credential_instance_url, gitlab_credential_runtime_config, gitlab_runtime_config,
    http_adapter_default_user_agent, http_adapter_extra_headers, openai_adapter_api_credential,
    openai_adapter_capability_family, parse_adapter_model_ref, require_provider_auth_credential,
    required_api_auth_credential, resolve_adapter_default_models, resolve_http_adapter_base_url,
    runtime_adapter_provider_id, static_bedrock_credentials, to_hash_map,
};

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
        self.build_provider_registry_with_catalog_and_env_and_config_path(catalog, env, None)
    }

    pub fn build_provider_registry_with_catalog_and_env_and_config_path(
        &self,
        catalog: Option<&ModelCatalogSnapshot>,
        env: &dyn ConfigEnvironment,
        config_path: Option<&Path>,
    ) -> Result<ProviderRegistry, ConfigError> {
        let mut registry = ProviderRegistry::new();

        for (provider_id, resolved) in &self.providers {
            if !resolved.enabled {
                continue;
            }

            let client =
                ProviderRegistry::build_http_client(crate::provider::ProviderHttpClientConfig {
                    timeout: std::time::Duration::from_secs(resolved.network.request_timeout_secs),
                    connect_timeout: std::time::Duration::from_secs(
                        resolved.network.connect_timeout_secs,
                    ),
                })?;
            let provider = build_provider(
                provider_id.as_ref(),
                resolved,
                client.clone(),
                env,
                catalog,
                config_path,
            )?;
            registry.register_arc(provider);
        }

        Ok(registry)
    }
}

pub(crate) fn build_provider(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
    catalog: Option<&ModelCatalogSnapshot>,
    config_path: Option<&Path>,
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
                    adapter_id.as_ref(),
                    adapter,
                    adapter_defaults
                        .get(adapter_id.as_str())
                        .expect("adapter default should exist")
                        .as_ref(),
                    &resolved.auth,
                    client.clone(),
                    env,
                    config_path,
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
                    agena_tool_mode: config.agena_tools.mode,
                    provider_native_tools: config.agena_tools.provider_native.clone(),
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

    let provider: Arc<dyn ModelRuntime> = Arc::new(MultiAdapterProvider::new(
        provider_id,
        resolved
            .defaults
            .adapter
            .clone()
            .expect("resolved provider default adapter"),
        adapter_defaults
            .get(
                resolved
                    .defaults
                    .adapter
                    .as_deref()
                    .expect("resolved provider default adapter"),
            )
            .cloned()
            .unwrap_or_else(|| LIST_MODELS_DEFAULT_MODEL_ID.to_owned()),
        adapters,
        routes,
        configured_only_adapters,
    ));

    if let Some(provider_record) = catalog.map(|snapshot| snapshot.merged_models()) {
        Ok(CatalogedModelsProvider::new(provider, provider_record))
    } else {
        Ok(provider)
    }
}

struct OpenAiConnection {
    credential: ManagedCredential,
    auth_data: Option<Arc<tokio::sync::Mutex<AuthData>>>,
    base_url: String,
    profile: OpenAiProfile,
    capability_family: CapabilityFamily,
}

#[allow(clippy::too_many_arguments)]
fn resolve_openai_connection(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
    config_path: Option<&Path>,
    capability_family: Option<crate::config::ProviderCapabilityFamilyConfig>,
    models_url: Option<&str>,
) -> Result<OpenAiConnection, ConfigError> {
    match auth {
        ProviderAuthConfig::Api(_) => {
            let credential =
                openai_adapter_api_credential(provider_id, auth, client, capability_family, env)?;
            Ok(OpenAiConnection {
                credential: credential.credential,
                auth_data: credential.auth_data,
                base_url: resolve_http_adapter_base_url(
                    provider_id,
                    auth,
                    HttpAdapterKind::OpenAi,
                )?,
                profile: OpenAiProfile::Standard,
                capability_family: openai_adapter_capability_family(
                    provider_id,
                    auth,
                    capability_family,
                    models_url,
                )
                .unwrap_or(CapabilityFamily::OpenAi),
            })
        }
        ProviderAuthConfig::Credential(credential_auth) => match credential_auth.issuer() {
            crate::provider::auth::CredentialIssuer::OpenaiChatgpt => {
                let credential = require_provider_auth_credential(
                    provider_id,
                    "api_key",
                    auth,
                    AuthSecretSelector::AccessOrApiKey,
                    AuthRefreshStrategy::OpenAiOAuth,
                    env,
                    config_path,
                )?;
                Ok(OpenAiConnection {
                    credential: credential.credential,
                    auth_data: credential.auth_data,
                    base_url: "https://chatgpt.com/backend-api/codex".to_owned(),
                    profile: OpenAiProfile::Standard,
                    capability_family: CapabilityFamily::OpenAi,
                })
            }
            crate::provider::auth::CredentialIssuer::GithubCopilot => {
                let credential = require_provider_auth_credential(
                    provider_id,
                    "bearer_token",
                    auth,
                    AuthSecretSelector::RefreshOrAccess,
                    AuthRefreshStrategy::ReloadFromStore,
                    env,
                    config_path,
                )?;
                Ok(OpenAiConnection {
                    credential: credential.credential,
                    auth_data: credential.auth_data,
                    base_url: "https://api.githubcopilot.com".to_owned(),
                    profile: OpenAiProfile::GithubCopilot,
                    capability_family: CapabilityFamily::OpenAi,
                })
            }
            crate::provider::auth::CredentialIssuer::GoogleAdc
            | crate::provider::auth::CredentialIssuer::SapAiCore => {
                let credential = openai_adapter_api_credential(
                    provider_id,
                    auth,
                    client,
                    capability_family,
                    env,
                )?;
                Ok(OpenAiConnection {
                    credential: credential.credential,
                    auth_data: credential.auth_data,
                    base_url: resolve_http_adapter_base_url(
                        provider_id,
                        auth,
                        HttpAdapterKind::OpenAi,
                    )?,
                    profile: OpenAiProfile::Standard,
                    capability_family: openai_adapter_capability_family(
                        provider_id,
                        auth,
                        capability_family,
                        models_url,
                    )
                    .unwrap_or(CapabilityFamily::OpenAi),
                })
            }
            _ => Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: "credential issuer is not supported by this OpenAI protocol adapter"
                    .to_owned(),
            }),
        },
        ProviderAuthConfig::None => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "OpenAI protocol adapters require API or credential authentication".to_owned(),
        }),
    }
}

fn build_gitlab_routed_adapter(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
    config_path: Option<&Path>,
    adapter_default_model: &str,
    backend: GitlabRoutedBackend,
) -> Result<Arc<dyn ModelRuntime>, ConfigError> {
    let inner = match auth {
        ProviderAuthConfig::Credential(credential_auth) if credential_auth.gitlab().is_some() => {
            GitlabProvider::from_managed_token_with_config(
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
                    config_path,
                )?
                .credential,
                gitlab_credential_runtime_config(credential_auth, adapter_default_model),
            )?
        }
        ProviderAuthConfig::Api(api) if api.gitlab().is_some() => {
            let gitlab = api.gitlab().expect("guard ensures gitlab api auth");
            GitlabProvider::from_managed_token_with_config(
                client,
                gitlab_auth_managed_credential(provider_id, auth, env, config_path)?.credential,
                gitlab_runtime_config(&gitlab, adapter_default_model),
            )?
        }
        _ => {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: "GitLab-routed adapter requires GitLab API or credential auth".to_owned(),
            });
        }
    };
    Ok(Arc::new(GitlabRoutedAdapter {
        inner: Arc::new(inner),
        backend,
        default_model: ModelId::new(adapter_default_model),
    }))
}

pub(crate) fn build_adapter_provider(
    provider_id: &str,
    adapter_id: &str,
    config: &ResolvedProviderAdapterConfig,
    adapter_default_model: &str,
    auth: &ProviderAuthConfig,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
    config_path: Option<&Path>,
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
        ProviderAdapterDefinition::OpenAiResponses(adapter) => match auth {
            ProviderAuthConfig::Credential(_credential_auth)
                if _credential_auth.gitlab().is_some() =>
            {
                build_gitlab_routed_adapter(
                    provider_id,
                    auth,
                    client,
                    env,
                    config_path,
                    adapter_default_model,
                    GitlabRoutedBackend::OpenAiResponses,
                )?
            }
            ProviderAuthConfig::Api(api) if api.gitlab().is_some() => build_gitlab_routed_adapter(
                provider_id,
                auth,
                client,
                env,
                config_path,
                adapter_default_model,
                GitlabRoutedBackend::OpenAiResponses,
            )?,
            _ => {
                let connection = resolve_openai_connection(
                    provider_id,
                    auth,
                    client.clone(),
                    env,
                    config_path,
                    adapter.options.capability_family,
                    adapter.options.models_url.as_deref(),
                )?;
                Arc::new(OpenAiResponsesAdapter::new_managed_with_options(
                    runtime_provider_id.as_str(),
                    client,
                    connection.credential,
                    connection.base_url,
                    adapter_default_model.to_owned(),
                    OpenAiResponsesAdapterOptions {
                        backend: adapter.options.backend.into(),
                        auth_data: connection.auth_data,
                        profile: connection.profile,
                        models_url: adapter.options.models_url.clone(),
                        auth_header: adapter.options.auth_header.clone(),
                        auth_scheme: adapter.options.auth_scheme.clone(),
                        capability_family: connection.capability_family,
                        extra_headers: http_adapter_extra_headers(
                            adapter,
                            Some(http_adapter_default_user_agent(
                                auth,
                                HttpAdapterKind::OpenAi,
                                adapter_default_model,
                            )),
                        ),
                        top_level_prompt_cache_override: None,
                    },
                ))
            }
        },
        ProviderAdapterDefinition::OpenAiChatCompletions(adapter) => {
            if matches!(auth, ProviderAuthConfig::Credential(config) if config.gitlab().is_some())
                || matches!(auth, ProviderAuthConfig::Api(api) if api.gitlab().is_some())
            {
                build_gitlab_routed_adapter(
                    provider_id,
                    auth,
                    client,
                    env,
                    config_path,
                    adapter_default_model,
                    GitlabRoutedBackend::OpenAiChatCompletions,
                )?
            } else {
                let connection = resolve_openai_connection(
                    provider_id,
                    auth,
                    client.clone(),
                    env,
                    config_path,
                    adapter.options.capability_family,
                    adapter.options.models_url.as_deref(),
                )?;
                Arc::new(OpenAiChatCompletionsAdapter::new_managed_with_options(
                    runtime_provider_id.as_str(),
                    client,
                    connection.credential,
                    connection.base_url,
                    adapter_default_model.to_owned(),
                    OpenAiChatCompletionsAdapterOptions {
                        auth_data: connection.auth_data,
                        profile: connection.profile,
                        models_url: adapter.options.models_url.clone(),
                        auth_header: adapter.options.auth_header.clone(),
                        auth_scheme: adapter.options.auth_scheme.clone(),
                        capability_family: connection.capability_family,
                        extra_headers: http_adapter_extra_headers(
                            adapter,
                            Some(http_adapter_default_user_agent(
                                auth,
                                HttpAdapterKind::OpenAi,
                                adapter_default_model,
                            )),
                        ),
                        top_level_prompt_cache_override: None,
                    },
                ))
            }
        }
        ProviderAdapterDefinition::OpenAiRealtime(adapter) => {
            let connection = resolve_openai_connection(
                provider_id,
                auth,
                client.clone(),
                env,
                config_path,
                adapter.options.capability_family,
                adapter.options.models_url.as_deref(),
            )?;
            Arc::new(OpenAiRealtimeAdapter::new_managed_with_options(
                runtime_provider_id.as_str(),
                client,
                connection.credential,
                connection.base_url,
                adapter_default_model.to_owned(),
                OpenAiRealtimeAdapterOptions {
                    auth_data: connection.auth_data,
                    models_url: adapter.options.models_url.clone(),
                    auth_header: adapter.options.auth_header.clone(),
                    auth_scheme: adapter.options.auth_scheme.clone(),
                    capability_family: connection.capability_family,
                    extra_headers: http_adapter_extra_headers(
                        adapter,
                        Some(http_adapter_default_user_agent(
                            auth,
                            HttpAdapterKind::OpenAi,
                            adapter_default_model,
                        )),
                    ),
                    realtime_ws_url: adapter.options.realtime_ws_url.clone(),
                },
            ))
        }
        ProviderAdapterDefinition::Anthropic(adapter) => match auth {
            ProviderAuthConfig::Credential(_credential_auth)
                if _credential_auth.gitlab().is_some() =>
            {
                build_gitlab_routed_adapter(
                    provider_id,
                    auth,
                    client,
                    env,
                    config_path,
                    adapter_default_model,
                    GitlabRoutedBackend::Anthropic,
                )?
            }
            ProviderAuthConfig::Api(api) if api.gitlab().is_some() => build_gitlab_routed_adapter(
                provider_id,
                auth,
                client,
                env,
                config_path,
                adapter_default_model,
                GitlabRoutedBackend::Anthropic,
            )?,
            ProviderAuthConfig::Credential(credential_auth)
                if matches!(
                    credential_auth.issuer(),
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
                    config_path,
                )?;
                let base_url = copilot_base_url(credential.auth_data.as_ref(), None)
                    .unwrap_or_else(|| "https://api.githubcopilot.com".to_owned());

                let provider = AnthropicAdapter::new_managed_with_options(
                    runtime_provider_id.as_str(),
                    client,
                    credential.credential,
                    base_url,
                    adapter_default_model.to_owned(),
                    AnthropicAdapterOptions {
                        auth_data: credential.auth_data,
                        auth_header: adapter.options.auth_header.clone(),
                        auth_scheme: adapter.options.auth_scheme.clone(),
                        models_url: adapter.options.models_url.clone(),
                        messages_url: adapter.options.messages_url.clone(),
                        profile: AnthropicProfile::GithubCopilot,
                        extra_beta_header: adapter.options.extra_beta_header.clone(),
                        override_beta_header: adapter.options.extra_beta_header.is_some(),
                        extra_headers: http_adapter_extra_headers(
                            adapter,
                            Some(http_adapter_default_user_agent(
                                auth,
                                HttpAdapterKind::Anthropic,
                                adapter_default_model,
                            )),
                        ),
                        eager_input_streaming_override: adapter.options.eager_input_streaming,
                    },
                );
                Arc::new(provider)
            }
            _ => Arc::new(AnthropicAdapter::new_managed_with_options(
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
                AnthropicAdapterOptions {
                    auth_data: None,
                    auth_header: adapter.options.auth_header.clone(),
                    auth_scheme: adapter.options.auth_scheme.clone(),
                    models_url: adapter.options.models_url.clone(),
                    messages_url: adapter.options.messages_url.clone(),
                    profile: AnthropicProfile::Standard,
                    extra_beta_header: adapter.options.extra_beta_header.clone(),
                    override_beta_header: adapter.options.extra_beta_header.is_some(),
                    extra_headers: http_adapter_extra_headers(
                        adapter,
                        Some(http_adapter_default_user_agent(
                            auth,
                            HttpAdapterKind::Anthropic,
                            adapter_default_model,
                        )),
                    ),
                    eager_input_streaming_override: adapter.options.eager_input_streaming,
                },
            )),
        },
        ProviderAdapterDefinition::Gemini(adapter) => {
            Arc::new(GeminiAdapter::new_managed_with_options(
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
                GeminiAdapterOptions {
                    auth_header: adapter
                        .options
                        .auth_header
                        .clone()
                        .map(|header| (header, adapter.options.auth_scheme.clone())),
                    auth_query_parameter: None,
                    extra_headers: http_adapter_extra_headers(
                        adapter,
                        Some(http_adapter_default_user_agent(
                            auth,
                            HttpAdapterKind::Gemini,
                            adapter_default_model,
                        )),
                    ),
                    stream_mode: adapter.options.stream_mode.into(),
                    realtime_ws_url: adapter.options.realtime_ws_url.clone(),
                },
            ))
        }
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
                    if api.gitlab().is_some() {
                        gitlab_auth_managed_credential(provider_id, auth, env, config_path)?
                            .credential
                    } else if api_auth_has_direct_source(api, env) {
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
                        config_path,
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
            ProviderAuthConfig::Api(api) if api.bedrock_sigv4().is_some() => {
                let sigv4 = api
                    .bedrock_sigv4()
                    .expect("guard ensures bedrock sigv4 api auth");
                AmazonBedrockAdapter::new_sigv4(
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
                )
            }
            _ => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "amazon_bedrock adapter requires api subtype `bedrock_sigv4`"
                        .to_owned(),
                });
            }
        }),
    };

    Ok(provider)
}
