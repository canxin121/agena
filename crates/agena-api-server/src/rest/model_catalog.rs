#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelCatalogListQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn model_catalog_summary(
    runtime: &agena::runtime::AgenaRuntime,
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> ModelCatalogResponse {
    ModelCatalogResponse {
        refreshing: runtime.model_catalog_refresh_active(),
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_error: catalog.last_error.clone(),
        model_count: catalog.models.len(),
    }
}

fn model_catalog_model_resources(
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> Vec<crate::local_api::CatalogModelResource> {
    catalog
        .models
        .iter()
        .cloned()
        .map(|model| {
            crate::local_api::CatalogModelResource::from_record(
                model,
                catalog.last_successful_source,
            )
        })
        .collect()
}

fn model_catalog_model_search_text(model: &crate::local_api::CatalogModelResource) -> String {
    let thinking_mode_text = model
        .thinking_modes
        .iter()
        .flat_map(|mode| {
            [
                agena::provider::configured_thinking_mode_selector(mode).unwrap_or_default(),
                mode.display_name.clone().unwrap_or_default(),
                mode.description.clone().unwrap_or_default(),
                mode.thinking
                    .as_ref()
                    .and_then(|value| serde_json::to_string(value).ok())
                    .unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");
    let speed_mode_text = model
        .speed_modes
        .iter()
        .flat_map(|(name, mode)| {
            [
                name.clone(),
                mode.display_name.clone().unwrap_or_default(),
                mode.description.clone().unwrap_or_default(),
                serde_json::to_string(&mode.request_override).unwrap_or_default(),
                serde_json::to_string(&mode.adapter_overrides).unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");

    [
        model.model_id.clone(),
        model.display_name.clone().unwrap_or_default(),
        model.origin.clone().unwrap_or_default(),
        model.description.clone().unwrap_or_default(),
        model.output_modalities.join(","),
        model
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        model
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        model
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        model
            .pricing
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok())
            .unwrap_or_default(),
        model.default_temperature.clone().unwrap_or_default(),
        model.default_top_p.clone().unwrap_or_default(),
        model
            .default_top_k
            .map(|value| value.to_string())
            .unwrap_or_default(),
        match model.source {
            crate::local_api::ModelCatalogSourceKind::Generated => "generated".to_owned(),
            crate::local_api::ModelCatalogSourceKind::Cache => "cache".to_owned(),
        },
        model.source_label.clone().unwrap_or_default(),
        model
            .lifecycle
            .map(|value| match value {
                agena::model::ModelLifecycle::Active => "active",
                agena::model::ModelLifecycle::Preview => "preview",
                agena::model::ModelLifecycle::Beta => "beta",
                agena::model::ModelLifecycle::Alpha => "alpha",
                agena::model::ModelLifecycle::Experimental => "experimental",
                agena::model::ModelLifecycle::Deprecated => "deprecated",
            })
            .unwrap_or_default()
            .to_owned(),
        thinking_mode_text,
        speed_mode_text,
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
}

pub async fn get_model_catalog(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<ModelCatalogListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let runtime = state.runtime();
    let snapshot = runtime.current_snapshot();
    let catalog = snapshot.model_catalog_response();
    let summary = model_catalog_summary(runtime, &catalog);
    let available_origins = {
        let mut origins = model_catalog_model_resources(&catalog)
            .into_iter()
            .filter_map(|model| {
                let origin = model.origin.unwrap_or_default();
                let trimmed = origin.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        origins.sort();
        origins
    };

    let search = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase());
    let origin_filter = query
        .origin
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "all");
    let offset = query.offset.unwrap_or(0);
    let limit = crate::local_api::normalize_limit(query.limit.map(|value| value as u64)) as usize;

    let filtered = model_catalog_model_resources(&catalog)
        .into_iter()
        .filter(|model| {
            if let Some(origin_filter) = origin_filter
                && model.origin.as_deref().map(str::trim) != Some(origin_filter)
            {
                return false;
            }
            if let Some(search) = search.as_deref() {
                return model_catalog_model_search_text(model).contains(search);
            }
            true
        })
        .collect::<Vec<_>>();
    let total = filtered.len();
    let items = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();

    Ok(Json(ModelCatalogListResponse {
        summary,
        total,
        offset,
        limit,
        available_origins,
        items,
    }))
}

pub async fn lookup_model_catalog(
    State(state): State<AppState>,
    Json(request): Json<ModelCatalogLookupRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let requested = request
        .model_ids
        .into_iter()
        .flat_map(|model_id| {
            let raw = model_id.trim().to_owned();
            if raw.is_empty() {
                return Vec::new();
            }
            let canonical = agena::model_catalog::canonical_model_catalog_id(raw.as_str());
            if canonical.is_empty() || canonical == raw {
                vec![raw]
            } else {
                vec![raw, canonical]
            }
        })
        .collect::<BTreeSet<_>>();
    let snapshot = state.runtime().current_snapshot();
    let catalog = snapshot.model_catalog_response();
    let items = model_catalog_model_resources(&catalog)
        .into_iter()
        .filter(|model| requested.contains(model.model_id.as_str()))
        .collect::<Vec<_>>();
    Ok(items_json(items))
}

pub async fn refresh_model_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let runtime = state.runtime();
    let task = runtime
        .start_model_catalog_refresh(agena::runtime::RuntimeBackgroundTaskOrigin::User)
        .map_err(super::server_error_from_runtime_background_task)?;
    let snapshot = runtime.current_snapshot();
    let catalog = snapshot.model_catalog_response();
    Ok(Json(ModelCatalogRefreshResponse {
        started: task.started,
        task: task.task.into(),
        summary: model_catalog_summary(runtime, &catalog),
    }))
}
use super::{
    AppState, AxumQuery, BTreeSet, Deserialize, IntoResponse, Json, ModelCatalogListResponse,
    ModelCatalogLookupRequest, ModelCatalogRefreshResponse, ModelCatalogResponse, ServerError,
    State, items_json,
};
