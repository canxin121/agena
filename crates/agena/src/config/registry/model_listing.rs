use super::{
    AdapterId, BTreeMap, ConfigEnvironment, ConfigError, GitlabRoutedBackend, HttpAdapterKind,
    LIST_MODELS_DEFAULT_MODEL_ID, ProviderAdapterDefinition, ProviderAdapterModelsResult,
    ProviderAuthConfig, ProviderId, ResolvedProviderAdapterConfig, ResolvedProviderConfig,
    build_adapter_provider, gitlab_credential_proxy_base_url, gitlab_proxy_base_url,
    parse_adapter_model_ref, provider_endpoint_root, resolve_http_adapter_base_url,
};

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
            adapter_id.as_ref(),
            adapter,
            LIST_MODELS_DEFAULT_MODEL_ID,
            auth,
            client.clone(),
            env,
            None,
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
                        crate::model_catalog::canonical_model_catalog_id(model.id.as_ref());
                    if !catalog_model_id.is_empty() {
                        model.catalog_model_id = Some(crate::model::ModelId::new(catalog_model_id));
                    }
                    let fallback = provider.model_capabilities_for_adapter(None, &model.id);
                    model.capabilities = model
                        .capabilities
                        .clone()
                        .merged_with_fallbacks_from(&fallback);
                    let metadata_fallback = provider.model_metadata_for_adapter(None, &model.id);
                    model.metadata = model
                        .metadata
                        .clone()
                        .merged_with_fallbacks_from(&metadata_fallback);
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

pub(crate) fn resolve_adapter_default_models(
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
        let default_model = resolved
            .defaults
            .model
            .clone()
            .or_else(|| {
                resolved
                    .models
                    .iter()
                    .filter(|(route, config)| {
                        config.enabled
                            && parse_adapter_model_ref(provider_id, route)
                                .ok()
                                .is_some_and(|(route_adapter_id, _)| {
                                    route_adapter_id == *adapter_id
                                })
                    })
                    .find_map(|(route, _)| {
                        parse_adapter_model_ref(provider_id, route)
                            .ok()
                            .map(|(_, model_id)| model_id)
                    })
            })
            .unwrap_or_else(|| LIST_MODELS_DEFAULT_MODEL_ID.to_owned());
        defaults.insert(adapter_id.clone(), default_model);
    }

    Ok(defaults)
}

pub(crate) fn resolved_adapter_models_base_url(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    definition: &ProviderAdapterDefinition,
) -> Result<Option<String>, ConfigError> {
    match definition {
        ProviderAdapterDefinition::OpenAi(_) => match auth {
            ProviderAuthConfig::Api(api) if api.gitlab().is_some() => {
                Ok(Some(gitlab_proxy_base_url(
                    &api.gitlab().expect("guard ensures gitlab api auth"),
                    GitlabRoutedBackend::OpenAi,
                )))
            }
            ProviderAuthConfig::Credential(config) if config.gitlab().is_some() => Ok(Some(
                gitlab_credential_proxy_base_url(config, GitlabRoutedBackend::OpenAi),
            )),
            _ => Ok(Some(resolve_http_adapter_base_url(
                provider_id,
                auth,
                HttpAdapterKind::OpenAi,
            )?)),
        },
        ProviderAdapterDefinition::Anthropic(_) => match auth {
            ProviderAuthConfig::Api(api) if api.gitlab().is_some() => {
                Ok(Some(gitlab_proxy_base_url(
                    &api.gitlab().expect("guard ensures gitlab api auth"),
                    GitlabRoutedBackend::Anthropic,
                )))
            }
            ProviderAuthConfig::Credential(config) if config.gitlab().is_some() => Ok(Some(
                gitlab_credential_proxy_base_url(config, GitlabRoutedBackend::Anthropic),
            )),
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
        ProviderAdapterDefinition::AmazonBedrock(_) => match auth {
            ProviderAuthConfig::Api(api) if api.bedrock_sigv4().is_some() => Ok(Some(
                api.bedrock_sigv4()
                    .expect("guard ensures bedrock sigv4 api auth")
                    .base_url,
            )),
            _ => Ok(Some(
                provider_endpoint_root(auth, provider_id)?.0.to_owned(),
            )),
        },
    }
}
