//! Provider and model catalog queries for the application layer.

use agena_api::{
    queries::{
        ListProviderAdapterModelsParams, ListSavedProviderAdapterModelsParams, ProviderSecretSource,
    },
    resource::{
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse,
        ProviderAdapterSummaryResource, ProviderDefaultsResource,
        ProviderModelCapabilitiesResource, ProviderModelMetadataResource,
        ProviderModelRequestOverrideResource, ProviderModelResource,
        ProviderModelSpeedModeResource, ProviderModelThinkingModeResource, ProviderModelsResponse,
        ProviderSummaryResource, ReasoningEffortResource, ThinkingDisplayResource,
        ThinkingRequestResource,
    },
};
use agena_domain::{Model, ModelCapabilities, ProviderId};
use agena_provider::{
    DraftHttpProviderAdapterModelsRequest, DraftProviderAdapterModelsRequest,
    ProviderAdapterModelsEntry, ProviderApiKeySource, ProviderCatalogError,
    ProviderProtocolPaths as ProviderCatalogProtocolPaths,
};

use crate::provider_studio::catalog::{
    catalog_lookup_candidates, catalog_model_to_catalog_definition,
    preferred_catalog_model_for_lookup_ids,
};
use crate::{Application, ApplicationError};

/// Discover AWS profile names in the server process environment. Provider
/// adapters consume credentials on the server, so clients must never inspect
/// their own HOME for this catalog.
pub fn list_aws_profile_names() -> Vec<String> {
    let credentials_path = std::env::var("AWS_SHARED_CREDENTIALS_FILE")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| std::path::PathBuf::from(home).join(".aws/credentials"))
        });
    let config_path = std::env::var("AWS_CONFIG_FILE")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| std::path::PathBuf::from(home).join(".aws/config"))
        });
    let mut profiles = std::collections::BTreeSet::new();
    if let Some(path) = credentials_path
        && let Ok(text) = std::fs::read_to_string(path)
    {
        profiles.extend(parse_aws_profile_names(
            text.as_str(),
            AwsProfileFileKind::Credentials,
        ));
    }
    if let Some(path) = config_path
        && let Ok(text) = std::fs::read_to_string(path)
    {
        profiles.extend(parse_aws_profile_names(
            text.as_str(),
            AwsProfileFileKind::Config,
        ));
    }
    profiles.into_iter().collect()
}

#[derive(Debug, Clone, Copy)]
enum AwsProfileFileKind {
    Credentials,
    Config,
}

fn parse_aws_profile_names(text: &str, kind: AwsProfileFileKind) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if !line.starts_with('[') || !line.ends_with(']') {
            continue;
        }
        let section = line.trim_start_matches('[').trim_end_matches(']').trim();
        match kind {
            AwsProfileFileKind::Credentials => {
                if !section.is_empty() {
                    names.insert(section.to_owned());
                }
            }
            AwsProfileFileKind::Config => {
                if section.eq_ignore_ascii_case("default") {
                    names.insert("default".to_owned());
                } else if section.len() > "profile ".len()
                    && section
                        .get(.."profile ".len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("profile "))
                {
                    let profile = section["profile ".len()..].trim();
                    if !profile.is_empty() {
                        names.insert(profile.to_owned());
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}

pub fn list_providers_response(state: &Application) -> Vec<ProviderSummaryResource> {
    state
        .provider_catalog()
        .list_providers()
        .into_iter()
        .map(|provider| ProviderSummaryResource {
            provider_id: provider.provider_id.to_string(),
            defaults: ProviderDefaultsResource {
                adapter: provider.defaults.adapter,
                model: provider.defaults.model,
            },
            adapters: provider
                .adapters
                .into_iter()
                .map(|adapter| ProviderAdapterSummaryResource {
                    adapter_id: adapter.adapter_id,
                    enabled: adapter.enabled,
                    configured_model_count: adapter.configured_model_count,
                })
                .collect(),
        })
        .collect()
}

pub async fn list_provider_models_response(
    state: &Application,
    provider_id: String,
) -> Result<ProviderModelsResponse, ApplicationError> {
    let provider_id_value = ProviderId::new(provider_id.clone());
    if !state
        .provider_catalog()
        .contains_provider(&provider_id_value)
    {
        return Err(ApplicationError::not_found_with_diagnostic(
            "The provider was not found.",
            format!("provider {provider_id} not found"),
        ));
    }

    let catalog = state.provider_catalog();
    let models = match catalog.list_models(&provider_id_value).await {
        Ok(models) if !models.is_empty() => models,
        Ok(_) => catalog
            .configured_local_models(&provider_id_value)
            .map_err(|error| ApplicationError::internal(error.to_string()))?,
        Err(error) => {
            let fallback = catalog
                .configured_local_models(&provider_id_value)
                .map_err(|fallback_error| ApplicationError::internal(fallback_error.to_string()))?;
            if fallback.is_empty() {
                return Err(ApplicationError::internal(error.to_string()));
            }
            fallback
        }
    };
    Ok(ProviderModelsResponse {
        provider_id,
        models: models
            .into_iter()
            .map(provider_model_resource_from_domain)
            .collect(),
    })
}

/// Return only models implied by saved server configuration. Thin clients use
/// this for session/default-model choices so opening the TUI never performs
/// provider network discovery and never offers an unconfigured model.
pub fn list_configured_provider_models_response(
    state: &Application,
    provider_id: String,
) -> Result<ProviderModelsResponse, ApplicationError> {
    let provider_id_value = ProviderId::new(provider_id.clone());
    if !state
        .provider_catalog()
        .contains_provider(&provider_id_value)
    {
        return Err(ApplicationError::not_found_with_diagnostic(
            "The provider was not found.",
            format!("provider {provider_id} not found"),
        ));
    }
    let models = state
        .provider_catalog()
        .configured_local_models(&provider_id_value)
        .map_err(|error| ApplicationError::internal(error.to_string()))?;
    Ok(ProviderModelsResponse {
        provider_id,
        models: models
            .into_iter()
            .map(provider_model_resource_from_domain)
            .collect(),
    })
}

pub fn provider_model_resource_from_domain(value: Model) -> ProviderModelResource {
    ProviderModelResource {
        provider_id: value.provider_id.to_string(),
        adapter_id: value.adapter_id.map(|id| id.to_string()),
        id: value.id.to_string(),
        catalog_model_id: value.catalog_model_id.map(|id| id.to_string()),
        display_name: value.display_name,
        native_compaction: value.native_compaction,
        capabilities: provider_model_capabilities_resource_from_domain(value.capabilities),
        metadata: provider_model_metadata_resource_from_domain(value.metadata),
        thinking_modes: value
            .thinking_modes
            .into_iter()
            .map(provider_model_thinking_mode_resource_from_domain)
            .collect(),
        speed_modes: value
            .speed_modes
            .into_iter()
            .map(|(name, mode)| (name, provider_model_speed_mode_resource_from_domain(mode)))
            .collect(),
    }
}

fn provider_model_capabilities_resource_from_domain(
    value: ModelCapabilities,
) -> ProviderModelCapabilitiesResource {
    ProviderModelCapabilitiesResource {
        text_input: capability_support_resource_from_domain(value.text_input),
        image_input: capability_support_resource_from_domain(value.image_input),
        document_input: capability_support_resource_from_domain(value.document_input),
        audio_input: capability_support_resource_from_domain(value.audio_input),
        video_input: capability_support_resource_from_domain(value.video_input),
        file_input: capability_support_resource_from_domain(value.file_input),
        tool_calling: capability_support_resource_from_domain(value.tool_calling),
        streaming: capability_support_resource_from_domain(value.streaming),
        reasoning: capability_support_resource_from_domain(value.reasoning),
        structured_output: capability_support_resource_from_domain(value.structured_output),
        temperature_supported: capability_support_resource_from_domain(value.temperature_supported),
    }
}

const fn capability_support_resource_from_domain(
    value: agena_domain::CapabilitySupport,
) -> agena_api::resource::CapabilitySupportResource {
    match value {
        agena_domain::CapabilitySupport::Supported => {
            agena_api::resource::CapabilitySupportResource::Supported
        }
        agena_domain::CapabilitySupport::Unsupported => {
            agena_api::resource::CapabilitySupportResource::Unsupported
        }
        agena_domain::CapabilitySupport::Unknown => {
            agena_api::resource::CapabilitySupportResource::Unknown
        }
    }
}

fn provider_model_metadata_resource_from_domain(
    value: agena_domain::ModelMetadata,
) -> ProviderModelMetadataResource {
    ProviderModelMetadataResource {
        lifecycle: value.lifecycle.map(model_lifecycle_resource_from_domain),
        context_window_tokens: value.limits.context_window_tokens,
        max_input_tokens: value.limits.max_input_tokens,
        max_output_tokens: value.limits.max_output_tokens,
        description: value.description,
        knowledge_cutoff: value.knowledge_cutoff,
        release_date: value.release_date,
        last_updated: value.last_updated,
        open_weights: value.open_weights,
        supports_parallel_tool_calls: value.supports_parallel_tool_calls,
        supports_verbosity: value.supports_verbosity,
        default_verbosity: value.default_verbosity,
        default_temperature: value.default_temperature,
        default_top_p: value.default_top_p,
        default_top_k: value.default_top_k,
        assistant_reasoning_interleaved: value.assistant_reasoning_interleaved,
        assistant_reasoning_field: value.assistant_reasoning_field,
        output_modalities: value.output_modalities,
        pricing: value
            .pricing
            .map(|pricing| agena_api::resource::ModelPricing {
                input_usd_per_million_tokens: pricing.input_usd_per_million_tokens,
                output_usd_per_million_tokens: pricing.output_usd_per_million_tokens,
                cache_read_usd_per_million_tokens: pricing.cache_read_usd_per_million_tokens,
                cache_write_usd_per_million_tokens: pricing.cache_write_usd_per_million_tokens,
                tiers: pricing
                    .tiers
                    .into_iter()
                    .map(|tier| agena_api::resource::ModelPricingTier {
                        tier_type: tier.tier_type,
                        size_tokens: tier.size_tokens,
                        input_usd_per_million_tokens: tier.input_usd_per_million_tokens,
                        output_usd_per_million_tokens: tier.output_usd_per_million_tokens,
                        cache_read_usd_per_million_tokens: tier.cache_read_usd_per_million_tokens,
                        cache_write_usd_per_million_tokens: tier.cache_write_usd_per_million_tokens,
                    })
                    .collect(),
            }),
    }
}

const fn model_lifecycle_resource_from_domain(
    value: agena_domain::ModelLifecycle,
) -> agena_api::resource::ModelLifecycle {
    match value {
        agena_domain::ModelLifecycle::Active => agena_api::resource::ModelLifecycle::Active,
        agena_domain::ModelLifecycle::Preview => agena_api::resource::ModelLifecycle::Preview,
        agena_domain::ModelLifecycle::Beta => agena_api::resource::ModelLifecycle::Beta,
        agena_domain::ModelLifecycle::Alpha => agena_api::resource::ModelLifecycle::Alpha,
        agena_domain::ModelLifecycle::Experimental => {
            agena_api::resource::ModelLifecycle::Experimental
        }
        agena_domain::ModelLifecycle::Deprecated => agena_api::resource::ModelLifecycle::Deprecated,
    }
}

fn provider_model_thinking_mode_resource_from_domain(
    value: agena_domain::ModelThinkingMode,
) -> ProviderModelThinkingModeResource {
    ProviderModelThinkingModeResource {
        is_default: value.is_default,
        display_name: value.display_name,
        description: value.description,
        preset: value.preset,
        thinking: value.thinking.map(thinking_request_resource_from_domain),
        request_override: provider_model_request_override_resource_from_domain(
            value.request_override,
        ),
        adapter_overrides: value
            .adapter_overrides
            .into_iter()
            .map(|(adapter, override_patch)| {
                (
                    adapter,
                    provider_model_request_override_resource_from_domain(override_patch),
                )
            })
            .collect(),
    }
}

fn provider_model_speed_mode_resource_from_domain(
    value: agena_domain::ModelSpeedMode,
) -> ProviderModelSpeedModeResource {
    ProviderModelSpeedModeResource {
        is_default: value.is_default,
        display_name: value.display_name,
        description: value.description,
        request_override: provider_model_request_override_resource_from_domain(
            value.request_override,
        ),
        adapter_overrides: value
            .adapter_overrides
            .into_iter()
            .map(|(adapter, override_patch)| {
                (
                    adapter,
                    provider_model_request_override_resource_from_domain(override_patch),
                )
            })
            .collect(),
    }
}

fn provider_model_request_override_resource_from_domain(
    value: agena_domain::ModelSpeedModeRequestOverride,
) -> ProviderModelRequestOverrideResource {
    ProviderModelRequestOverrideResource {
        headers: value.headers,
        body_patch: value.body_patch,
    }
}

fn thinking_request_resource_from_domain(
    value: agena_domain::ThinkingRequest,
) -> ThinkingRequestResource {
    match value {
        agena_domain::ThinkingRequest::Budget { budget_tokens } => {
            ThinkingRequestResource::Budget { budget_tokens }
        }
        agena_domain::ThinkingRequest::Adaptive { effort, display } => {
            ThinkingRequestResource::Adaptive {
                effort: effort.map(reasoning_effort_resource_from_domain),
                display: display.map(thinking_display_resource_from_domain),
            }
        }
        agena_domain::ThinkingRequest::Effort { effort } => ThinkingRequestResource::Effort {
            effort: reasoning_effort_resource_from_domain(effort),
        },
        agena_domain::ThinkingRequest::Disabled => ThinkingRequestResource::Disabled,
    }
}

const fn reasoning_effort_resource_from_domain(
    value: agena_domain::ReasoningEffort,
) -> ReasoningEffortResource {
    match value {
        agena_domain::ReasoningEffort::Minimal => ReasoningEffortResource::Minimal,
        agena_domain::ReasoningEffort::Low => ReasoningEffortResource::Low,
        agena_domain::ReasoningEffort::Medium => ReasoningEffortResource::Medium,
        agena_domain::ReasoningEffort::High => ReasoningEffortResource::High,
        agena_domain::ReasoningEffort::Xhigh => ReasoningEffortResource::Xhigh,
        agena_domain::ReasoningEffort::Max => ReasoningEffortResource::Max,
    }
}

const fn thinking_display_resource_from_domain(
    value: agena_domain::ThinkingDisplay,
) -> ThinkingDisplayResource {
    match value {
        agena_domain::ThinkingDisplay::Summarized => ThinkingDisplayResource::Summarized,
        agena_domain::ThinkingDisplay::Omitted => ThinkingDisplayResource::Omitted,
    }
}

pub async fn list_provider_adapter_models_response(
    state: &Application,
    params: ListProviderAdapterModelsParams,
) -> Result<ProviderAdapterModelsResponse, ApplicationError> {
    let listing = state
        .provider_catalog()
        .list_draft_adapter_models(DraftProviderAdapterModelsRequest::Http(
            DraftHttpProviderAdapterModelsRequest {
                provider_id: params.provider_id,
                base_url: params.base_url,
                protocol_paths: ProviderCatalogProtocolPaths {
                    openai: params.protocol_paths.openai,
                    anthropic: params.protocol_paths.anthropic,
                    gemini: params.protocol_paths.gemini,
                },
                api_key: params.api_key.map(|source| match source {
                    ProviderSecretSource::Inline(value) => ProviderApiKeySource::Inline(value),
                    ProviderSecretSource::Env(value) => ProviderApiKeySource::Environment(value),
                }),
                adapter_ids: params.adapter_ids,
            },
        ))
        .await
        .map_err(map_provider_catalog_error)?;
    Ok(provider_adapter_models_response(state, listing))
}

pub async fn list_saved_provider_adapter_models_response(
    state: &Application,
    params: ListSavedProviderAdapterModelsParams,
) -> Result<ProviderAdapterModelsResponse, ApplicationError> {
    let provider_id = ProviderId::new(params.provider_id);
    let listing = state
        .provider_catalog()
        .list_saved_adapter_models(&provider_id, params.adapter_ids)
        .await
        .map_err(map_provider_catalog_error)?;
    Ok(provider_adapter_models_response(state, listing))
}

pub(crate) fn provider_adapter_models_response(
    app: &Application,
    adapter_models: agena_provider::ProviderAdapterModelsListing,
) -> ProviderAdapterModelsResponse {
    let lookup_ids = adapter_models
        .adapters
        .iter()
        .flat_map(|adapter| adapter.models.iter())
        .flat_map(|model| catalog_lookup_candidates(model.id.as_ref()))
        .collect::<Vec<_>>();
    let catalog_entries = app.lookup_model_catalog_models(&lookup_ids);
    ProviderAdapterModelsResponse {
        provider_id: adapter_models.provider_id,
        adapters: adapter_models
            .adapters
            .into_iter()
            .map(|adapter| provider_adapter_models_resource(adapter, &catalog_entries))
            .collect(),
    }
}

/// Configured (saved) adapter models for a provider, enriched from the model
/// catalog. The Provider Studio opens and re-opens after save with this view,
/// so its rows carry the same complete display data as a live listing rather
/// than bare routes.
pub fn configured_provider_adapter_models_response(
    app: &Application,
    provider_id: Option<&str>,
) -> Vec<ProviderAdapterModelsResource> {
    let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let Some(routing) = app
        .provider_catalog()
        .configured_routing(&ProviderId::new(provider_id))
    else {
        return Vec::new();
    };
    let lookup_ids = routing
        .adapters
        .iter()
        .flat_map(|adapter| adapter.model_ids.iter())
        .flat_map(|model_id| catalog_lookup_candidates(model_id))
        .collect::<Vec<_>>();
    let catalog_entries = app.lookup_model_catalog_models(&lookup_ids);
    routing
        .adapters
        .into_iter()
        .map(|adapter| {
            let adapter_id = adapter.adapter_id.clone();
            let mut models = adapter
                .model_ids
                .into_iter()
                .map(|model_id| {
                    let mut model = agena_domain::Model::new(provider_id, model_id);
                    model.adapter_id = Some(agena_domain::AdapterId::new(adapter_id.as_str()));
                    provider_model_resource_from_domain(enrich_listing_model_from_catalog(
                        model,
                        &catalog_entries,
                    ))
                })
                .collect::<Vec<_>>();
            sort_models_alphabetically(&mut models);
            ProviderAdapterModelsResource {
                adapter_id: adapter_id.clone(),
                enabled: adapter.enabled,
                resolved_base_url: None,
                models,
                failure: None,
            }
        })
        .collect()
}

/// Enrich a raw listing model with its preferred catalog entry. Returns the
/// model unchanged when no catalog entry matches. The adapter-merged listing
/// model's own capabilities/metadata act as the fallback, so existing
/// (non-unknown) values win and only gaps are filled from the catalog —
/// matching the session path's `CatalogedModelsProvider` merge.
fn enrich_listing_model_from_catalog(
    model: agena_domain::Model,
    catalog_entries: &[crate::dto::CatalogModelResource],
) -> agena_domain::Model {
    let lookup_ids = catalog_lookup_candidates(model.id.as_ref());
    match preferred_catalog_model_for_lookup_ids(catalog_entries, &lookup_ids) {
        Some(catalog_model) => {
            let definition = catalog_model_to_catalog_definition(catalog_model);
            agena_provider::apply_catalog_definition_as_baseline(
                &definition,
                &model.capabilities.clone(),
                &model.metadata.clone(),
                model,
            )
        }
        None => model,
    }
}

fn provider_adapter_models_resource(
    value: ProviderAdapterModelsEntry,
    catalog_entries: &[crate::dto::CatalogModelResource],
) -> ProviderAdapterModelsResource {
    let mut models = value
        .models
        .into_iter()
        .map(|model| {
            provider_model_resource_from_domain(enrich_listing_model_from_catalog(
                model,
                catalog_entries,
            ))
        })
        .collect::<Vec<_>>();
    sort_models_alphabetically(&mut models);
    ProviderAdapterModelsResource {
        adapter_id: value.adapter_id,
        enabled: value.enabled,
        resolved_base_url: value.resolved_base_url,
        models,
        failure: value.failure.map(Into::into),
    }
}

/// Sort a model listing alphabetically by id (case-insensitive). Provider
/// `list_models` responses are in arbitrary order, and the Provider Studio
/// shows the live list directly, so the rows are ordered here for display.
fn sort_models_alphabetically(models: &mut [ProviderModelResource]) {
    models.sort_by_key(|model| model.id.to_lowercase());
}

fn map_provider_catalog_error(error: ProviderCatalogError) -> ApplicationError {
    match error {
        ProviderCatalogError::InvalidRequest(message) => {
            ApplicationError::bad_request_with_diagnostic(
                "The provider request is invalid.",
                message,
            )
        }
        ProviderCatalogError::NotFound(message) => {
            ApplicationError::not_found_with_diagnostic("The provider was not found.", message)
        }
        ProviderCatalogError::Operation(message) => ApplicationError::internal(message),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AwsProfileFileKind, enrich_listing_model_from_catalog, parse_aws_profile_names,
        sort_models_alphabetically,
    };
    use agena_domain::{Model, ReasoningEffort, ThinkingRequest};
    use agena_provider::{CatalogModelRecord, ConfiguredModelThinkingMode};

    fn catalog_model_with_thinking_modes() -> crate::dto::CatalogModelResource {
        let mut modes = Vec::new();
        for effort in [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ] {
            modes.push(ConfiguredModelThinkingMode {
                is_default: Some(effort == ReasoningEffort::High),
                thinking: Some(ThinkingRequest::Effort { effort }),
                ..Default::default()
            });
        }
        crate::dto::CatalogModelResource::from_record(
            CatalogModelRecord {
                model_id: "deepseek-v4-pro".to_owned(),
                context_window_tokens: Some(1_048_576),
                thinking_modes: modes.into(),
                ..Default::default()
            },
            None,
        )
    }

    #[test]
    fn listing_model_is_enriched_with_catalog_modes_and_limits() {
        let catalog_entries = vec![catalog_model_with_thinking_modes()];
        let mut model = Model::new("cpa", "deepseek-v4-pro");
        model.display_name = Some("DeepSeek V4 Pro".to_owned());

        let enriched = enrich_listing_model_from_catalog(model, &catalog_entries);

        let selectors = enriched
            .thinking_modes
            .iter()
            .filter_map(|mode| mode.selector().map(|value| value.into_owned()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            selectors,
            ["low", "medium", "high", "max"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect()
        );
        assert_eq!(
            enriched.metadata.limits.context_window_tokens,
            Some(1_048_576)
        );
        assert_eq!(enriched.display_name.as_deref(), Some("DeepSeek V4 Pro"));
    }

    #[test]
    fn listing_model_without_catalog_entry_passes_through_unchanged() {
        let model = Model::new("cpa", "brand-new-model");
        let enriched = enrich_listing_model_from_catalog(model, &[]);
        assert_eq!(enriched.id.as_ref(), "brand-new-model");
        assert!(enriched.thinking_modes.is_empty());
    }

    #[test]
    fn model_listing_is_sorted_case_insensitively_by_id() {
        use agena_api::resource::ProviderModelResource;

        fn listing_model(id: &str) -> ProviderModelResource {
            ProviderModelResource::configured("openai_responses", id)
        }

        let mut models = vec![
            listing_model("o3-mini"),
            listing_model("GPT-5.1"),
            listing_model("gpt-4o-mini"),
            listing_model("O1"),
            listing_model("gpt-5"),
        ];
        sort_models_alphabetically(&mut models);
        let ids = models.into_iter().map(|model| model.id).collect::<Vec<_>>();
        assert_eq!(ids, ["gpt-4o-mini", "gpt-5", "GPT-5.1", "O1", "o3-mini",]);
    }

    #[test]
    fn aws_profile_sections_follow_credentials_and_config_file_rules() {
        let credentials = parse_aws_profile_names(
            "[default]\n[team.prod]\n[developer profile]\n",
            AwsProfileFileKind::Credentials,
        );
        assert_eq!(credentials, ["default", "developer profile", "team.prod"]);

        let config = parse_aws_profile_names(
            "[default]\n[profile team.prod]\n[PROFILE Developer Profile]\n\
             [sso-session company]\n[services local]\n",
            AwsProfileFileKind::Config,
        );
        assert_eq!(config, ["Developer Profile", "default", "team.prod"]);
    }
}
