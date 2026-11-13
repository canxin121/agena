//! Provider and model catalog queries for the application layer.

use agena_api::{
    queries::{
        ListProviderAdapterModelsParams, ListSavedProviderAdapterModelsParams, ProviderSecretSource,
    },
    resource::{
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse,
        ProviderAdapterSummaryResource, ProviderModelCapabilitiesResource,
        ProviderModelMetadataResource, ProviderModelResource, ProviderModelSpeedModeResource,
        ProviderModelThinkingModeResource, ProviderModelsResponse, ProviderSummaryResource,
    },
};
use agena_domain::{Model, ModelCapabilities, ProviderId};
use agena_provider::{
    DraftHttpProviderAdapterModelsRequest, DraftProviderAdapterModelsRequest,
    ProviderAdapterModelsEntry, ProviderApiKeySource, ProviderCatalogError,
    ProviderProtocolPaths as ProviderCatalogProtocolPaths,
};

use crate::provider_studio::catalog::{
    catalog_lookup_candidates, catalog_model_for_route, catalog_model_to_catalog_definition,
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
    let models = match catalog
        .discover_models(&provider_id_value)
        .await
        .and_then(|discovery| {
            if discovery.models.is_empty() && !discovery.complete {
                return Err(agena_provider::ProviderCatalogError::Operation(
                    "Model discovery failed for the provider's adapters.".to_owned(),
                ));
            }
            Ok(discovery
                .models
                .into_iter()
                .map(|entry| entry.model)
                .collect::<Vec<_>>())
        }) {
        Ok(models) if !models.is_empty() => models,
        Ok(_) => catalog
            .configured_local_models(&provider_id_value)
            .map_err(|error| ApplicationError::internal_error(&error))?,
        Err(error) => {
            let fallback = catalog
                .configured_local_models(&provider_id_value)
                .map_err(|fallback_error| ApplicationError::internal_error(&fallback_error))?;
            if fallback.is_empty() {
                return Err(ApplicationError::internal_error(&error));
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
        .map_err(|error| ApplicationError::internal_error(&error))?;
    Ok(ProviderModelsResponse {
        provider_id,
        models: models
            .into_iter()
            .map(provider_model_resource_from_domain)
            .collect(),
    })
}

pub fn provider_model_resource_from_domain(value: Model) -> ProviderModelResource {
    let mut thinking_modes = value.thinking_modes;
    thinking_modes.sort_by(agena_domain::compare_thinking_mode_strength);
    ProviderModelResource {
        provider_id: value.provider_id.to_string(),
        adapter_id: value.adapter_id.map(|id| id.to_string()),
        id: value.id.to_string(),
        catalog_model_id: value.catalog_model_id.map(|id| id.to_string()),
        pinned_catalog_model_id: None,
        display_name: value.display_name,
        native_compaction: value.native_compaction,
        capabilities: provider_model_capabilities_resource_from_domain(value.capabilities),
        metadata: provider_model_metadata_resource_from_domain(value.metadata),
        thinking_modes: thinking_modes
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
        lifecycle: value.lifecycle,
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
        verbosity_levels: value.verbosity_levels,
        provider_default_verbosity: value.provider_default_verbosity,
        default_temperature: value.default_temperature,
        default_top_p: value.default_top_p,
        default_top_k: value.default_top_k,
        assistant_reasoning_interleaved: value.assistant_reasoning_interleaved,
        assistant_reasoning_field: value.assistant_reasoning_field,
        output_modalities: value.output_modalities,
        pricing: value.pricing,
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
        thinking: value.thinking,
        request_override: value.request_override,
        adapter_overrides: value.adapter_overrides,
    }
}

fn provider_model_speed_mode_resource_from_domain(
    value: agena_domain::ModelSpeedMode,
) -> ProviderModelSpeedModeResource {
    ProviderModelSpeedModeResource {
        is_default: value.is_default,
        display_name: value.display_name,
        description: value.description,
        request_override: value.request_override,
        adapter_overrides: value.adapter_overrides,
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
    Ok(provider_adapter_models_response_with_pins(
        state,
        listing,
        &saved_catalog_routes(state, provider_id.as_ref()),
    ))
}

/// Saved route definitions keyed by adapter and model id, for the routes that
/// carry any configuration. A saved listing applies the same
/// "route overrides the entry" merge a configured read uses, so a route's own
/// fields survive a live re-listing.
type SavedCatalogRoutes = std::collections::BTreeMap<(String, String), SavedCatalogRoute>;

#[derive(Clone, Default)]
struct SavedCatalogRoute {
    /// The entry the route names, if any. `None` means the route resolves the
    /// catalog automatically and only carries field overrides.
    catalog_model_id: Option<String>,
    /// `None` when the route has no saved definition at all.
    definition: Option<agena_provider::ConfiguredModelDefinition>,
}

fn saved_catalog_routes(app: &Application, provider_id: &str) -> SavedCatalogRoutes {
    app.provider_catalog()
        .configured_routing(&ProviderId::new(provider_id))
        .map(|routing| {
            routing
                .adapters
                .into_iter()
                .flat_map(|adapter| {
                    let adapter_id = adapter.adapter_id;
                    adapter
                        .models
                        .into_iter()
                        .map(move |(model_id, definition)| {
                            let route = SavedCatalogRoute {
                                catalog_model_id: definition.catalog_model_id.clone(),
                                definition: Some(definition),
                            };
                            ((adapter_id.clone(), model_id), route)
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn provider_adapter_models_response(
    app: &Application,
    adapter_models: agena_provider::ProviderAdapterModelsListing,
) -> ProviderAdapterModelsResponse {
    provider_adapter_models_response_with_pins(app, adapter_models, &Default::default())
}

fn provider_adapter_models_response_with_pins(
    app: &Application,
    adapter_models: agena_provider::ProviderAdapterModelsListing,
    pins: &SavedCatalogRoutes,
) -> ProviderAdapterModelsResponse {
    let pinned_ids = pins
        .values()
        .filter_map(|route| route.catalog_model_id.clone());
    let lookup_ids = pinned_ids
        .chain(
            adapter_models
                .adapters
                .iter()
                .flat_map(|adapter| adapter.models.iter())
                .flat_map(|model| catalog_lookup_candidates(model.id.as_ref())),
        )
        .collect::<Vec<_>>();
    let catalog_entries = app.lookup_model_catalog_models(&lookup_ids);
    ProviderAdapterModelsResponse {
        provider_id: adapter_models.provider_id,
        adapters: adapter_models
            .adapters
            .into_iter()
            .map(|adapter| {
                let adapter_id = adapter.adapter_id.clone();
                provider_adapter_models_resource(adapter, &catalog_entries, |model_id| {
                    pins.get(&(adapter_id.clone(), model_id.to_owned()))
                        .cloned()
                })
            })
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
        // The entry a route names is not necessarily a candidate for its model
        // id, so it has to be looked up explicitly too.
        .chain(
            routing
                .adapters
                .iter()
                .flat_map(|adapter| adapter.models.values())
                .filter_map(|definition| definition.catalog_model_id.clone()),
        )
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
                    let mut model = agena_domain::Model::new(provider_id, model_id.as_str());
                    model.adapter_id = Some(agena_domain::AdapterId::new(adapter_id.as_str()));
                    let saved = adapter.models.get(model_id.as_str());
                    let mut resource = provider_model_resource_from_domain(
                        enrich_configured_model_from_catalog(model, saved, &catalog_entries),
                    );
                    // Report the route's stored match so the studio can show
                    // it as the route's own choice after a reload.
                    resource.pinned_catalog_model_id =
                        saved.and_then(|definition| definition.catalog_model_id.clone());
                    resource
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

/// Enrich a saved route with the catalog entry it resolves to.
///
/// The catalog entry is the base and the route's saved fields override it, so
/// writing only `catalog_model_id` mirrors the entry and adding fields next to
/// it overrides just those. Resolution honours the route's explicit match
/// first, so a corrected match is what the editor reads back rather than the
/// id-derived suggestion.
fn enrich_configured_model_from_catalog(
    model: agena_domain::Model,
    saved: Option<&agena_provider::ConfiguredModelDefinition>,
    catalog_entries: &[crate::dto::CatalogModelResource],
) -> agena_domain::Model {
    let lookup_ids = catalog_lookup_candidates(model.id.as_ref());
    let catalog_model = catalog_model_for_route(
        catalog_entries,
        saved.and_then(|definition| definition.catalog_model_id.as_deref()),
        &lookup_ids,
    );
    let catalog_definition = catalog_model.map(catalog_model_to_catalog_definition);

    // The catalog entry supplies the baseline; the route supplies overrides.
    let mut model = match &catalog_definition {
        Some(definition) => {
            let mut model = agena_provider::apply_catalog_definition_as_baseline(
                definition,
                &model.capabilities.clone(),
                &model.metadata.clone(),
                model,
            );
            // A display name has no capability or metadata slot, so the
            // baseline merge cannot carry it. The entry supplies one unless the
            // route names the model itself.
            if let Some(display_name) = definition.display_name.as_deref() {
                model.display_name = Some(display_name.to_owned());
            }
            model
        }
        None => model,
    };
    if let Some(saved) = saved {
        model = agena_provider::apply_configured_definition_as_authority(
            saved,
            &model.capabilities.clone(),
            &model.metadata.clone(),
            model,
        );
        agena_provider::apply_configured_display_name(saved, &mut model, false);
    }
    model
}

/// Enrich a raw listing model with the catalog entry it resolves to. A saved
/// route's explicit match wins over the automatic one derived from its id.
/// Returns the model unchanged when no catalog entry matches. The
/// adapter-merged listing model's own capabilities/metadata act as the
/// fallback, so existing (non-unknown) values win and only gaps are filled from
/// the catalog — matching the session path's `CatalogedModelsProvider` merge.
fn enrich_listing_model_from_catalog(
    model: agena_domain::Model,
    catalog_entries: &[crate::dto::CatalogModelResource],
    pinned_model_id: Option<&str>,
) -> agena_domain::Model {
    let lookup_ids = catalog_lookup_candidates(model.id.as_ref());
    match catalog_model_for_route(catalog_entries, pinned_model_id, &lookup_ids) {
        Some(catalog_model) => {
            let definition = catalog_model_to_catalog_definition(catalog_model);
            if pinned_model_id.is_some() {
                // The listing model was already filled from the automatic
                // match, so this correction has to win over those values. The
                // provider's own listing still supplies what the entry omits.
                return agena_provider::apply_catalog_definition_as_authority(
                    &definition,
                    &model.capabilities.clone(),
                    &model.metadata.clone(),
                    model,
                );
            }
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
    saved_route_for: impl Fn(&str) -> Option<SavedCatalogRoute>,
) -> ProviderAdapterModelsResource {
    let mut models = value
        .models
        .into_iter()
        .map(|model| {
            let saved = saved_route_for(model.id.as_ref());
            let pinned = saved
                .as_ref()
                .and_then(|saved| saved.catalog_model_id.as_deref());
            let mut model = enrich_listing_model_from_catalog(model, catalog_entries, pinned);
            if let Some(saved) = &saved {
                if let Some(catalog_model_id) = &saved.catalog_model_id {
                    model.catalog_model_id =
                        Some(agena_domain::ModelId::new(catalog_model_id.as_str()));
                }
                if let Some(definition) = &saved.definition {
                    model = agena_provider::apply_configured_definition_as_authority(
                        definition,
                        &model.capabilities.clone(),
                        &model.metadata.clone(),
                        model,
                    );
                    agena_provider::apply_configured_display_name(definition, &mut model, false);
                }
            }
            let mut resource = provider_model_resource_from_domain(model);
            resource.pinned_catalog_model_id = saved.and_then(|saved| saved.catalog_model_id);
            resource
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
    use super::enrich_configured_model_from_catalog;
    use agena_api::resource::{CatalogModelResource, ModelCatalogSourceKind};
    use agena_provider::ConfiguredModelDefinition;

    fn catalog_entry(
        model_id: &str,
        display_name: &str,
        context_window: u32,
    ) -> CatalogModelResource {
        CatalogModelResource {
            model_id: model_id.to_owned(),
            source: ModelCatalogSourceKind::Generated,
            provider_sources: Vec::new(),
            verbosity_by_provider: Default::default(),
            source_label: None,
            display_name: Some(display_name.to_owned()),
            origin: None,
            lifecycle: None,
            context_window_tokens: Some(context_window),
            max_input_tokens: None,
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
            thinking_modes: Default::default(),
            speed_modes: Default::default(),
            capabilities: Default::default(),
        }
    }

    fn saved_route(pinned: Option<&str>) -> ConfiguredModelDefinition {
        ConfiguredModelDefinition {
            display_name: Some("Saved name".to_owned()),
            context_window_tokens: Some(777_777),
            catalog_model_id: pinned.map(ToOwned::to_owned),
            ..Default::default()
        }
    }

    fn enriched(
        saved: &ConfiguredModelDefinition,
        entries: &[CatalogModelResource],
    ) -> agena_domain::Model {
        let model = agena_domain::Model::new("fixture", "route-model");
        enrich_configured_model_from_catalog(model, Some(saved), entries)
    }

    #[test]
    fn a_route_without_its_own_fields_reads_the_catalog_entry() {
        let entries = [catalog_entry("route-model", "Catalog name", 1_000_000)];
        let model = enriched(&ConfiguredModelDefinition::default(), &entries);

        assert_eq!(model.display_name.as_deref(), Some("Catalog name"));
        assert_eq!(model.metadata.limits.context_window_tokens, Some(1_000_000));
    }

    #[test]
    fn route_fields_override_the_catalog_entry() {
        let entries = [catalog_entry("route-model", "Catalog name", 1_000_000)];
        let model = enriched(&saved_route(None), &entries);

        assert_eq!(model.display_name.as_deref(), Some("Saved name"));
        assert_eq!(model.metadata.limits.context_window_tokens, Some(777_777));
    }

    #[test]
    fn an_unset_route_field_falls_back_to_the_catalog_entry() {
        // Only the name is overridden; the limit still tracks the entry, so a
        // catalog refresh reaches an otherwise overridden route.
        let entries = [catalog_entry("route-model", "Catalog name", 1_000_000)];
        let saved = ConfiguredModelDefinition {
            display_name: Some("Saved name".to_owned()),
            ..Default::default()
        };
        let model = enriched(&saved, &entries);

        assert_eq!(model.display_name.as_deref(), Some("Saved name"));
        assert_eq!(model.metadata.limits.context_window_tokens, Some(1_000_000));
    }

    #[test]
    fn naming_a_catalog_entry_replaces_the_automatic_match() {
        let entries = [
            catalog_entry("route-model", "Automatic name", 1_000_000),
            catalog_entry("chosen-entry", "Chosen name", 2_000_000),
        ];
        let saved = ConfiguredModelDefinition {
            catalog_model_id: Some("chosen-entry".to_owned()),
            ..Default::default()
        };
        let model = enriched(&saved, &entries);

        assert_eq!(model.display_name.as_deref(), Some("Chosen name"));
        assert_eq!(model.metadata.limits.context_window_tokens, Some(2_000_000));
    }
}
