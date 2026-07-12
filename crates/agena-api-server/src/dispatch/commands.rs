use crate::dispatch::HttpApiResultExt;
use crate::session_support::{
    session_execution_reply_request, session_execution_request, session_execution_resource,
    session_permission_reply_request, session_user_message_request,
};
use agena_api::resource::SessionResource;

// ─── Command dispatch ───────────────────────────────────────────────────

async fn execution_command_result(
    state: &AppState,
    manager: &agena::session::SessionManager,
    session: &agena::session::Session,
) -> Result<CommandResult, ServerError> {
    Ok(CommandResult::Execution(
        session_execution_resource(state, manager, session).await?,
    ))
}

pub async fn dispatch_command(
    state: &AppState,
    command: Command,
) -> Result<CommandResult, ServerError> {
    let manager = state.session_manager()?;
    match command {
        Command::CreateWorkspace(CreateWorkspaceParams { path }) => {
            let workspace = state
                .service()
                .create_workspace(WorkspacePathRequest { path })
                .await
                .server()?;
            Ok(CommandResult::Workspace(workspace.into()))
        }
        Command::UpdateWorkspace(UpdateWorkspaceParams {
            workspace_id, path, ..
        }) => {
            let workspace = state
                .service()
                .replace_workspace(workspace_id, WorkspacePathRequest { path })
                .await
                .server()?;
            Ok(CommandResult::Workspace(workspace.into()))
        }
        Command::DeleteWorkspace(DeleteWorkspaceParams { workspace_id }) => {
            state
                .service()
                .delete_workspace(workspace_id)
                .await
                .server()?;
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
                .server()?;
            Ok(CommandResult::Workspace(workspace.into()))
        }
        Command::CreateSession(CreateSessionParams {
            workspace_id,
            title,
            parent_id,
        }) => {
            let session = state
                .service()
                .create_session(HttpSessionCreateRequest {
                    workspace_id,
                    session: crate::local_api::SessionHierarchyRequest { title, parent_id },
                })
                .await
                .server()?;
            Ok(CommandResult::Session(session))
        }
        Command::SubmitMessage(SubmitMessageParams {
            session_id,
            options,
            parts,
        }) => {
            let request = session_user_message_request(state, session_id, options, parts).await?;
            let session = manager.submit_user_message(request).await?;
            execution_command_result(state, manager.as_ref(), &session).await
        }
        Command::ContinueRun(ContinueRunParams {
            session_id,
            options,
        }) => {
            let request = session_execution_request(state, session_id, options).await?;
            let session = manager.continue_session(request).await?;
            execution_command_result(state, manager.as_ref(), &session).await
        }
        Command::CompactSession(CompactSessionParams {
            session_id,
            options,
        }) => {
            let request = session_execution_request(state, session_id, options).await?;
            let session = manager.compact_session(request).await?;
            execution_command_result(state, manager.as_ref(), &session).await
        }
        Command::CancelRun(CancelRunParams { session_id }) => {
            // Best-effort: if the run just finished moments before the
            // cancel arrived, no active execution is normal — surface as Ack so
            // the client doesn't spin on it.
            match manager.cancel_active_execution(session_id).await {
                Ok(()) => Ok(CommandResult::Ack),
                Err(_) => Ok(CommandResult::Ack),
            }
        }
        Command::RewindSession(RewindSessionParams {
            session_id,
            message_id,
            expected_version,
        }) => {
            let session = manager
                .rewind_session(agena::session::SessionRewindRequest {
                    session_id,
                    message_id,
                    expected_version,
                })
                .await?;
            execution_command_result(state, manager.as_ref(), &session).await
        }
        Command::ForkSession(ForkSessionParams {
            session_id,
            at_message_id,
            title,
        }) => {
            let session = manager
                .fork_session(agena::session::SessionForkRequest {
                    session_id,
                    at_message_id,
                    title,
                    expected_version: None,
                })
                .await?;
            execution_command_result(state, manager.as_ref(), &session).await
        }
        Command::ListSessionTree(ListSessionTreeParams { root_id }) => {
            let summaries = manager.list_session_tree(root_id).await?;
            let resources: Vec<SessionResource> =
                summaries.into_iter().map(SessionResource::from).collect();
            Ok(CommandResult::SessionTree(resources))
        }
        Command::ExportSession(ExportSessionParams { session_id }) => {
            let jsonl = manager.export_session_jsonl(session_id).await?;
            Ok(CommandResult::SessionExport { jsonl })
        }
        Command::ImportSession(ImportSessionParams { jsonl }) => {
            let session = manager.import_session_jsonl(&jsonl).await?;
            execution_command_result(state, manager.as_ref(), &session).await
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
            let session = manager.reply_permission(request).await?;
            execution_command_result(state, manager.as_ref(), &session).await
        }
        Command::ReplyUserInput(ReplyUserInputParams {
            session_id,
            options,
            reply,
        }) => {
            let request =
                session_execution_reply_request(state, session_id, options, reply).await?;
            let session = manager.reply_user_input(request).await?;
            execution_command_result(state, manager.as_ref(), &session).await
        }
        Command::UpdateSession(UpdateSessionParams {
            session_id,
            title,
            parent_id,
            expected_version,
        }) => {
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await
                    .server()?;
            }
            let session = state
                .service()
                .replace_session(session_id, SessionHierarchyRequest { title, parent_id })
                .await
                .server()?;
            Ok(CommandResult::Session(session))
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
                    .server()?;
            }
            state.service().delete_session(session_id).await.server()?;
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
                .server()?;
            Ok(CommandResult::PermissionRule(rule.into()))
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
                .server()?;
            Ok(CommandResult::PermissionRule(rule.into()))
        }
        Command::RevokePermissionRule(RevokePermissionRuleParams { rule_id, reason }) => {
            let rule = state
                .service()
                .revoke_permission_rule(rule_id, reason)
                .await
                .server()?;
            Ok(CommandResult::PermissionRule(rule.into()))
        }
        Command::DeletePermissionRule(DeletePermissionRuleParams { rule_id }) => {
            state
                .service()
                .delete_permission_rule(rule_id)
                .await
                .server()?;
            Ok(CommandResult::PermissionRuleDeleted { id: rule_id })
        }
    }
}
use super::{
    AppState, CancelRunParams, Command, CommandResult, CompactSessionParams, ContinueRunParams,
    CreateSessionParams, CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
    DeleteWorkspaceParams, ExportSessionParams, ForkSessionParams, HttpSessionCreateRequest,
    ImportSessionParams, ListSessionTreeParams, PermissionRuleWriteRequest,
    ReplacePermissionRuleParams, ReplyPermissionParams, ReplyUserInputParams,
    ResolveWorkspaceParams, RevokePermissionRuleParams, RewindSessionParams, ServerError,
    SessionHierarchyRequest, SubmitMessageParams, UpdateSessionParams, UpdateWorkspaceParams,
    UpsertPermissionRuleParams, WorkspacePathRequest, WorkspaceResolveRequest,
};
