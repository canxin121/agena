use super::*;

pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(&state, Query::ListProviders).await? {
        QueryResult::Providers(providers) => Ok(Json(providers)),
        _ => unreachable!("providers query returned unexpected result"),
    }
}

pub async fn list_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(
        &state,
        Query::ListProviderModels(agena_api::queries::ListProviderModelsParams { provider_id }),
    )
    .await?
    {
        QueryResult::ProviderModels(models) => Ok(Json(models)),
        _ => unreachable!("provider models query returned unexpected result"),
    }
}

pub async fn list_provider_adapter_models(
    State(state): State<AppState>,
    Json(request): Json<ProviderAdapterModelsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(
        &state,
        Query::ListProviderAdapterModels(ListProviderAdapterModelsParams {
            provider_id: request.provider_id,
            base_url: request.base_url,
            protocol_paths: request.protocol_paths,
            api_key: request.api_key,
            api_key_env: request.api_key_env,
            adapter_ids: request.adapter_ids,
        }),
    )
    .await?
    {
        QueryResult::ProviderAdapterModels(response) => Ok(Json(response)),
        _ => unreachable!("provider adapter models query returned unexpected result"),
    }
}

pub async fn list_saved_provider_adapter_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<SavedProviderAdapterModelsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(
        &state,
        Query::ListSavedProviderAdapterModels(ListSavedProviderAdapterModelsParams {
            provider_id,
            adapter_ids: request.adapter_ids,
        }),
    )
    .await?
    {
        QueryResult::ProviderAdapterModels(response) => Ok(Json(response)),
        _ => unreachable!("saved provider adapter models query returned unexpected result"),
    }
}
