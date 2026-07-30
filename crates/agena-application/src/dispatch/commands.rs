use crate::dto::{SessionCreateRequest, SessionHierarchyRequest, SessionUpdateRequest};
use crate::session::{
    resolve_session_run_options, session_execution_request, session_execution_resource,
    session_permission_reply_request, session_resource_from_summary,
    session_user_input_reply_request, session_user_message_request,
};
use crate::{
    application::ApplicationSessionServices,
    dispatch::{ApplicationResultExt, IntoWire},
};

// ─── Command dispatch ───────────────────────────────────────────────────

async fn execution_command_result(
    state: &Application,
    session_services: &ApplicationSessionServices,
    session_id: i64,
) -> Result<CommandResult, ApplicationError> {
    Ok(CommandResult::Execution(
        session_execution_resource(
            state,
            session_services.execution_control.as_ref(),
            session_services.queries.as_ref(),
            session_id,
        )
        .await?,
    ))
}

pub async fn dispatch_command(
    state: &Application,
    command: Command,
) -> Result<CommandResult, ApplicationError> {
    let session_services = state.session_execution_services()?;
    match command {
        Command::CreateWorkspace(CreateWorkspaceParams { path }) => {
            let workspace = state
                .service()
                .create_workspace(WorkspacePathRequest { path })
                .await
                .application()?;
            Ok(CommandResult::Workspace(workspace.into_wire()))
        }
        Command::UpdateWorkspace(UpdateWorkspaceParams {
            workspace_id, path, ..
        }) => {
            let workspace = state
                .service()
                .replace_workspace(workspace_id, WorkspacePathRequest { path })
                .await
                .application()?;
            Ok(CommandResult::Workspace(workspace.into_wire()))
        }
        Command::DeleteWorkspace(DeleteWorkspaceParams { workspace_id }) => {
            state
                .service()
                .delete_workspace(workspace_id)
                .await
                .application()?;
            Ok(CommandResult::WorkspaceDeleted { id: workspace_id })
        }
        Command::ResolveWorkspace(ResolveWorkspaceParams {
            path,
            create_if_missing,
        }) => {
            let workspace = state
                .service()
                .resolve_workspace(WorkspaceResolveRequest {
                    workspace: WorkspacePathRequest { path },
                    create_if_missing,
                })
                .await
                .application()?;
            Ok(CommandResult::Workspace(workspace.into_wire()))
        }
        Command::CreateSession(CreateSessionParams {
            workspace_id,
            title,
            parent_id,
        }) => {
            let session = state
                .service()
                .create_session(SessionCreateRequest {
                    workspace_id,
                    session: SessionHierarchyRequest { title, parent_id },
                })
                .await
                .application()?;
            Ok(CommandResult::Session(session))
        }
        Command::SubmitMessage(SubmitMessageParams {
            session_id,
            options,
            document,
        }) => {
            let request =
                session_user_message_request(state, session_id, options, document).await?;
            let outcome = session_services
                .commands
                .submit_user_message(request)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::ContinueRun(ContinueRunParams {
            session_id,
            options,
        }) => {
            let request = session_execution_request(state, session_id, options).await?;
            let outcome = session_services
                .commands
                .continue_session(request)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::CompactSession(CompactSessionParams {
            session_id,
            options,
        }) => {
            let request = session_execution_request(state, session_id, options).await?;
            let outcome = session_services
                .commands
                .compact_session(request)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::CancelRun(CancelRunParams {
            session_id,
            execution_id,
        }) => {
            let result = session_services
                .execution_control
                .cancel_execution(session_id, execution_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            Ok(CommandResult::Cancellation(result))
        }
        Command::RewindSession(RewindSessionParams {
            session_id,
            message_id,
            expected_version,
        }) => {
            let outcome = session_services
                .commands
                .rewind_session(agena_runtime::SessionRewindRequest {
                    session_id,
                    message_id,
                    expected_version,
                })
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::ForkSession(ForkSessionParams {
            session_id,
            at_message_id,
            title,
        }) => {
            let outcome = session_services
                .commands
                .fork_session(agena_runtime::SessionForkRequest {
                    session_id,
                    at_message_id,
                    title,
                    expected_version: None,
                })
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::ListSessionTree(ListSessionTreeParams { root_id }) => {
            let summaries = session_services
                .queries
                .list_session_tree(root_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            let resources = summaries
                .into_iter()
                .map(session_resource_from_summary)
                .collect();
            Ok(CommandResult::SessionTree(resources))
        }
        Command::ExportSession(ExportSessionParams { session_id }) => {
            let jsonl = session_services
                .queries
                .export_session_jsonl(session_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            Ok(CommandResult::SessionExport { jsonl })
        }
        Command::ImportSession(ImportSessionParams { jsonl }) => {
            let outcome = session_services
                .commands
                .import_session_jsonl(&jsonl)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::ReplyPermission(ReplyPermissionParams {
            session_id,
            options,
            reply,
        }) => {
            let request = session_permission_reply_request(
                state,
                session_id,
                options,
                reply,
                Some("jsonrpc".to_string()),
            )
            .await?;
            let outcome = session_services
                .commands
                .reply_permission(request)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::ReplyUserInput(ReplyUserInputParams {
            session_id,
            options,
            reply,
        }) => {
            let request =
                session_user_input_reply_request(state, session_id, options, reply).await?;
            let outcome = session_services
                .commands
                .reply_user_input(request)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::UpdateSession(UpdateSessionParams {
            session_id,
            title,
            expected_version,
        }) => {
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await
                    .application()?;
            }
            let session = state
                .service()
                .replace_session(session_id, SessionUpdateRequest { title })
                .await
                .application()?;
            Ok(CommandResult::Session(session))
        }
        Command::UpdateSessionSelection(UpdateSessionSelectionParams {
            session_id,
            options,
        }) => {
            let options = resolve_session_run_options(state, session_id, options).await?;
            let outcome = session_services
                .commands
                .update_session_selection(session_id, options)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
            execution_command_result(state, &session_services, outcome.session_id).await
        }
        Command::DeleteSession(DeleteSessionParams {
            session_id,
            expected_version,
        }) => {
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await
                    .application()?;
            }
            state
                .service()
                .delete_session(session_id)
                .await
                .application()?;
            Ok(CommandResult::SessionDeleted { id: session_id })
        }
        Command::UpsertPermissionRule(UpsertPermissionRuleParams {
            action_key,
            subject_kind,
            tool_name,
            qualifier,
            path_access_kind,
            workspace_root,
            target_path,
            network_target,
            network_host,
            network_port,
            scope,
            session_id,
            mode,
        }) => {
            let rule = state
                .service()
                .create_permission_rule(PermissionRuleWriteRequest {
                    action_key,
                    subject_kind,
                    tool_name,
                    qualifier,
                    path_access_kind,
                    workspace_root,
                    target_path,
                    network_target,
                    network_host,
                    network_port,
                    scope,
                    session_id,
                    mode,
                })
                .await
                .application()?;
            Ok(CommandResult::PermissionRule(rule.into_wire()))
        }
        Command::ReplacePermissionRule(ReplacePermissionRuleParams { rule_id, rule }) => {
            let rule = state
                .service()
                .replace_permission_rule(
                    rule_id,
                    PermissionRuleWriteRequest {
                        action_key: rule.action_key,
                        subject_kind: rule.subject_kind,
                        tool_name: rule.tool_name,
                        qualifier: rule.qualifier,
                        path_access_kind: rule.path_access_kind,
                        workspace_root: rule.workspace_root,
                        target_path: rule.target_path,
                        network_target: rule.network_target,
                        network_host: rule.network_host,
                        network_port: rule.network_port,
                        scope: rule.scope,
                        session_id: rule.session_id,
                        mode: rule.mode,
                    },
                )
                .await
                .application()?;
            Ok(CommandResult::PermissionRule(rule.into_wire()))
        }
        Command::RevokePermissionRule(RevokePermissionRuleParams { rule_id, reason }) => {
            let rule = state
                .service()
                .revoke_permission_rule(rule_id, reason)
                .await
                .application()?;
            Ok(CommandResult::PermissionRule(rule.into_wire()))
        }
        Command::DeletePermissionRule(DeletePermissionRuleParams { rule_id }) => {
            state
                .service()
                .delete_permission_rule(rule_id)
                .await
                .application()?;
            Ok(CommandResult::PermissionRuleDeleted { id: rule_id })
        }
    }
}
use super::{
    Application, ApplicationError, CancelRunParams, Command, CommandResult, CompactSessionParams,
    ContinueRunParams, CreateSessionParams, CreateWorkspaceParams, DeletePermissionRuleParams,
    DeleteSessionParams, DeleteWorkspaceParams, ExportSessionParams, ForkSessionParams,
    ImportSessionParams, ListSessionTreeParams, PermissionRuleWriteRequest,
    ReplacePermissionRuleParams, ReplyPermissionParams, ReplyUserInputParams,
    ResolveWorkspaceParams, RevokePermissionRuleParams, RewindSessionParams, SubmitMessageParams,
    UpdateSessionParams, UpdateSessionSelectionParams, UpdateWorkspaceParams,
    UpsertPermissionRuleParams, WorkspacePathRequest, WorkspaceResolveRequest,
};
