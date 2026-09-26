use agena_domain::{AdapterId, ProviderId};

use super::{
    AdapterBuildContext, BTreeMap, CatalogedModelsProvider, ConfigEnvironment, ConfigError,
    GitlabRoutedBackend, HttpAdapterKind, LIST_MODELS_DEFAULT_MODEL_ID, ModelCatalogSnapshot,
    ProviderAdapterDefinition, ProviderAdapterModelsResult, ProviderAuthConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, build_adapter_provider,
    gitlab_credential_proxy_base_url, gitlab_proxy_base_url, parse_adapter_model_ref,
    provider_endpoint_root, resolve_http_adapter_base_url,
};

pub async fn list_provider_adapter_models(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapters: &BTreeMap<String, ResolvedProviderAdapterConfig>,
    catalog: Option<&ModelCatalogSnapshot>,
    client: reqwest::Client,
    env: &dyn ConfigEnvironment,
    client_identity: &crate::ProviderClientIdentity,
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
                failure: None,
            });
            continue;
        }

        let provider = match build_adapter_provider(
            provider_id,
            adapter_id.as_ref(),
            adapter,
            LIST_MODELS_DEFAULT_MODEL_ID,
            auth,
            AdapterBuildContext {
                client: client.clone(),
                env,
                config_path: None,
                client_identity,
            },
        ) {
            Ok(provider) => provider,
            Err(err) => {
                let failure = adapter_models_config_failure(provider_id, adapter_id, &err);
                results.push(ProviderAdapterModelsResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
                    resolved_base_url,
                    models: Vec::new(),
                    failure: Some(failure),
                });
                continue;
            }
        };
        // Model discovery uses a fresh adapter instead of the configured runtime
        // provider. Apply the same catalog baseline so draft and saved listings
        // expose the limits and capabilities used by actual sessions.
        let provider = if let Some(catalog) = catalog {
            CatalogedModelsProvider::new(provider, catalog.merged_models().into_provider_catalog())
        } else {
            provider
        };

        match provider.list_models().await {
            Ok(mut models) => {
                for model in &mut models {
                    model.provider_id = ProviderId::new(provider_id.to_owned());
                    model.adapter_id = Some(AdapterId::new(adapter_id.clone()));
                    let catalog_model_id =
                        agena_provider::normalized_catalog_model_id(model.id.as_ref());
                    if !catalog_model_id.is_empty() {
                        model.catalog_model_id = Some(agena_domain::ModelId::new(catalog_model_id));
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
                    failure: None,
                });
            }
            Err(err) => {
                let failure = adapter_models_failure(provider_id, adapter_id, &err);
                results.push(ProviderAdapterModelsResult {
                    adapter_id: adapter_id.clone(),
                    enabled: true,
                    resolved_base_url,
                    models: Vec::new(),
                    failure: Some(failure),
                });
            }
        }
    }
    results.sort_by(|left, right| left.adapter_id.cmp(&right.adapter_id));
    results
}

fn adapter_models_config_failure(
    provider_id: &str,
    adapter_id: &str,
    error: &ConfigError,
) -> agena_failure::Failure {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    let failure = Failure::new(
        FailureCode::new("provider.misconfigured"),
        FailureCategory::InvalidInput,
        FailureResponsibility::Caller,
        RetryDirective::CorrectInput,
        RecoveryDirective::OpenSettings,
        FailureImpact::RuntimeDegraded,
        UserPresentation::new(
            "provider.misconfigured",
            "The provider is not configured correctly. Review its settings.",
        ),
    );
    tracing::warn!(
        failure_id = %failure.id,
        provider = %provider_id,
        adapter = %adapter_id,
        diagnostic = %error,
        "provider model listing setup failed"
    );
    failure
}

fn adapter_models_failure(
    provider_id: &str,
    adapter_id: &str,
    error: &agena_runtime_provider::ProviderError,
) -> agena_failure::Failure {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };
    use agena_provider::ProviderErrorKind;

    let kind = error.provider_error_kind();
    let (code, category, responsibility, retry, recovery, fallback) = match kind {
        Some(ProviderErrorKind::Authentication) => (
            "provider.authentication_required",
            FailureCategory::AuthenticationRequired,
            FailureResponsibility::Caller,
            RetryDirective::AfterUserAction,
            RecoveryDirective::Reauthenticate,
            "Provider authentication is required before models can be listed.",
        ),
        Some(ProviderErrorKind::RateLimited) => (
            "provider.rate_limited",
            FailureCategory::RateLimited,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "The provider is rate-limiting model discovery. Try again shortly.",
        ),
        Some(ProviderErrorKind::Timeout | ProviderErrorKind::Connection) => (
            "provider.connection_failed",
            FailureCategory::Timeout,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "The provider could not be reached in time. Check the connection and try again.",
        ),
        Some(ProviderErrorKind::Misconfiguration | ProviderErrorKind::InvalidRequest) => (
            "provider.misconfigured",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::OpenSettings,
            "The provider is not configured correctly. Review its settings.",
        ),
        None if matches!(error, agena_runtime_provider::ProviderError::Config(_)) => (
            "provider.misconfigured",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::OpenSettings,
            "The provider is not configured correctly. Review its settings.",
        ),
        _ => (
            "provider.model_discovery_failed",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "The provider could not list models. Try again shortly.",
        ),
    };
    let failure = Failure::new(
        FailureCode::new(code),
        category,
        responsibility,
        retry,
        recovery,
        FailureImpact::RuntimeDegraded,
        UserPresentation::new(code, fallback),
    );
    tracing::warn!(
        failure_id = %failure.id,
        provider = %provider_id,
        adapter = %adapter_id,
        diagnostic = %error,
        "provider model listing failed"
    );
    failure
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
            .models
            .iter()
            .filter(|(route, config)| {
                config.enabled
                    && parse_adapter_model_ref(provider_id, route)
                        .ok()
                        .is_some_and(|(route_adapter_id, _)| route_adapter_id == *adapter_id)
            })
            .find_map(|(route, _)| {
                parse_adapter_model_ref(provider_id, route)
                    .ok()
                    .map(|(_, model_id)| model_id)
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
        ProviderAdapterDefinition::OpenAiResponses(_) => match auth {
            ProviderAuthConfig::Api(api) if api.gitlab().is_some() => {
                Ok(Some(gitlab_proxy_base_url(
                    &api.gitlab().expect("guard ensures gitlab api auth"),
                    GitlabRoutedBackend::OpenAiResponses,
                )))
            }
            ProviderAuthConfig::Credential(config) if config.gitlab().is_some() => Ok(Some(
                gitlab_credential_proxy_base_url(config, GitlabRoutedBackend::OpenAiResponses),
            )),
            _ => Ok(Some(resolve_http_adapter_base_url(
                provider_id,
                auth,
                HttpAdapterKind::OpenAi,
            )?)),
        },
        ProviderAdapterDefinition::OpenAiChatCompletions(_) => match auth {
            ProviderAuthConfig::Api(api) if api.gitlab().is_some() => {
                Ok(Some(gitlab_proxy_base_url(
                    &api.gitlab().expect("guard ensures gitlab api auth"),
                    GitlabRoutedBackend::OpenAiChatCompletions,
                )))
            }
            ProviderAuthConfig::Credential(config) if config.gitlab().is_some() => {
                Ok(Some(gitlab_credential_proxy_base_url(
                    config,
                    GitlabRoutedBackend::OpenAiChatCompletions,
                )))
            }
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

#[cfg(test)]
mod tests {
    use super::{adapter_models_config_failure, adapter_models_failure};
    use agena_provider::{CatalogModelDefinition, ModelCatalogSnapshot, ProviderErrorKind};

    #[tokio::test]
    async fn adapter_listing_hydrates_prefixed_model_from_catalog() {
        use tokio::io::AsyncWriteExt as _;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind model fixture");
        let address = listener.local_addr().expect("model fixture address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept model request");
            let body = r#"{"object":"list","data":[{"id":"cline-pass/deepseek-v4.1-flash","object":"model"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write model response");
        });

        let target = agena_runtime_provider::config_support::draft_provider_adapter_models_target(
            Some("cline"),
            format!("http://{address}").as_str(),
            agena_provider::ProviderProtocolPathsConfig::default(),
            Some("fixture-key"),
            None,
            &["openai_chat_completions".to_owned()],
        )
        .expect("draft model target");
        let mut catalog = ModelCatalogSnapshot::default();
        catalog.official.models.insert(
            "deepseek-v4.1-flash".to_owned(),
            CatalogModelDefinition {
                context_window_tokens: Some(1_000_000),
                max_input_tokens: Some(1_000_000),
                max_output_tokens: Some(384_000),
                description: Some("Catalog description".to_owned()),
                ..Default::default()
            },
        );

        let results = super::list_provider_adapter_models(
            &target.provider_id,
            &target.auth,
            &target.adapters,
            Some(&catalog),
            reqwest::Client::new(),
            &agena_runtime_config::ProcessEnvironment,
            &crate::ProviderClientIdentity::new(Default::default()),
        )
        .await;
        server.abort();

        assert_eq!(results.len(), 1);
        assert!(results[0].failure.is_none());
        assert_eq!(results[0].models.len(), 1);
        let model = &results[0].models[0];
        assert_eq!(model.id.as_ref(), "cline-pass/deepseek-v4.1-flash");
        assert_eq!(
            model.catalog_model_id.as_ref().map(AsRef::<str>::as_ref),
            Some("deepseek-v4.1-flash")
        );
        assert_eq!(model.metadata.limits.context_window_tokens, Some(1_000_000));
        assert_eq!(model.metadata.limits.max_input_tokens, Some(1_000_000));
        assert_eq!(model.metadata.limits.max_output_tokens, Some(384_000));
        assert_eq!(
            model.metadata.description.as_deref(),
            Some("Catalog description")
        );
    }

    #[test]
    fn adapter_model_listing_never_serializes_provider_diagnostics() {
        let diagnostic = "raw provider body token=secret socket=/private/provider.sock authorization: bearer abc";
        let error = agena_runtime_provider::ProviderError::ProviderClassified {
            provider: "malicious".to_owned(),
            message: diagnostic.to_owned(),
            kind: ProviderErrorKind::Authentication,
            retryable: false,
        };
        let failure = adapter_models_failure("malicious", "adapter", &error);
        let resource = agena_failure::UserProblem::from(&failure);
        let encoded = serde_json::to_string(&resource).expect("serialize user projection");

        assert!(!encoded.contains(diagnostic));
        assert!(!encoded.contains("token=secret"));
        assert!(!encoded.contains("/private/provider.sock"));
        assert!(!encoded.contains("bearer abc"));
    }

    #[test]
    fn adapter_model_setup_never_serializes_config_diagnostics() {
        let diagnostic = "config at /Users/example/secret.json contains api_key=secret";
        let error = super::ConfigError::Validation(diagnostic.to_owned());
        let failure = adapter_models_config_failure("malicious", "adapter", &error);
        let encoded = serde_json::to_string(&agena_failure::UserProblem::from(&failure))
            .expect("serialize user projection");

        assert!(!encoded.contains(diagnostic));
        assert!(!encoded.contains("/Users/example"));
        assert!(!encoded.contains("api_key=secret"));
    }
}
