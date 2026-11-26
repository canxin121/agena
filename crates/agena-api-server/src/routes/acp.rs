use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};

use crate::{AppState, rest};

pub(crate) fn routes() -> Router<AppState> {
    let router = Router::new()
        .route(
            "/workspaces/{workspace_id}/sessions",
            post(rest::external_agent_workspace).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route("/sessions", post(rest::create_session))
        .route(
            "/sessions/{session_id}",
            get(rest::get_session).delete(rest::delete_session),
        )
        .route("/sessions/{session_id}/state", get(rest::get_session_state))
        .route(
            "/sessions/{session_id}/parts",
            get(rest::list_session_parts),
        )
        .route("/sessions/{session_id}/runs", get(rest::read_session_runs))
        .route("/sessions/{session_id}/plan", get(rest::get_session_plan))
        .route("/sessions/{session_id}/cost", get(rest::get_session_cost))
        .route(
            "/sessions/{session_id}/file-changes",
            get(rest::get_session_file_changes),
        )
        .route(
            "/sessions/{session_id}/permission",
            axum::routing::put(rest::replace_session_permission),
        )
        .route(
            "/sessions/{session_id}/agent-session",
            post(rest::external_agent_session),
        )
        .route(
            "/sessions/{session_id}/agent-options",
            post(rest::external_agent_options),
        )
        .route(
            "/sessions/{session_id}/agent-auth",
            get(rest::external_agent_live_auth).post(rest::external_agent_auth),
        )
        .route(
            "/sessions/{session_id}/agent-providers",
            get(rest::external_agent_providers),
        )
        .route(
            "/sessions/{session_id}/agent-work",
            get(rest::list_external_agent_work).post(rest::cancel_external_agent_work),
        )
        .route(
            "/sessions/{session_id}/agent-steer",
            post(rest::steer_external_agent),
        )
        .route(
            "/sessions/{session_id}/messages",
            post(rest::submit_message).layer(DefaultBodyLimit::max(96 * 1024 * 1024)),
        )
        .route("/sessions/{session_id}/continue", post(rest::continue_run))
        .route("/sessions/{session_id}/cancel", post(rest::cancel_run))
        .route(
            "/sessions/{session_id}/queue/{execution_id}",
            get(rest::queued_message_for_edit)
                .put(rest::edit_queued_message)
                .delete(rest::delete_queued_message),
        )
        .route(
            "/sessions/{session_id}/queue/{execution_id}/take",
            post(rest::take_queued_message),
        )
        .route(
            "/sessions/{session_id}/queue/pause",
            post(rest::set_message_queue_paused),
        )
        .route(
            "/sessions/{session_id}/queue/clear",
            post(rest::clear_message_queue),
        )
        .route(
            "/sessions/{session_id}/user-input-replies",
            post(rest::reply_user_input),
        )
        .route(
            "/sessions/{session_id}/permission-replies",
            post(rest::reply_permission),
        )
        .route(
            "/sessions/{session_id}/interactive/{request_id}/present",
            post(rest::mark_interactive_request_presented),
        )
        .route(
            "/sessions/{session_id}/changes/stream",
            get(rest::stream_session_changes),
        )
        .route(
            "/sessions/{session_id}/content/{resource_id}",
            get(rest::read_content),
        )
        .route(
            "/sessions/{session_id}/content/{resource_id}/text",
            get(rest::read_content_text),
        )
        .route(
            "/sessions/{session_id}/content/{resource_id}/stream",
            get(rest::stream_content),
        )
        .route(
            "/sessions/{session_id}/parts/{part_id}/tool-sections/{section}",
            get(rest::get_session_tool_detail),
        )
        .route(
            "/sessions/{session_id}/media",
            get(rest::download_session_media_file),
        )
        .route(
            "/sessions/{session_id}/attachments",
            post(rest::upload_session_file).layer(DefaultBodyLimit::max(72 * 1024 * 1024)),
        );
    #[cfg(feature = "sse")]
    let router = router.route("/changes/stream", get(crate::sse::handler));
    router.layer(axum::middleware::map_response(
        |mut response: axum::response::Response| async move {
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            response
        },
    ))
}
