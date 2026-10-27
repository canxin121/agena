// macOS ld can emit a non-actionable compact-unwind diagnostic for this large
// test binary; keep test output focused on actionable Rust diagnostics.

//! # agena-api-server
//!
//! Unified transport crate for Agena surfaces. It wires the shared [`agena_api`]
//! protocol and adjacent local protocols over feature-gated transports including
//! HTTP/REST, WebSocket, SSE, Unix-socket IPC, and JSON-RPC app-server entrypoints,
//! all backed by the same Runtime application-service event-stream boundary.
//!
//! ## Layout
//!
//! - [`state`]: shared `AppState` (runtime + manager + bus accessors).
//! - [`dispatch`]: command/query dispatch helpers — used by both REST and WS
//!   so that adding a new operation only requires touching one place.
//! - [`rest`]: HTTP routes that accept JSON bodies and return JSON responses.
//! - [`ws`]: `/api/v1/ws` upgrade handler implementing the
//!   [`agena_api::ws::ClientMessage`] / [`agena_api::ws::ServerMessage`]
//!   protocol with multiplexed subscriptions.
//! - [`sse`]: `/api/v1/changes/stream` push-only part-patch/signal stream.
//! - [`ipc`]: optional Unix socket binder reusing the same WS protocol.
//!
//! ## Design notes
//!
//! - The transports never poll the database. They consume Runtime's stable
//!   live-change surfaces. Catch-up is an explicit current parts snapshot;
//!   live notifications themselves are never persisted or replayed.
//! - Commands and queries route through `dispatch::*` so that the WS handler
//!   and REST handler share identical semantics where practical, while the
//!   REST surface keeps returning the plain JSON resources the Studio web UI
//!   already consumes.

pub mod dispatch;
pub mod error;
#[cfg(feature = "ipc")]
pub mod ipc;
#[cfg(any(feature = "http", feature = "ws", feature = "sse"))]
mod json_codec;
#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
#[cfg(any(feature = "http", feature = "ws", feature = "sse"))]
mod live;
#[cfg(feature = "http")]
pub mod rest;
#[cfg(feature = "http")]
mod revisions;
#[cfg(feature = "sse")]
pub mod sse;
pub mod state;
#[cfg(feature = "ws")]
pub mod ws;

pub use state::AppState;

use axum::Router;
#[cfg(any(feature = "http", feature = "ws", feature = "sse"))]
use axum::routing::get;
#[cfg(feature = "http")]
use axum::{
    extract::DefaultBodyLimit,
    middleware::{self, Next},
    routing::post,
};

/// Increment the per-process HTTP request counter and record the
/// request duration in the latency histogram.
#[cfg(feature = "http")]
async fn count_request(req: axum::extract::Request, next: Next) -> axum::response::Response {
    rest::METRIC_HTTP_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let elapsed_us = start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    rest::record_http_latency(elapsed_us);
    response
}

/// Build the v1 axum router with every transport mounted.
pub fn router(state: AppState) -> Router {
    #[cfg(feature = "http")]
    let router = {
        let router = Router::new();
        router
            .route("/healthz", get(rest::healthz))
            .route("/readyz", get(rest::readyz))
            .route("/metrics", get(rest::metrics))
            .route("/api/v1/health", get(rest::health))
            .route("/api/v1/changes/revisions", get(rest::resource_revisions))
            .route("/api/v1/runtime", get(rest::get_runtime_status))
            .route("/api/v1/usage", get(rest::get_usage_stats))
            .route("/api/v1/runtime/reload", post(rest::reload_runtime))
            .route(
                "/api/v1/runtime/tasks",
                get(rest::list_runtime_background_tasks),
            )
            .route(
                "/api/v1/runtime/tasks/{task_id}/cancel",
                post(rest::cancel_runtime_background_task),
            )
            .route(
                "/api/v1/settings",
                get(rest::get_settings)
                    .put(rest::set_settings)
                    .patch(rest::patch_settings)
                    .delete(rest::delete_settings),
            )
            .route(
                "/api/v1/settings/layers/{layer}",
                get(rest::get_layer_settings)
                    .put(rest::set_layer_settings)
                    .delete(rest::delete_layer_settings),
            )
            .route("/api/v1/settings/entries", get(rest::list_settings))
            .route("/api/v1/settings/sources", get(rest::get_settings_sources))
            .route("/api/v1/settings/validate", post(rest::validate_settings))
            .route("/api/v1/config/resolved", get(rest::get_resolved_config))
            .route("/api/v1/model-catalog", get(rest::get_model_catalog))
            .route(
                "/api/v1/model-catalog/lookup",
                post(rest::lookup_model_catalog),
            )
            .route(
                "/api/v1/model-catalog/match",
                post(rest::match_model_catalog),
            )
            .route(
                "/api/v1/model-catalog/refresh",
                post(rest::refresh_model_catalog),
            )
            .route(
                "/api/v1/sessions/{session_id}/media",
                get(rest::download_session_media_file),
            )
            .route("/api/v1/git/status", get(rest::get_git_status))
            .route("/api/v1/git/stage", post(rest::stage_git_changes))
            .route("/api/v1/git/commits", post(rest::create_git_commit))
            .route(
                "/api/v1/git/pull-requests",
                post(rest::create_git_pull_request),
            )
            .route("/api/v1/project/git/init", post(rest::init_git_repository))
            .route("/api/v1/vcs/diff/raw", get(rest::get_vcs_diff_raw))
            .route("/api/v1/memories", get(rest::list_memories))
            .route("/api/v1/memories/batch-delete", post(rest::delete_memories))
            .route("/api/v1/operator/tools", get(rest::list_operator_tools))
            .route(
                "/api/v1/operator/tools/invoke",
                post(rest::invoke_operator_tool),
            )
            .route("/api/v1/memories/overview", get(rest::get_memory_overview))
            .route("/api/v1/memories/index", post(rest::ensure_memory_index))
            .route(
                "/api/v1/memories/{name}",
                get(rest::get_memory)
                    .put(rest::save_memory)
                    .delete(rest::delete_memory),
            )
            .route(
                "/api/v1/skills",
                get(rest::list_skills).post(rest::create_skill),
            )
            .route("/api/v1/skills/validate", post(rest::validate_skill))
            .route(
                "/api/v1/skills/install/preview",
                post(rest::preview_skill_install).layer(DefaultBodyLimit::max(90 * 1024 * 1024)),
            )
            .route(
                "/api/v1/skills/install",
                post(rest::install_skills).layer(DefaultBodyLimit::max(90 * 1024 * 1024)),
            )
            .route(
                "/api/v1/skills/{id}",
                get(rest::get_skill)
                    .put(rest::update_skill)
                    .delete(rest::delete_skill),
            )
            .route(
                "/api/v1/skills/{id}/enabled",
                axum::routing::put(rest::set_skill_enabled),
            )
            .route(
                "/api/v1/skills/{id}/resource",
                get(rest::get_skill_resource)
                    .put(rest::write_skill_resource)
                    .delete(rest::delete_skill_resource),
            )
            .route("/api/v1/plugins", get(rest::list_plugins))
            .route(
                "/api/v1/plugins/surface",
                get(rest::get_plugin_surface_catalog),
            )
            .route(
                "/api/v1/plugins/architecture",
                get(rest::get_plugin_architecture_catalog),
            )
            .route(
                "/api/v1/plugins/tools/changes",
                get(rest::list_plugin_tool_registry_changes),
            )
            .route(
                "/api/v1/plugins/tools/invoke",
                post(rest::invoke_plugin_tool),
            )
            .route("/api/v1/plugins/{plugin_id}", get(rest::get_plugin))
            .route(
                "/api/v1/plugins/{plugin_id}/settings",
                get(rest::get_plugin_settings).put(rest::update_plugin_settings),
            )
            .route(
                "/api/v1/plugins/{plugin_id}/commands/{command_id}/invoke",
                post(rest::run_plugin_command),
            )
            .route(
                "/api/v1/plugins/{plugin_id}/logs",
                get(rest::list_plugin_logs),
            )
            .route(
                "/api/v1/plugins/marketplace/search",
                post(rest::search_marketplace_plugins),
            )
            .route(
                "/api/v1/plugins/marketplace/sync",
                post(rest::sync_marketplace_registry),
            )
            .route(
                "/api/v1/plugins/marketplace/installed",
                get(rest::list_marketplace_installed_plugins),
            )
            .route(
                "/api/v1/plugins/marketplace/outdated",
                get(rest::list_marketplace_outdated_plugins),
            )
            .route(
                "/api/v1/plugins/marketplace/install",
                post(rest::install_marketplace_plugin),
            )
            .route(
                "/api/v1/plugins/marketplace/uninstall",
                post(rest::uninstall_marketplace_plugin),
            )
            .route(
                "/api/v1/plugins/marketplace/upgrade",
                post(rest::upgrade_marketplace_plugins),
            )
            .route("/api/v1/auth/providers", get(rest::list_auth_providers))
            .route(
                "/api/v1/auth/providers/openai/browser/start",
                post(rest::start_openai_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/openai/browser/finish",
                post(rest::finish_openai_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/openai/device/start",
                post(rest::start_openai_device_auth),
            )
            .route(
                "/api/v1/auth/providers/openai/device/poll",
                post(rest::poll_openai_device_auth),
            )
            .route(
                "/api/v1/auth/providers/gitlab/browser/start",
                post(rest::start_gitlab_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/gitlab/browser/finish",
                post(rest::finish_gitlab_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/github-copilot/device/start",
                post(rest::start_copilot_device_auth),
            )
            .route(
                "/api/v1/auth/providers/github-copilot/device/poll",
                post(rest::poll_copilot_device_auth),
            )
            .route(
                "/api/v1/auth/providers/{provider_id}",
                get(rest::get_auth_provider).delete(rest::delete_auth_provider),
            )
            .route(
                "/api/v1/auth/providers/{provider_id}/api-key",
                axum::routing::put(rest::set_auth_provider_api_key),
            )
            .route(
                "/api/v1/auth/providers/{provider_id}/refresh",
                post(rest::refresh_auth_provider),
            )
            .route("/api/v1/providers", get(rest::list_providers))
            .route(
                "/api/v1/providers/client-versions/refresh",
                post(rest::refresh_provider_client_versions),
            )
            .route(
                "/api/v1/providers/aws-profiles",
                get(rest::list_aws_profile_names),
            )
            .route(
                "/api/v1/providers/models",
                post(rest::list_provider_adapter_models),
            )
            .route(
                "/api/v1/providers/{provider_id}/models",
                get(rest::list_provider_models).post(rest::list_saved_provider_adapter_models),
            )
            .route(
                "/api/v1/providers/{provider_id}/configured-models",
                get(rest::list_configured_provider_adapter_models),
            )
            .route(
                "/api/v1/providers/{provider_id}/configured-local-models",
                get(rest::list_configured_provider_models),
            )
            .route(
                "/api/v1/provider-studio/draft",
                get(rest::get_provider_studio_draft),
            )
            .route(
                "/api/v1/provider-studio/draft/models",
                post(rest::list_provider_studio_draft_models),
            )
            .route(
                "/api/v1/provider-studio/draft/model",
                post(rest::get_provider_studio_model_draft),
            )
            .route(
                "/api/v1/provider-studio/draft/model/catalog-template",
                post(rest::apply_provider_studio_catalog_template),
            )
            .route(
                "/api/v1/provider-studio/save",
                post(rest::save_provider_studio_draft),
            )
            .route(
                "/api/v1/provider-studio/save-adapter",
                post(rest::save_provider_studio_adapter),
            )
            .route(
                "/api/v1/provider-studio/save-model",
                post(rest::save_provider_studio_model),
            )
            .route(
                "/api/v1/provider-studio/delete-provider",
                post(rest::delete_provider_studio_provider),
            )
            .route(
                "/api/v1/provider-studio/delete-adapter",
                post(rest::delete_provider_studio_adapter),
            )
            .route(
                "/api/v1/provider-studio/delete-model",
                post(rest::delete_provider_studio_model),
            )
            .route(
                "/api/v1/provider-studio/auth/start",
                post(rest::start_provider_studio_auth),
            )
            .route(
                "/api/v1/provider-studio/auth/continue",
                post(rest::continue_provider_studio_auth),
            )
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
            .route(
                "/api/v1/sessions",
                get(rest::list_sessions).post(rest::create_session),
            )
            .route("/api/v1/sessions/overview", get(rest::session_overview))
            .route(
                "/api/v1/sessions/{session_id}",
                get(rest::get_session)
                    .put(rest::replace_session)
                    .delete(rest::delete_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/state",
                get(rest::get_session_state),
            )
            .route(
                "/api/v1/sessions/{session_id}/file-changes",
                get(rest::get_session_file_changes),
            )
            .route(
                "/api/v1/sessions/{session_id}/plan",
                get(rest::get_session_plan),
            )
            .route(
                "/api/v1/sessions/{session_id}/cost",
                get(rest::get_session_cost),
            )
            .route(
                "/api/v1/sessions/{session_id}/selection",
                axum::routing::put(rest::replace_session_selection),
            )
            .route(
                "/api/v1/sessions/{session_id}/permission",
                axum::routing::put(rest::replace_session_permission),
            )
            .route(
                "/api/v1/sessions/{session_id}/parts",
                get(rest::list_session_parts),
            )
            .route(
                "/api/v1/sessions/{session_id}/content/{resource_id}",
                get(rest::read_content),
            )
            .route(
                "/api/v1/sessions/{session_id}/content/{resource_id}/text",
                get(rest::read_content_text),
            )
            .route(
                "/api/v1/sessions/{session_id}/content/{resource_id}/stream",
                get(rest::stream_content),
            )
            .route(
                "/api/v1/sessions/{session_id}/parts/{part_id}/tool-sections/{section}",
                get(rest::get_session_tool_detail),
            )
            .route(
                "/api/v1/sessions/{session_id}/runs",
                get(rest::read_session_runs),
            )
            .route(
                "/api/v1/sessions/{session_id}/changes/stream",
                get(rest::stream_session_changes),
            )
            .route(
                "/api/v1/sessions/{session_id}/messages",
                post(rest::submit_message).layer(DefaultBodyLimit::max(96 * 1024 * 1024)),
            )
            .route(
                "/api/v1/sessions/{session_id}/continue",
                post(rest::continue_run),
            )
            .route(
                "/api/v1/sessions/{session_id}/compact",
                post(rest::compact_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/fork",
                post(rest::fork_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/btw",
                post(rest::ask_btw).layer(DefaultBodyLimit::max(32 * 1024)),
            )
            .route(
                "/api/v1/sessions/{session_id}/queue/{execution_id}",
                get(rest::queued_message_for_edit)
                    .put(rest::edit_queued_message)
                    .delete(rest::delete_queued_message),
            )
            .route(
                "/api/v1/sessions/{session_id}/queue/{execution_id}/take",
                post(rest::take_queued_message),
            )
            .route(
                "/api/v1/sessions/{session_id}/queue/pause",
                post(rest::set_message_queue_paused),
            )
            .route(
                "/api/v1/sessions/{session_id}/queue/clear",
                post(rest::clear_message_queue),
            )
            .route(
                "/api/v1/sessions/{session_id}/cancel",
                post(rest::cancel_run),
            )
            .route(
                "/api/v1/sessions/{session_id}/permission-replies",
                post(rest::reply_permission),
            )
            .route(
                "/api/v1/sessions/{session_id}/user-input-replies",
                post(rest::reply_user_input),
            )
            .route(
                "/api/v1/sessions/{session_id}/interactive/{request_id}/present",
                post(rest::mark_interactive_request_presented),
            )
            .route(
                "/api/v1/sessions/{session_id}/rewind",
                post(rest::rewind_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/export",
                get(rest::export_session),
            )
            .route(
                "/api/v1/sessions/import",
                post(rest::import_session).layer(DefaultBodyLimit::disable()),
            )
            .route(
                "/api/v1/sessions/tree/{root_id}",
                get(rest::list_session_tree),
            )
            .route(
                "/api/v1/permission-rules",
                get(rest::list_permission_rules).post(rest::create_permission_rule),
            )
            .route(
                "/api/v1/permission-rules/batch",
                post(rest::batch_permission_rules),
            )
            .route(
                "/api/v1/permission-rules/{rule_id}",
                get(rest::get_permission_rule)
                    .put(rest::replace_permission_rule)
                    .delete(rest::delete_permission_rule),
            )
            .route(
                "/api/v1/permission-rules/{rule_id}/revoke",
                post(rest::revoke_permission_rule),
            )
            .route("/api/v1/activities", get(rest::list_activities))
            .route(
                "/api/v1/activities/clear-finished",
                post(rest::clear_finished_activities),
            )
            .route("/api/v1/activities/{activity_id}", get(rest::get_activity))
            .route(
                "/api/v1/activities/{activity_id}/logs",
                get(rest::get_activity_logs),
            )
            .route(
                "/api/v1/activities/{activity_id}/stop",
                post(rest::stop_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/pause",
                post(rest::pause_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/resume",
                post(rest::resume_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/delete",
                post(rest::delete_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/dismiss",
                post(rest::dismiss_activity),
            )
            .route("/api/v1/notifications", get(rest::list_notifications))
            .route(
                "/api/v1/notifications/{notification_id}/dismiss",
                post(rest::dismiss_notification),
            )
            .route(
                "/api/v1/notifications/{notification_id}/actions/{action_id}",
                post(rest::resolve_notification_action),
            )
            .route("/plugin-rpc/{plugin_id}", post(rest::plugin_rpc))
            .layer(middleware::from_fn(count_request))
    };

    #[cfg(not(feature = "http"))]
    let router = Router::new();

    #[cfg(feature = "ws")]
    let router = router.route("/api/v1/ws", get(ws::handler));

    #[cfg(feature = "sse")]
    let router = router
        .route("/api/v1/changes/stream", get(sse::handler))
        .route(
            "/api/v1/notifications/stream",
            get(sse::notifications_stream),
        );

    router.with_state(state)
}

/// Build the streaming-only transport router for hosts that already mount
/// overlapping REST endpoints via another API surface.
pub fn transport_router(state: AppState) -> Router {
    let router = Router::new();

    #[cfg(feature = "ws")]
    let router = router.route("/api/v1/ws", get(ws::handler));

    #[cfg(feature = "sse")]
    let router = router
        .route("/api/v1/changes/stream", get(sse::handler))
        .route(
            "/api/v1/notifications/stream",
            get(sse::notifications_stream),
        );

    router.with_state(state)
}
