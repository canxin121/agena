use crate::{error::ServerError, state::AppState};
use agena_api::integrations::*;
use axum::{
    Json,
    extract::{Path, State},
};
use uuid::Uuid;
fn error(e: agena_assistant::Error) -> ServerError {
    match e {
        agena_assistant::Error::Invalid(s) => ServerError::bad_request(s),
        agena_assistant::Error::Conflict(s) => ServerError::conflict(s),
        agena_assistant::Error::NotFound => {
            ServerError::not_found("integration not found in this workspace")
        }
        agena_assistant::Error::Unavailable(s) => ServerError::service_unavailable(s),
        agena_assistant::Error::Policy(s) => ServerError::bad_request_with_diagnostic(
            "invalid channel access policy; check identities, owners, role capabilities and limits",
            s,
        ),
        agena_assistant::Error::Storage(s) => ServerError::internal(s),
    }
}
pub async fn list_integrations(
    State(s): State<AppState>,
    Path(source): Path<i64>,
) -> Result<Json<IntegrationsSnapshot>, ServerError> {
    s.application()
        .assistant_service()?
        .integrations(source)
        .await
        .map(Json)
        .map_err(error)
}
pub async fn create_integration(
    State(s): State<AppState>,
    Path(source): Path<i64>,
    Json(r): Json<CreateIntegration>,
) -> Result<Json<IntegrationBinding>, ServerError> {
    s.application()
        .assistant_service()?
        .create_integration(source, r)
        .await
        .map(Json)
        .map_err(error)
}
pub async fn integration_detail(
    State(s): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
) -> Result<Json<IntegrationDetail>, ServerError> {
    s.application()
        .assistant_service()?
        .integration_detail(source, id)
        .await
        .map(Json)
        .map_err(error)
}
pub async fn control_integration(
    State(s): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
    Json(r): Json<IntegrationControl>,
) -> Result<Json<IntegrationBinding>, ServerError> {
    s.application()
        .assistant_service()?
        .control_integration(source, id, r)
        .await
        .map(Json)
        .map_err(error)
}

pub async fn update_channel_access(
    State(s): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
    Json(r): Json<ChannelPolicyUpdate>,
) -> Result<Json<IntegrationBinding>, ServerError> {
    s.application()
        .assistant_service()?
        .update_channel_policy(source, id, r)
        .await
        .map(Json)
        .map_err(error)
}

/// Read-only platform inventory for one project conversation's workspace.
pub async fn platform_inventory(
    State(s): State<AppState>,
    Path(source): Path<i64>,
) -> Result<Json<PlatformInventory>, ServerError> {
    s.application()
        .assistant_service()?
        .platform_inventory(source)
        .await
        .map(Json)
        .map_err(error)
}
