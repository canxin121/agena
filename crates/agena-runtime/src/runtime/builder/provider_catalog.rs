use super::*;

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
