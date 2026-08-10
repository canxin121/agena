pub async fn get_git_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.git_status()).await
}

pub async fn get_snapshot_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.snapshot_status()).await
}

pub async fn init_git_repository(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.git_init()).await
}

pub async fn get_vcs_diff_raw(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    state
        .vcs_diff_raw()
        .await
        .map_err(server_error_from_application)
}

pub async fn stage_git_changes(
    State(state): State<AppState>,
    Json(request): Json<GitStageRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.git_stage(request)).await
}

pub async fn create_git_commit(
    State(state): State<AppState>,
    Json(request): Json<GitCommitRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.git_commit(request)).await
}

pub async fn create_git_pull_request(
    State(state): State<AppState>,
    Json(request): Json<GitPullRequestCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.git_create_pull_request(request)).await
}
use super::{
    AppState, GitCommitRequest, GitPullRequestCreateRequest, GitStageRequest, IntoResponse, Json,
    ServerError, State, json_http, server_error_from_application,
};
