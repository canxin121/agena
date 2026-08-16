pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        agena_application::provider_queries::list_providers_response(state.application()),
    ))
}

pub async fn refresh_provider_client_versions(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let versions = state
        .application()
        .refresh_provider_client_versions()
        .await
        .map_err(server_error_from_application)?;
    Ok(Json(serde_json::json!({
        "codex": versions.codex,
        "claude": versions.claude,
        "gemini": versions.gemini,
    })))
}

pub async fn list_aws_profile_names(
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(serde_json::json!({
        "profiles": agena_application::provider_queries::list_aws_profile_names(),
    })))
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

pub async fn list_configured_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        agena_application::provider_queries::list_configured_provider_models_response(
            state.application(),
            provider_id,
        )
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

pub async fn list_configured_provider_adapter_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        agena_application::provider_queries::configured_provider_adapter_models_response(
            state.application(),
            Some(provider_id.as_str()),
        ),
    ))
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct ProviderStudioDraftQuery {
    #[serde(default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioDraftModelsRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioModelDraftRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
    pub adapter_id: String,
    pub model_id: String,
    #[serde(default)]
    pub provider_model: Option<agena_api::resource::ProviderModelResource>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioSaveDraftRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
    #[serde(default)]
    pub adapter_model_lists: Vec<agena_api::resource::ProviderAdapterModelsResource>,
    #[serde(default)]
    pub selected_adapter_ids: Vec<String>,
    #[serde(default)]
    pub selected_model_keys: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioSaveAdapterRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
    pub adapter_models: agena_api::resource::ProviderAdapterModelsResource,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioSaveModelRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
    pub adapter_id: String,
    pub model_id: String,
    pub model_value: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioDeleteProviderRequest {
    pub provider_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioDeleteAdapterRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
    pub adapter_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioDeleteModelRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
    pub adapter_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProviderStudioAuthRequest {
    pub draft: agena_application::provider_studio::ProviderConfigDraft,
}

pub async fn get_provider_studio_draft(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<ProviderStudioDraftQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let draft = state
        .application()
        .provider_config_draft(query.provider_id.as_deref())
        .map_err(server_error_from_application)?;
    Ok(Json(draft))
}

pub async fn list_provider_studio_draft_models(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioDraftModelsRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let response = state
        .application()
        .list_draft_provider_adapter_models(&request.draft, &request.adapter_ids)
        .await
        .map_err(server_error_from_application)?;
    Ok(Json(response))
}

pub async fn get_provider_studio_model_draft(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioModelDraftRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let value = state
        .application()
        .provider_model_draft_value(
            &request.draft,
            request.adapter_id.as_str(),
            request.model_id.as_str(),
            request.provider_model.as_ref(),
        )
        .map_err(server_error_from_application)?;
    Ok(Json(serde_json::json!({ "value": value })))
}

pub async fn save_provider_studio_draft(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioSaveDraftRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .save_provider_draft(
                request.draft,
                &request.adapter_model_lists,
                &request.selected_adapter_ids,
                &request.selected_model_keys,
            )
            .await,
    ))
}

pub async fn save_provider_studio_adapter(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioSaveAdapterRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .save_provider_adapter_matches(request.draft, request.adapter_models)
            .await,
    ))
}

pub async fn save_provider_studio_model(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioSaveModelRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .save_provider_model_value(
                request.draft,
                request.adapter_id.as_str(),
                request.model_id.as_str(),
                request.model_value,
            )
            .await,
    ))
}

pub async fn delete_provider_studio_provider(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioDeleteProviderRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .delete_provider(request.provider_id.as_str())
            .await,
    ))
}

pub async fn delete_provider_studio_adapter(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioDeleteAdapterRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .delete_provider_adapter(request.draft, request.adapter_id.as_str())
            .await,
    ))
}

pub async fn delete_provider_studio_model(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioDeleteModelRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .delete_provider_model(
                request.draft,
                request.adapter_id.as_str(),
                request.model_id.as_str(),
            )
            .await,
    ))
}

pub async fn start_provider_studio_auth(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioAuthRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .start_provider_draft_auth(request.draft)
            .await,
    ))
}

pub async fn continue_provider_studio_auth(
    State(state): State<AppState>,
    Json(request): Json<ProviderStudioAuthRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .continue_provider_draft_auth(request.draft)
            .await,
    ))
}
use super::{
    AppState, AxumQuery, IntoResponse, Json, ListProviderAdapterModelsParams,
    ListSavedProviderAdapterModelsParams, Path, ProviderAdapterModelsRequest,
    SavedProviderAdapterModelsRequest, ServerError, State, server_error_from_application,
};
