//! Provider/model presentation mappings: routing views, local model lists,
//! catalog lookups, and the inspector rows for think/speed/verbosity choices.

use agena_api::resource::ProviderAdapterModelsResource;
use agena_application::dto::ModelCatalogListResponse;
use agena_domain::Model as ProviderModel;
use agena_domain::ModelRef;
use anyhow::{Context, Result, anyhow};

use crate::app_backend::inspector::{InspectorRow, summarize_named_mode};

/// Enabled adapters and their configured model ids for `provider_id`.
///
/// No processing-center endpoint exposes the in-process provider catalog's
/// configured-routing projection; the Provider Studio (the only consumer) is
/// unavailable in remote client mode, so this degrades to an empty list.
pub(crate) fn configured_provider_model_routes(
    application: &crate::TuiBackend,
    provider_id: Option<&str>,
) -> Vec<(String, String)> {
    let _ = (application, provider_id);
    Vec::new()
}

/// Configured adapter model resources for `provider_id`, enriched from the
/// model catalog so the Provider Studio draft shows complete display data.
///
/// Provider Studio has no public center API; degrade to an empty list.
pub(crate) fn configured_provider_adapter_models(
    application: &crate::TuiBackend,
    provider_id: Option<&str>,
) -> Vec<ProviderAdapterModelsResource> {
    let _ = (application, provider_id);
    Vec::new()
}

pub(crate) fn list_local_provider_models(
    application: &crate::TuiBackend,
    provider_id: &str,
) -> Result<Vec<ProviderModel>> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Ok(Vec::new());
    }
    application.list_local_provider_models(provider_id)
}

pub(crate) fn model_display_name(
    application: &crate::TuiBackend,
    model: &ModelRef,
) -> Option<String> {
    preferred_model_display_name(
        list_local_provider_models(application, model.provider_id.as_ref()).ok()?,
        model,
    )
}

/// The cached model-catalog listing. Synchronous: consumed while building the
/// settings studio inside the TUI event loop. The catalog workbench refreshes
/// the cache over HTTP (see `catalog_page`); until then this returns an empty
/// listing.
pub(crate) fn list_model_catalog_models(
    application: &crate::TuiBackend,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<ModelCatalogListResponse> {
    let _ = (query, offset, limit);
    Ok(application.model_catalog())
}

/// Fetch a model-catalog page from the center into the shared cache and return
/// the response, for the catalog workbench's paged browsing.
pub(crate) async fn catalog_page(
    application: &crate::TuiBackend,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<ModelCatalogListResponse> {
    application
        .refresh_model_catalog_cache(query, offset, limit)
        .await?;
    Ok(application.model_catalog())
}

/// Resolve the effective think-mode rows for the model implied by `request`.
pub(crate) fn runtime_thinking_mode_rows(
    application: &crate::TuiBackend,
    request: &agena_api::resource::RunOptions,
) -> Result<Vec<InspectorRow>> {
    let model = application.resolved_model_for_run_options(request)?;
    model_thinking_mode_rows(application, &model)
}

pub(crate) fn model_thinking_mode_rows(
    application: &crate::TuiBackend,
    model: &ModelRef,
) -> Result<Vec<InspectorRow>> {
    let mut modes = application
        .configured_model(model)
        .ok_or_else(|| anyhow!("model is not configured"))?
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
    application: &crate::TuiBackend,
    request: &agena_api::resource::RunOptions,
) -> Result<Vec<InspectorRow>> {
    let model = application.resolved_model_for_run_options(request)?;
    model_speed_mode_rows(application, &model)
}

pub(crate) fn model_speed_mode_rows(
    application: &crate::TuiBackend,
    model: &ModelRef,
) -> Result<Vec<InspectorRow>> {
    let mut rows = application
        .configured_model(model)
        .ok_or_else(|| anyhow!("model is not configured"))?
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
    application: &crate::TuiBackend,
    request: &agena_api::resource::RunOptions,
) -> Result<Vec<String>> {
    let model = application.resolved_model_for_run_options(request)?;
    model_verbosity_values(application, &model)
}

pub(crate) fn model_verbosity_values(
    application: &crate::TuiBackend,
    model: &ModelRef,
) -> Result<Vec<String>> {
    let metadata = application
        .configured_model(model)
        .ok_or_else(|| anyhow!("model is not configured"))?
        .metadata;
    Ok(metadata.supported_verbosity_levels_for_model(&model.model_id))
}

pub(crate) async fn refresh_model_catalog(application: &crate::TuiBackend) -> Result<()> {
    application
        .client()
        .refresh_model_catalog()
        .await
        .context("failed to refresh the model catalog through the center")?;
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
