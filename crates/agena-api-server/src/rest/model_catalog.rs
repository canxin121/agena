//! HTTP adapters for the Application-owned model catalog use cases.
//!
//! The API layer maps query/body parameters only. Catalog snapshot access,
//! resource projection, filtering, canonical-ID lookup, and user-requested
//! refresh all remain in `agena-application`.

use super::{AppState, AxumQuery, Deserialize, IntoResponse, Json, ServerError, State, items_json};
use agena_api::resource::ModelCatalogLookupRequest;

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for the model catalog listing.
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

pub async fn get_model_catalog(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<ModelCatalogListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let offset = query.offset.unwrap_or(0);
    let limit =
        agena_application::pagination::normalize_limit(query.limit.map(|value| value as u64))
            as usize;
    Ok(Json(state.list_model_catalog_with_origin(
        query.q.as_deref().unwrap_or_default(),
        query.origin.as_deref(),
        offset,
        limit,
    )))
}

pub async fn lookup_model_catalog(
    State(state): State<AppState>,
    Json(request): Json<ModelCatalogLookupRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(items_json(
        state.lookup_model_catalog_models(request.model_ids.as_slice()),
    ))
}

pub async fn refresh_model_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state.refresh_model_catalog().map_err(ServerError::from)?,
    ))
}
