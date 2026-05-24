use super::*;

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
        entry_count: catalog.entries.len(),
    }
}

fn model_catalog_entry_resources(
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> Vec<crate::local_api::ModelCatalogEntryResource> {
    catalog
        .entries
        .iter()
        .cloned()
        .map(|entry| {
            crate::local_api::ModelCatalogEntryResource::from_record(
                entry,
                catalog.last_successful_source,
            )
        })
        .collect()
}

fn model_catalog_entry_search_text(entry: &crate::local_api::ModelCatalogEntryResource) -> String {
    let thinking_mode_text = entry
        .thinking_modes
        .iter()
        .flat_map(|(name, mode)| {
            [
                name.clone(),
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
    let speed_mode_text = entry
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
        entry.model_id.clone(),
        entry.display_name.clone().unwrap_or_default(),
        entry.origin.clone().unwrap_or_default(),
        entry.description.clone().unwrap_or_default(),
        entry.output_modalities.join(","),
        entry
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        entry
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        entry
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        entry
            .pricing
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok())
            .unwrap_or_default(),
        entry.default_temperature.clone().unwrap_or_default(),
        entry.default_top_p.clone().unwrap_or_default(),
        entry
            .default_top_k
            .map(|value| value.to_string())
            .unwrap_or_default(),
        match entry.source {
            crate::local_api::ModelCatalogSourceKind::Generated => "generated".to_owned(),
            crate::local_api::ModelCatalogSourceKind::Cache => "cache".to_owned(),
        },
        entry.source_label.clone().unwrap_or_default(),
        entry
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
        let mut origins = model_catalog_entry_resources(&catalog)
            .into_iter()
            .filter_map(|entry| {
                let origin = entry.origin.unwrap_or_default();
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

    let filtered = model_catalog_entry_resources(&catalog)
        .into_iter()
        .filter(|entry| {
            if let Some(origin_filter) = origin_filter
                && entry.origin.as_deref().map(str::trim) != Some(origin_filter)
            {
                return false;
            }
            if let Some(search) = search.as_deref() {
                return model_catalog_entry_search_text(entry).contains(search);
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
    let items = model_catalog_entry_resources(&catalog)
        .into_iter()
        .filter(|entry| requested.contains(entry.model_id.as_str()))
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
