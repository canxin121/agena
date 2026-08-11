//! Provider/model presentation mappings: routing views, local model lists,
//! catalog lookups, and the inspector rows for think/speed/verbosity choices.

use agena_api::resource::{ProviderAdapterModelsResource, ProviderModelResource};
use agena_application::{
    Application,
    dto::{CatalogModelResource, ModelCatalogListResponse},
};
use agena_domain::Model as ProviderModel;
use agena_domain::{ModelRef, ProviderId};
use anyhow::{Result, anyhow};

use crate::app_backend::inspector::{InspectorRow, summarize_named_mode};

/// Enabled adapters and their configured model ids for `provider_id`.
pub(crate) fn configured_provider_model_routes(
    application: &Application,
    provider_id: Option<&str>,
) -> Vec<(String, String)> {
    let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    application
        .provider_catalog()
        .configured_routing(&ProviderId::new(provider_id))
        .into_iter()
        .flat_map(|provider| provider.adapters)
        .filter(|adapter| adapter.enabled)
        .flat_map(|adapter| {
            let adapter_id = adapter.adapter_id;
            adapter
                .model_ids
                .into_iter()
                .map(move |model_id| (adapter_id.clone(), model_id))
        })
        .collect()
}

/// Configured adapter model resources for `provider_id`.
pub(crate) fn configured_provider_adapter_models(
    application: &Application,
    provider_id: Option<&str>,
) -> Vec<ProviderAdapterModelsResource> {
    let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    application
        .provider_catalog()
        .configured_routing(&ProviderId::new(provider_id))
        .into_iter()
        .flat_map(|provider| provider.adapters)
        .map(|adapter| ProviderAdapterModelsResource {
            adapter_id: adapter.adapter_id.clone(),
            enabled: adapter.enabled,
            resolved_base_url: None,
            models: adapter
                .model_ids
                .into_iter()
                .map(|model_id| {
                    ProviderModelResource::configured(adapter.adapter_id.as_str(), model_id)
                })
                .collect(),
            failure: None,
        })
        .collect()
}

pub(crate) fn list_local_provider_models(
    application: &Application,
    provider_id: &str,
) -> Result<Vec<ProviderModel>> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Ok(Vec::new());
    }
    application
        .provider_catalog()
        .configured_local_models(&ProviderId::new(provider_id))
        .map_err(|error| anyhow!(error.to_string()))
}

pub(crate) fn model_display_name(application: &Application, model: &ModelRef) -> Option<String> {
    preferred_model_display_name(
        list_local_provider_models(application, model.provider_id.as_ref()).ok()?,
        model,
    )
}

pub(crate) fn list_model_catalog_models(
    application: &Application,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<ModelCatalogListResponse> {
    Ok(application.list_model_catalog_with_origin(query, None, offset, limit))
}

pub(crate) fn lookup_model_catalog_models(
    application: &Application,
    model_ids: &[String],
) -> Vec<CatalogModelResource> {
    application.lookup_model_catalog_models(model_ids)
}

/// Resolve the effective think-mode rows for the model implied by `request`.
pub(crate) fn runtime_thinking_mode_rows(
    application: &Application,
    request: &agena_api::resource::RunOptions,
) -> Result<Vec<InspectorRow>> {
    let model = application.resolved_model_for_run_options(request)?;
    model_thinking_mode_rows(application, &model)
}

pub(crate) fn model_thinking_mode_rows(
    application: &Application,
    model: &ModelRef,
) -> Result<Vec<InspectorRow>> {
    let mut modes = application
        .provider_catalog()
        .model_execution_options(model)
        .map_err(|error| anyhow!(error.to_string()))?
        .thinking_modes;
    modes.sort_by(agena_domain::compare_thinking_mode_strength);
    Ok(modes
        .into_iter()
        .filter_map(|mode| {
            Some(InspectorRow {
                label: mode.selector()?.into_owned(),
                detail: summarize_named_mode(
                    mode.display_name.as_deref(),
                    mode.description.as_deref(),
                ),
            })
        })
        .collect())
}

/// Resolve the effective speed-mode rows for the model implied by `request`.
pub(crate) fn runtime_speed_mode_rows(
    application: &Application,
    request: &agena_api::resource::RunOptions,
) -> Result<Vec<InspectorRow>> {
    let model = application.resolved_model_for_run_options(request)?;
    model_speed_mode_rows(application, &model)
}

pub(crate) fn model_speed_mode_rows(
    application: &Application,
    model: &ModelRef,
) -> Result<Vec<InspectorRow>> {
    let mut rows = application
        .provider_catalog()
        .model_execution_options(model)
        .map_err(|error| anyhow!(error.to_string()))?
        .speed_modes
        .into_iter()
        .map(|(name, mode)| InspectorRow {
            label: name,
            detail: summarize_named_mode(mode.display_name.as_deref(), mode.description.as_deref()),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(rows)
}

/// Resolve the effective verbosity values for the model implied by `request`.
pub(crate) fn runtime_verbosity_values(
    application: &Application,
    request: &agena_api::resource::RunOptions,
) -> Result<Vec<String>> {
    let model = application.resolved_model_for_run_options(request)?;
    model_verbosity_values(application, &model)
}

pub(crate) fn model_verbosity_values(
    application: &Application,
    model: &ModelRef,
) -> Result<Vec<String>> {
    let metadata = application
        .provider_catalog()
        .model_execution_options(model)
        .map_err(|error| anyhow!(error.to_string()))?
        .metadata;
    Ok(metadata.supported_verbosity_levels_for_model(&model.model_id))
}

pub(crate) async fn refresh_model_catalog(application: &Application) -> Result<()> {
    application
        .refresh_model_catalog()
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(())
}

fn preferred_model_display_name(models: Vec<ProviderModel>, model: &ModelRef) -> Option<String> {
    models
        .into_iter()
        .find(|candidate| {
            candidate.id == model.model_id
                && model
                    .adapter_id
                    .as_ref()
                    .is_none_or(|adapter_id| candidate.adapter_id.as_ref() == Some(adapter_id))
        })
        .and_then(|candidate| candidate.display_name)
        .map(|display_name| display_name.trim().to_owned())
        .filter(|display_name| !display_name.is_empty())
}
