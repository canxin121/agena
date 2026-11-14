use crate::{error::ServerError, state::AppState};
use agena_api::integrations::*;
use axum::{
    Json,
    extract::{Path, State},
};
use uuid::Uuid;

pub async fn list_integrations(
    State(state): State<AppState>,
    Path(source): Path<i64>,
) -> Result<Json<IntegrationsSnapshot>, ServerError> {
    state
        .application()
        .integrations(source)
        .await
        .map(Json)
        .map_err(Into::into)
}

pub async fn create_integration(
    State(state): State<AppState>,
    Path(source): Path<i64>,
    Json(request): Json<CreateIntegration>,
) -> Result<Json<IntegrationBinding>, ServerError> {
    state
        .application()
        .create_integration(source, request)
        .await
        .map(Json)
        .map_err(Into::into)
}

pub async fn integration_detail(
    State(state): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
) -> Result<Json<IntegrationDetail>, ServerError> {
    state
        .application()
        .integration_detail(source, id)
        .await
        .map(Json)
        .map_err(Into::into)
}

pub async fn control_integration(
    State(state): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
    Json(request): Json<IntegrationControl>,
) -> Result<Json<IntegrationBinding>, ServerError> {
    state
        .application()
        .control_integration(source, id, request)
        .await
        .map(Json)
        .map_err(Into::into)
}

pub async fn update_channel_access(
    State(state): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
    Json(request): Json<ChannelPolicyUpdate>,
) -> Result<Json<IntegrationBinding>, ServerError> {
    state
        .application()
        .update_channel_access(source, id, request)
        .await
        .map(Json)
        .map_err(Into::into)
}

/// Read-only platform inventory for one conversation's integration scope.
pub async fn platform_inventory(
    State(state): State<AppState>,
    Path(source): Path<i64>,
) -> Result<Json<PlatformInventory>, ServerError> {
    state
        .application()
        .platform_inventory(source)
        .await
        .map(Json)
        .map_err(Into::into)
}
