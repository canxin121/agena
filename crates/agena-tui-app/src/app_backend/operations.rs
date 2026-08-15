//! Session and workspace operations the terminal drives through
//! `TuiBackend`. These are presentation-facing entry points that build on
//! the HTTP client (the down-moved frozen methods live on `TuiBackend`
//! itself; this module holds everything else).

use anyhow::{Context, Result};

use agena_api::resource::{
    PermissionReply as ApiPermissionReply, PermissionReplyKind as ApiPermissionReplyKind,
    PermissionScope as ApiPermissionScope, ProviderSummaryResource, RunOptions,
    SessionExecutionResource, SessionResource, UserInputReply as ApiUserInputReply,
    UserInputReplyKind as ApiUserInputReplyKind,
};
use agena_domain::{PermissionReplyKind, PermissionScope, UserInputReply};

use super::TuiBackend;

/// Load usage statistics from the server for the terminal's usage
/// overview.
pub(crate) async fn usage_stats(
    application: &TuiBackend,
    query: agena_domain::UsageStatsQuery,
) -> Result<agena_domain::UsageStats> {
    application
        .client()
        .usage_stats(
            query.period,
            query.from,
            query.to,
            &query.provider_ids,
            &query.model_ids,
            &query.session_ids,
            query.include_subagents,
            query.timezone_offset_minutes,
        )
        .await
        .context("failed to load usage statistics")
}

/// Fetch the full session execution projection.
pub(crate) async fn get_session_state(
    application: &TuiBackend,
    session_id: i64,
) -> Result<SessionExecutionResource> {
    application
        .get_session_state(session_id)
        .await
        .context("failed to load session state")
}

/// Submit a user document (composer message) as a run.
pub(crate) async fn submit_document_with_options(
    application: &TuiBackend,
    session_id: i64,
    document: agena_domain::ComposerDocument,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    application
        .submit_document(session_id, document, request)
        .await
        .context("failed to submit user message")
}

/// Update the session's selected model/options without starting a run.
pub(crate) async fn update_session_selection(
    application: &TuiBackend,
    session_id: i64,
    options: RunOptions,
) -> Result<SessionExecutionResource> {
    application
        .update_session_selection(session_id, options)
        .await
        .context("failed to update session model selection")
}

/// Continue an existing session with the given run options.
pub(crate) async fn continue_session_with_options(
    application: &TuiBackend,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    application
        .continue_session(session_id, request)
        .await
        .context("failed to continue session")
}

/// Compact an existing session with the given run options.
pub(crate) async fn compact_session_with_options(
    application: &TuiBackend,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    application
        .compact_session(session_id, request)
        .await
        .context("failed to compact session")
}

/// Cancel the active run of `session_id`.
pub(crate) async fn cancel_run(
    application: &TuiBackend,
    session_id: i64,
    execution_id: agena_domain::ExecutionId,
) -> Result<agena_domain::CancellationResult> {
    application
        .cancel_run(session_id, execution_id)
        .await
        .context("failed to cancel active run")
}

/// Reply to a pending permission request.
pub(crate) async fn reply_permission_with_options(
    application: &TuiBackend,
    session_id: i64,
    request_id: String,
    kind: PermissionReplyKind,
    scope: Option<PermissionScope>,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    application
        .reply_permission(
            session_id,
            request,
            ApiPermissionReply {
                request_id,
                kind: match kind {
                    PermissionReplyKind::AllowOnce => ApiPermissionReplyKind::AllowOnce,
                    PermissionReplyKind::AllowAlways => ApiPermissionReplyKind::AllowAlways,
                    PermissionReplyKind::DenyOnce => ApiPermissionReplyKind::DenyOnce,
                    PermissionReplyKind::DenyAlways => ApiPermissionReplyKind::DenyAlways,
                    PermissionReplyKind::AutoApprove => ApiPermissionReplyKind::AutoApprove,
                },
                reason: None,
                scope: scope.map(|scope| match scope {
                    PermissionScope::Session => ApiPermissionScope::Session,
                    PermissionScope::Workspace => ApiPermissionScope::Workspace,
                    PermissionScope::Global => ApiPermissionScope::Global,
                }),
            },
        )
        .await
        .context("failed to reply to permission request")
}

/// Reply to a pending interactive user-input request.
pub(crate) async fn reply_user_input_with_options(
    application: &TuiBackend,
    session_id: i64,
    reply: UserInputReply,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    application
        .reply_user_input(
            session_id,
            request,
            ApiUserInputReply {
                request_id: reply.request_id,
                kind: match reply.kind {
                    agena_domain::UserInputReplyKind::Submit => ApiUserInputReplyKind::Submit,
                    agena_domain::UserInputReplyKind::Cancel => ApiUserInputReplyKind::Cancel,
                    agena_domain::UserInputReplyKind::Timeout => ApiUserInputReplyKind::Timeout,
                },
                answers: reply.answers,
                reason: reply.reason,
            },
        )
        .await
        .context("failed to submit user input reply")
}

/// Clone a session's full history into a new child session — a real fork,
/// unlike `create_session`, which starts an empty child.
pub(crate) async fn fork_session(
    application: &TuiBackend,
    session_id: i64,
    title: Option<String>,
) -> Result<SessionExecutionResource> {
    application
        .fork_session(session_id, title)
        .await
        .context("failed to fork session")
}

/// Durable, idempotent acknowledgement that an interactive user-input request
/// has been shown to the user.
pub(crate) async fn present_interactive_request(
    application: &TuiBackend,
    session_id: i64,
    request_id: String,
) -> Result<SessionExecutionResource> {
    application
        .mark_interactive_request_presented(session_id, request_id)
        .await
        .context("failed to mark interactive request presented")
}

/// Render the terminal diagnostic summary from the server's runtime
/// status projection.
pub(crate) async fn runtime_snapshot_summary(application: &TuiBackend) -> Result<String> {
    let status = application.client().runtime_status().await?;
    Ok(format!(
        "generation {} · loaded {} · {} providers · {} plugins",
        status.generation,
        status.loaded_at.to_rfc3339(),
        status.provider_ids.len(),
        status.plugin_count,
    ))
}

/// List a single page of workspace sessions for the session switcher.
pub(crate) async fn list_workspace_sessions_page(
    application: &TuiBackend,
    roots_only: bool,
    exclude_subagents: bool,
    search: Option<&str>,
    cursor: Option<String>,
    limit: u64,
) -> Result<agena_api::pagination::PaginatedResponse<SessionResource>> {
    application
        .list_workspace_sessions_page(roots_only, exclude_subagents, search, cursor, limit)
        .await
}

/// List all known providers (without adapter detail).
pub(crate) fn list_providers(application: &TuiBackend) -> Vec<ProviderSummaryResource> {
    application.provider_summaries()
}

/// List configured providers (with adapter detail).
pub(crate) fn list_configured_providers(application: &TuiBackend) -> Vec<ProviderSummaryResource> {
    application.provider_summaries()
}

/// Set a workspace-scoped config file setting, reloading the runtime when the
/// edit requires it.
pub(crate) async fn set_workspace_config_setting(
    application: &TuiBackend,
    path: &str,
    value: serde_json::Value,
) -> Result<agena_runtime::ConfigSettingsEditResponse> {
    application
        .set_workspace_config_setting(path, value)
        .await
        .context("failed to set workspace config setting")
}

/// Delete a workspace-scoped config file setting, reloading the runtime when
/// the edit requires it.
pub(crate) async fn delete_workspace_config_setting(
    application: &TuiBackend,
    path: &str,
) -> Result<agena_runtime::ConfigSettingsEditResponse> {
    application
        .delete_workspace_config_setting(path)
        .await
        .context("failed to delete workspace config setting")
}

/// Refresh provider client versions from the remote registry.
///
/// No server HTTP endpoint exposes this, so it degrades to a clear
/// unavailable error in remote client mode.
pub(crate) async fn refresh_provider_client_versions(
    application: &TuiBackend,
) -> Result<agena_provider::ProviderClientVersions> {
    let _ = application;
    anyhow::bail!(
        "provider client version refresh is unavailable in remote TUI mode until it has a public server API"
    )
}
