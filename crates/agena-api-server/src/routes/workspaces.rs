use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/workspaces",
            get(rest::list_workspaces).post(rest::create_workspace),
        )
        .route("/api/v1/workspaces/resolve", post(rest::resolve_workspace))
        .route(
            "/api/v1/workspaces/session-stats",
            get(rest::workspace_session_stats),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}",
            get(rest::get_workspace)
                .put(rest::replace_workspace)
                .delete(rest::delete_workspace),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/files",
            get(rest::list_workspace_files)
                .post(rest::upload_workspace_file)
                // 50 MiB raw files expand to about 66.7 MiB as base64; keep
                // enough room for the JSON envelope before service-level
                // decoded-size validation enforces the actual 50 MiB cap.
                .layer(DefaultBodyLimit::max(72 * 1024 * 1024)),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/download",
            get(rest::download_workspace_file),
        )
}
