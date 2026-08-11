pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        agena_application::provider_queries::list_providers_response(state.application()),
    ))
}

pub async fn list_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        agena_application::provider_queries::list_provider_models_response(
            state.application(),
            provider_id,
        )
        .await
        .map_err(server_error_from_application)?,
    ))
}

pub async fn list_provider_adapter_models(
    State(state): State<AppState>,
    Json(request): Json<ProviderAdapterModelsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        agena_application::provider_queries::list_provider_adapter_models_response(
            state.application(),
            ListProviderAdapterModelsParams {
                provider_id: request.provider_id,
                base_url: request.base_url,
                protocol_paths: request.protocol_paths,
                api_key: request.api_key,
                adapter_ids: request.adapter_ids,
            },
        )
        .await
        .map_err(server_error_from_application)?,
    ))
}

pub async fn list_saved_provider_adapter_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<SavedProviderAdapterModelsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        agena_application::provider_queries::list_saved_provider_adapter_models_response(
            state.application(),
            ListSavedProviderAdapterModelsParams {
                provider_id,
                adapter_ids: request.adapter_ids,
            },
        )
        .await
        .map_err(server_error_from_application)?,
    ))
}
use super::{
    AppState, IntoResponse, Json, ListProviderAdapterModelsParams,
    ListSavedProviderAdapterModelsParams, Path, ProviderAdapterModelsRequest,
    SavedProviderAdapterModelsRequest, ServerError, State, server_error_from_application,
};
