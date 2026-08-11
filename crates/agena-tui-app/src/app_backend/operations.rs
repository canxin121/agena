//! Session and workspace operations the terminal drives through
//! `Application`. These are presentation-facing entry points that build on
//! the application service/session layer (the down-moved frozen methods live
//! on `Application` itself; this module holds everything else).

use agena_application::Application;
use anyhow::{Context, Result, anyhow};
use serde_json::Value as JsonValue;

use agena_api::resource::{
    PermissionReply as ApiPermissionReply, PermissionReplyKind as ApiPermissionReplyKind,
    PermissionScope as ApiPermissionScope, ProviderAdapterSummaryResource,
    ProviderDefaultsResource, ProviderSummaryResource, RunOptions, SessionExecutionResource,
    SessionResource, UserInputReply as ApiUserInputReply, UserInputReplyKind as ApiUserInputReplyKind,
};
use agena_domain::{PermissionReplyKind, PermissionScope, UserInputReply};

/// Load usage statistics for the terminal's usage overview.
pub(crate) async fn usage_stats(
    application: &Application,
    query: agena_domain::UsageStatsQuery,
) -> Result<agena_domain::UsageStats> {
    application
        .session_query_service()
        .map_err(anyhow::Error::new)?
        .usage_stats(query)
        .await
        .map_err(anyhow::Error::new)
        .context("failed to load usage statistics")
}

/// Fetch the full session execution projection.
pub(crate) async fn get_session_state(
    application: &Application,
    session_id: i64,
) -> Result<SessionExecutionResource> {
    let session_services = application.session_execution_services()?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to load session state")
}

/// Submit a user document (composer message) as a run.
pub(crate) async fn submit_document_with_options(
    application: &Application,
    session_id: i64,
    document: agena_domain::ComposerDocument,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    let request = agena_application::session::session_user_run_request(
        application,
        session_id,
        request,
        document,
    )
    .await?;
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .submit_user_run(request)
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to submit user message")
}

/// Update the session's selected model/options without starting a run.
pub(crate) async fn update_session_selection(
    application: &Application,
    session_id: i64,
    options: RunOptions,
) -> Result<SessionExecutionResource> {
    let options =
        agena_application::session::resolve_session_run_options(application, session_id, options)
            .await?;
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .update_session_selection(session_id, options)
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to update session model selection")
}

/// Continue an existing session with the given run options.
pub(crate) async fn continue_session_with_options(
    application: &Application,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    let request = agena_application::session::session_execution_request(application, session_id, request)
        .await?;
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .continue_session(request)
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to continue session")
}

/// Compact an existing session with the given run options.
pub(crate) async fn compact_session_with_options(
    application: &Application,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    let request = agena_application::session::session_execution_request(application, session_id, request)
        .await?;
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .compact_session(request)
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to compact session")
}

/// Cancel the active run of `session_id`.
pub(crate) async fn cancel_run(
    application: &Application,
    session_id: i64,
    execution_id: agena_domain::ExecutionId,
) -> Result<agena_domain::CancellationResult> {
    application
        .session_execution_services()
        .map_err(anyhow::Error::new)?
        .execution_control
        .cancel_execution(session_id, execution_id)
        .await
        .map_err(|error| {
            anyhow::Error::new(agena_application::ApplicationError::from_failure(error.failure))
        })
        .context("failed to cancel active run")
}

/// Inject `document` as a steer message into the active execution. Returns
/// `Err` when there is no active run or the run is in a phase that no longer
/// accepts steers (the caller should re-queue).
pub(crate) async fn steer_input(
    application: &Application,
    session_id: i64,
    document: agena_domain::ComposerDocument,
) -> Result<()> {
    application
        .session_execution_services()
        .map_err(anyhow::Error::new)?
        .commands
        .steer_input(session_id, document)
        .await
        .map_err(anyhow::Error::new)
        .context("failed to steer run")
}

/// Reply to a pending permission request.
pub(crate) async fn reply_permission_with_options(
    application: &Application,
    session_id: i64,
    request_id: String,
    kind: PermissionReplyKind,
    scope: Option<PermissionScope>,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    let request = agena_application::session::session_permission_reply_request(
        application,
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
        Some("jsonrpc".to_string()),
    )
    .await?;
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .reply_permission(request)
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to reply to permission request")
}

/// Reply to a pending interactive user-input request.
pub(crate) async fn reply_user_input_with_options(
    application: &Application,
    session_id: i64,
    reply: UserInputReply,
    request: RunOptions,
) -> Result<SessionExecutionResource> {
    let request = agena_application::session::session_user_input_reply_request(
        application,
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
    .await?;
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .reply_user_input(request)
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to submit user input reply")
}

/// Clone a session's full history into a new child session — a real fork,
/// unlike `create_session`, which starts an empty child.
pub(crate) async fn fork_session(
    application: &Application,
    session_id: i64,
    title: Option<String>,
) -> Result<SessionExecutionResource> {
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .fork_session(agena_runtime::SessionForkRequest {
            session_id,
            at_message_id: None,
            title,
            expected_version: None,
        })
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to fork session")
}

/// Durable, idempotent acknowledgement that an interactive user-input request
/// has been shown to the user.
pub(crate) async fn present_interactive_request(
    application: &Application,
    session_id: i64,
    request_id: String,
) -> Result<SessionExecutionResource> {
    let session_services = application.session_execution_services()?;
    let outcome = session_services
        .commands
        .mark_interactive_request_presented(session_id, request_id)
        .await
        .map_err(|error| agena_application::ApplicationError::from_failure(error.failure))?;
    agena_application::session::session_execution_resource(
        application,
        session_services.execution_control.as_ref(),
        session_services.queries.as_ref(),
        outcome.session_id,
    )
    .await
    .map_err(anyhow::Error::new)
    .context("failed to mark interactive request presented")
}

/// Render the terminal diagnostic summary from the Runtime-owned status
/// projection through Application rather than traversing Runtime status.
pub(crate) async fn runtime_snapshot_summary(application: &Application) -> Result<String> {
    let status = application.runtime_snapshot_summary().await;
    Ok(format!(
        "generation {} · loaded {} · {} providers · {} plugins",
        status.generation,
        status.loaded_at.to_rfc3339(),
        status.provider_count,
        status.plugin_count,
    ))
}

/// List a single page of workspace sessions for the session switcher.
pub(crate) async fn list_workspace_sessions_page(
    application: &Application,
    roots_only: bool,
    search: Option<&str>,
    cursor: Option<String>,
    limit: u64,
) -> Result<agena_api::pagination::PaginatedResponse<SessionResource>> {
    let workspace_id = current_workspace_id(application).await?;
    let page = application
        .service()
        .list_sessions(agena_application::dto::SessionListQuery {
            pagination: agena_application::dto::SearchPaginationQuery {
                pagination: agena_application::dto::CursorPaginationQuery {
                    cursor,
                    limit: Some(limit),
                },
                search: search.map(str::to_string),
            },
            workspace_id: Some(workspace_id),
            parent_id: None,
            roots: roots_only,
        })
        .await
        .map_err(anyhow::Error::new)
        .context("failed to list workspace sessions page")?;
    Ok(agena_application::pagination::api_page_from_application(
        page,
        |item| item,
    ))
}

/// List all known providers (without adapter detail).
pub(crate) fn list_providers(application: &Application) -> Vec<ProviderSummaryResource> {
    application
        .provider_catalog()
        .list_providers()
        .into_iter()
        .map(|provider| provider_summary_resource_from_catalog(provider, false))
        .collect()
}

/// List configured providers (with adapter detail).
pub(crate) fn list_configured_providers(application: &Application) -> Vec<ProviderSummaryResource> {
    application
        .provider_catalog()
        .list_providers()
        .into_iter()
        .map(|provider| provider_summary_resource_from_catalog(provider, true))
        .collect()
}

/// Set a workspace-scoped config file setting, reloading the runtime when the
/// edit requires it.
pub(crate) async fn set_workspace_config_setting(
    application: &Application,
    path: &str,
    value: JsonValue,
) -> Result<agena_runtime::ConfigSettingsEditResponse> {
    let response = application
        .runtime_config_settings()
        .set_project_file_setting(agena_runtime::ConfigSettingsSetInput {
            path: path.trim().to_owned(),
            value,
            options: agena_runtime::ConfigSettingsEditOptions {
                dry_run: false,
                validate: true,
                reload: true,
            },
        })
        .context("failed to set workspace config setting")?;

    if response.reload_required {
        application
            .runtime_control()
            .reload()
            .await
            .context("failed to reload runtime after workspace config change")?;
    }
    Ok(response)
}

/// Delete a workspace-scoped config file setting, reloading the runtime when
/// the edit requires it.
pub(crate) async fn delete_workspace_config_setting(
    application: &Application,
    path: &str,
) -> Result<agena_runtime::ConfigSettingsEditResponse> {
    let response = application
        .runtime_config_settings()
        .delete_project_file_setting(agena_runtime::ConfigSettingsDeleteInput {
            path: path.trim().to_owned(),
            options: agena_runtime::ConfigSettingsEditOptions {
                dry_run: false,
                validate: true,
                reload: true,
            },
        })
        .context("failed to delete workspace config setting")?;

    if response.reload_required {
        application
            .runtime_control()
            .reload()
            .await
            .context("failed to reload runtime after workspace config change")?;
    }
    Ok(response)
}

/// Refresh provider client versions from the remote registry.
pub(crate) async fn refresh_provider_client_versions(
    application: &Application,
) -> Result<agena_provider::ProviderClientVersions> {
    application
        .refresh_provider_client_versions()
        .await
        .context("failed to refresh provider client versions")
}

async fn current_workspace_id(application: &Application) -> Result<i64> {
    Ok(application
        .service()
        .resolve_workspace(agena_application::dto::WorkspaceResolveRequest {
            workspace: agena_application::dto::WorkspacePathRequest {
                path: application.workspace_root().to_string_lossy().to_string(),
            },
            create_if_missing: true,
        })
        .await
        .map_err(anyhow::Error::new)
        .context("failed to resolve current workspace")?
        .id)
}

fn provider_summary_resource_from_catalog(
    provider: agena_provider::ProviderCatalogEntry,
    include_adapters: bool,
) -> ProviderSummaryResource {
    ProviderSummaryResource {
        provider_id: provider.provider_id.to_string(),
        defaults: ProviderDefaultsResource {
            adapter: provider.defaults.adapter,
            model: provider.defaults.model,
        },
        adapters: if include_adapters {
            provider.adapters
        } else {
            Vec::new()
        }
        .into_iter()
        .map(|adapter| ProviderAdapterSummaryResource {
            adapter_id: adapter.adapter_id,
            enabled: adapter.enabled,
            configured_model_count: adapter.configured_model_count,
        })
        .collect(),
    }
}
