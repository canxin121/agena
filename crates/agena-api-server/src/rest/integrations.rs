use crate::{error::ServerError, state::AppState};
use agena_api::integrations::*;
use axum::{
    Json,
    extract::{Path, State},
};
use uuid::Uuid;
fn error(e: agena_superagent::Error) -> ServerError {
    match e {
        agena_superagent::Error::Invalid(s) => ServerError::bad_request(s),
        agena_superagent::Error::Conflict(s) => ServerError::conflict(s),
        agena_superagent::Error::NotFound => {
            ServerError::not_found("integration not found in this workspace")
        }
        agena_superagent::Error::Unavailable(s) => ServerError::service_unavailable(s),
        agena_superagent::Error::Policy(s) => ServerError::bad_request_with_diagnostic(
            "invalid channel access policy; check identities, owners, role capabilities and limits",
            s,
        ),
        agena_superagent::Error::Storage(s) => ServerError::internal(s),
    }
}
pub async fn list_integrations(
    State(s): State<AppState>,
    Path(source): Path<i64>,
) -> Result<Json<IntegrationsSnapshot>, ServerError> {
    s.application()
        .superagent()?
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
        .superagent()?
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
        .superagent()?
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
        .superagent()?
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
        .superagent()?
        .update_channel_policy(source, id, r)
        .await
        .map(Json)
        .map_err(error)
}
