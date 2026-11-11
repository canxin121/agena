use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/git/status", get(rest::get_git_status))
        .route("/api/v1/git/stage", post(rest::stage_git_changes))
        .route("/api/v1/git/commits", post(rest::create_git_commit))
        .route(
            "/api/v1/git/pull-requests",
            post(rest::create_git_pull_request),
        )
        .route("/api/v1/project/git/init", post(rest::init_git_repository))
        .route("/api/v1/vcs/diff/raw", get(rest::get_vcs_diff_raw))
}
