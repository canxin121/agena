pub async fn get_git_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().git_status(state.runtime())).await
}

pub async fn get_snapshot_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(state.service().snapshot_status(state.runtime())))
}

pub async fn init_git_repository(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().git_init(state.runtime())).await
}

pub async fn get_vcs_diff_raw(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    state
        .service()
        .vcs_diff_raw(state.runtime())
        .await
        .map_err(server_error_from_http)
}

pub async fn stage_git_changes(
    State(state): State<AppState>,
    Json(request): Json<GitStageRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().git_stage(state.runtime(), request)).await
}

pub async fn create_git_commit(
    State(state): State<AppState>,
    Json(request): Json<GitCommitRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().git_commit(state.runtime(), request)).await
}

pub async fn create_git_pull_request(
    State(state): State<AppState>,
    Json(request): Json<GitPullRequestCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .service()
            .git_create_pull_request(state.runtime(), request),
    )
    .await
}
use super::{
    AppState, GitCommitRequest, GitPullRequestCreateRequest, GitStageRequest, IntoResponse, Json,
    ServerError, State, json_http, server_error_from_http,
};
