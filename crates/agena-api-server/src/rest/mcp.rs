use agena_application::dto::{
    McpBearerCredentialDeleteQuery, McpBearerCredentialWriteRequest, McpCredentialMutationResource,
    McpOAuthDeleteQuery, McpOAuthFinishRequest, McpOAuthStartRequest, McpOAuthStartResource,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};

use crate::{AppState, error::ServerError};

pub async fn set_mcp_bearer_credential(
    State(state): State<AppState>,
    Path(server): Path<String>,
    Json(request): Json<McpBearerCredentialWriteRequest>,
) -> Result<Json<McpCredentialMutationResource>, ServerError> {
    Ok(Json(
        state
            .application()
            .set_mcp_bearer_credential(server.as_str(), request.token.as_str(), request.store)
            .map_err(ServerError::from)?,
    ))
}

pub async fn delete_mcp_bearer_credential(
    State(state): State<AppState>,
    Path(server): Path<String>,
    Query(query): Query<McpBearerCredentialDeleteQuery>,
) -> Result<Json<McpCredentialMutationResource>, ServerError> {
    Ok(Json(
        state
            .application()
            .delete_mcp_bearer_credential(server.as_str(), query.store)
            .map_err(ServerError::from)?,
    ))
}

pub async fn start_mcp_oauth(
    State(state): State<AppState>,
    Json(request): Json<McpOAuthStartRequest>,
) -> Result<Json<McpOAuthStartResource>, ServerError> {
    Ok(Json(
        state
            .application()
            .start_mcp_oauth(request)
            .await
            .map_err(ServerError::from)?,
    ))
}

pub async fn finish_mcp_oauth(
    State(state): State<AppState>,
    Json(request): Json<McpOAuthFinishRequest>,
) -> Result<Json<McpCredentialMutationResource>, ServerError> {
    Ok(Json(
        state
            .application()
            .finish_mcp_oauth(request)
            .await
            .map_err(ServerError::from)?,
    ))
}

pub async fn delete_mcp_oauth_credential(
    State(state): State<AppState>,
    Path(server): Path<String>,
    Query(query): Query<McpOAuthDeleteQuery>,
) -> Result<Json<McpCredentialMutationResource>, ServerError> {
    Ok(Json(
        state
            .application()
            .delete_mcp_oauth_credential(server.as_str(), query.revoke, query.url.as_deref())
            .await
            .map_err(ServerError::from)?,
    ))
}
