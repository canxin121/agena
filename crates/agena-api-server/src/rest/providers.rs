pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        Query::ListProviders,
        |result| match result {
            QueryResult::Providers(providers) => Some(providers),
            _ => None,
        },
        "providers query returned unexpected result",
    )
    .await
}

pub async fn list_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        Query::ListProviderModels(agena_api::queries::ListProviderModelsParams { provider_id }),
        |result| match result {
            QueryResult::ProviderModels(models) => Some(models),
            _ => None,
        },
        "provider models query returned unexpected result",
    )
    .await
}

pub async fn list_provider_adapter_models(
    State(state): State<AppState>,
    Json(request): Json<ProviderAdapterModelsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        Query::ListProviderAdapterModels(ListProviderAdapterModelsParams {
            provider_id: request.provider_id,
            base_url: request.base_url,
            protocol_paths: request.protocol_paths,
            api_key: request.api_key,
            adapter_ids: request.adapter_ids,
        }),
        |result| match result {
            QueryResult::ProviderAdapterModels(response) => Some(response),
            _ => None,
        },
        "provider adapter models query returned unexpected result",
    )
    .await
}

pub async fn list_saved_provider_adapter_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<SavedProviderAdapterModelsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        Query::ListSavedProviderAdapterModels(ListSavedProviderAdapterModelsParams {
            provider_id,
            adapter_ids: request.adapter_ids,
        }),
        |result| match result {
            QueryResult::ProviderAdapterModels(response) => Some(response),
            _ => None,
        },
        "saved provider adapter models query returned unexpected result",
    )
    .await
}
use super::{
    AppState, IntoResponse, Json, ListProviderAdapterModelsParams,
    ListSavedProviderAdapterModelsParams, Path, ProviderAdapterModelsRequest, Query, QueryResult,
    SavedProviderAdapterModelsRequest, ServerError, State, query_json,
};
