use agena_api::resource::SessionExecutionResource;
use agena_application::dto::{SessionCreateRequest, SessionHierarchyRequest, SessionUpdateRequest};

// ─── Command dispatch ───────────────────────────────────────────────────

/// Thin wire adapter: wrap the Application-provided execution projection into
/// the JSON-RPC command result.
fn execution_command_result(resource: SessionExecutionResource) -> CommandResult {
    CommandResult::Execution(resource)
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
                .await?;
            Ok(CommandResult::Workspace(workspace))
        }
        Command::UpdateWorkspace(UpdateWorkspaceParams {
            workspace_id, path, ..
        }) => {
            let workspace = state
                .service()
                .replace_workspace(workspace_id, WorkspacePathRequest { path })
                .await?;
            Ok(CommandResult::Workspace(workspace))
        }
        Command::DeleteWorkspace(DeleteWorkspaceParams { workspace_id }) => {
            state.service().delete_workspace(workspace_id).await?;
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
                .await?;
            Ok(CommandResult::Workspace(workspace))
        }
        Command::CreateSession(CreateSessionParams {
            workspace_id,
            title,
            auto_title,
            parent_id,
        }) => {
            let session = state
                .service()
                .create_session(SessionCreateRequest {
                    workspace_id,
                    session: SessionHierarchyRequest {
                        title,
                        parent_id,
                        auto_title,
                        additional_system_prompt: None,
                    },
                })
                .await?;
            Ok(CommandResult::Session(session))
        }
        Command::SubmitMessage(SubmitRunParams {
            session_id,
            options,
            document,
        }) => Ok(execution_command_result(
            state.submit_user_run(session_id, document, options).await?,
        )),
        Command::ContinueRun(ContinueRunParams {
            session_id,
            options,
        }) => Ok(execution_command_result(
            state.continue_session(session_id, options).await?,
        )),
        Command::CompactSession(CompactSessionParams {
            session_id,
            options,
        }) => Ok(execution_command_result(
            state.compact_session(session_id, options).await?,
        )),
        Command::CancelRun(CancelRunParams {
            session_id,
            execution_id,
        }) => Ok(CommandResult::Cancellation(
            state.cancel_run(session_id, execution_id).await?,
        )),
        Command::RewindSession(RewindSessionParams {
            session_id,
            at_message_id,
            expected_version,
        }) => {
            let outcome = session_services
                .commands
                .rewind_session(agena_runtime::SessionRewindRequest {
                    session_id,
                    at_message_id,
                    expected_version,
                })
                .await
                .map_err(|error| ApplicationError::from_failure(error.failure))?;
            Ok(execution_command_result(
                state.session_execution_resource(outcome.session_id).await?,
            ))
        }
        Command::ForkSession(ForkSessionParams {
            conversation_mode,
            session_id,
            at_message_id,
            title,
        }) => Ok(execution_command_result(
            state
                .fork_session(session_id, at_message_id, title, conversation_mode, None)
                .await?,
        )),
        Command::ListSessionTree(ListSessionTreeParams { root_id }) => {
            let summaries = session_services
                .queries
                .list_session_tree(root_id)
                .await
                .map_err(|error| ApplicationError::from_failure(*error.failure))?;
            let resources = state.session_resources_from_summaries(summaries).await?;
            Ok(CommandResult::SessionTree(resources))
        }
        Command::ReplyPermission(ReplyPermissionParams {
            session_id,
            options,
            reply,
        }) => Ok(execution_command_result(
            state
                .reply_permission(session_id, options, reply, Some("jsonrpc".to_string()))
                .await?,
        )),
        Command::ReplyUserInput(ReplyUserInputParams {
            session_id,
            options,
            reply,
        }) => Ok(execution_command_result(
            state.reply_user_input(session_id, options, reply).await?,
        )),
        Command::MarkInteractiveRequestPresented(MarkInteractiveRequestPresentedParams {
            session_id,
            request_id,
        }) => Ok(execution_command_result(
            state
                .mark_interactive_request_presented(session_id, request_id)
                .await?,
        )),
        Command::UpdateSession(UpdateSessionParams {
            session_id,
            title,
            expected_version,
        }) => {
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await?;
            }
            let session = state
                .service()
                .replace_session(
                    session_id,
                    SessionUpdateRequest {
                        title: Some(title),
                        favorite: None,
                        pinned: None,
                        auto_title: None,
                        additional_system_prompt: None,
                    },
                )
                .await?;
            Ok(CommandResult::Session(session))
        }
        Command::UpdateSessionSelection(UpdateSessionSelectionParams {
            session_id,
            options,
        }) => Ok(execution_command_result(
            state.update_session_selection(session_id, options).await?,
        )),
        Command::DeleteSession(DeleteSessionParams {
            session_id,
            expected_version,
        }) => {
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await?;
            }
            state.service().delete_session(session_id).await?;
            Ok(CommandResult::SessionDeleted { id: session_id })
        }
        Command::UpsertPermissionRule(request) => Ok(CommandResult::PermissionRule(
            state.service().create_permission_rule(request).await?,
        )),
        Command::ReplacePermissionRule(ReplacePermissionRuleParams { rule_id, rule }) => {
            Ok(CommandResult::PermissionRule(
                state
                    .service()
                    .replace_permission_rule(rule_id, rule)
                    .await?,
            ))
        }
        Command::RevokePermissionRule(RevokePermissionRuleParams { rule_id, reason }) => {
            let rule = state
                .service()
                .revoke_permission_rule(rule_id, reason)
                .await?;
            Ok(CommandResult::PermissionRule(rule))
        }
        Command::DeletePermissionRule(DeletePermissionRuleParams { rule_id }) => {
            state.service().delete_permission_rule(rule_id).await?;
            Ok(CommandResult::PermissionRuleDeleted { id: rule_id })
        }
        Command::StopActivity(StopActivityParams { activity_id }) => {
            let service = state.runtime_activities()?;
            let activity = service
                .stop_activity(&activity_id)
                .await
                .map_err(activity_command_error)?;
            Ok(CommandResult::Activity(
                agena_api::resource::BackgroundActivityResource::from(&activity),
            ))
        }
        Command::DismissActivity(DismissActivityParams { activity_id }) => {
            let service = state.runtime_activities()?;
            let activity = service
                .dismiss_activity(&activity_id)
                .map_err(activity_command_error)?;
            Ok(CommandResult::ActivityDeleted { id: activity.id })
        }
        Command::ClearFinishedActivities => {
            let service = state.runtime_activities()?;
            Ok(CommandResult::ActivitiesCleared {
                count: service
                    .clear_finished()
                    .await
                    .map_err(activity_command_error)?,
            })
        }
    }
}
use super::{
    Application, ApplicationError, CancelRunParams, Command, CommandResult, CompactSessionParams,
    ContinueRunParams, CreateSessionParams, CreateWorkspaceParams, DeletePermissionRuleParams,
    DeleteSessionParams, DeleteWorkspaceParams, DismissActivityParams, ForkSessionParams,
    ListSessionTreeParams, MarkInteractiveRequestPresentedParams, ReplacePermissionRuleParams,
    ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
    RevokePermissionRuleParams, RewindSessionParams, StopActivityParams, SubmitRunParams,
    UpdateSessionParams, UpdateSessionSelectionParams, UpdateWorkspaceParams, WorkspacePathRequest,
    WorkspaceResolveRequest,
};

fn activity_command_error(error: agena_runtime::ActivityControlError) -> ApplicationError {
    ApplicationError::bad_request_error(&error)
}
