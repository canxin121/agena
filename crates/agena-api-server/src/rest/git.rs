use super::*;

pub async fn get_git_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().git_status(state.runtime())).await
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
