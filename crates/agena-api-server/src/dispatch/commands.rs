use super::*;

// ─── Command dispatch ───────────────────────────────────────────────────

pub async fn dispatch_command(
    state: &AppState,
    command: Command,
) -> Result<CommandResult, ServerError> {
    let manager = state.session_manager()?;
    match command {
        Command::CreateWorkspace(CreateWorkspaceParams { path }) => {
            let workspace = state
                .service()
                .create_workspace(WorkspaceWriteRequest { path })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Workspace(workspace_from_http(workspace)))
        }
        Command::UpdateWorkspace(UpdateWorkspaceParams {
            workspace_id, path, ..
        }) => {
            let workspace = state
                .service()
                .replace_workspace(workspace_id, WorkspaceWriteRequest { path })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Workspace(workspace_from_http(workspace)))
        }
        Command::DeleteWorkspace(DeleteWorkspaceParams { workspace_id }) => {
            state
                .service()
                .delete_workspace(workspace_id)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::WorkspaceDeleted { id: workspace_id })
        }
        Command::ResolveWorkspace(ResolveWorkspaceParams {
            path,
            create_if_missing,
        }) => {
            let workspace = state
                .service()
                .resolve_workspace(WorkspaceResolveRequest {
                    path,
                    create_if_missing,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Workspace(workspace_from_http(workspace)))
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
                    title,
                    parent_id,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Session(session_from_http(session)))
        }
        Command::CreateSessionGoal(CreateSessionGoalParams {
            session_id,
            objective,
            token_budget,
        }) => {
            let goal = manager
                .create_goal(agena::session::SessionGoalCreateRequest {
                    session_id,
                    objective,
                    token_budget,
                })
                .await?;
            let session = manager.get_session(session_id).await?;
            let resource = state
                .service()
                .session_goal_resource(manager.as_ref(), &session, &goal)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::SessionGoal(session_goal_from_http(resource)))
        }
        Command::SetSessionGoal(SetSessionGoalParams {
            session_id,
            objective,
            status,
            token_budget,
            clear,
        }) => {
            if clear {
                let cleared = manager.clear_goal(session_id).await?;
                if !cleared {
                    return Err(ServerError::NotFound(format!(
                        "session {session_id} goal not found"
                    )));
                }
                return Ok(CommandResult::SessionGoalCleared { session_id });
            }

            let goal = if manager.get_goal(session_id).await?.is_some() {
                manager
                    .update_goal(agena::session::SessionGoalUpdateRequest {
                        session_id,
                        objective,
                        status,
                        token_budget,
                        expected_goal_id: None,
                    })
                    .await?
            } else {
                if !matches!(status, None | Some(agena::session::GoalStatus::Active)) {
                    return Err(ServerError::BadRequest(format!(
                        "session {session_id} goal must be created with status active"
                    )));
                }
                let objective = objective.ok_or_else(|| {
                    ServerError::BadRequest(format!(
                        "session {session_id} goal objective is required when creating a goal"
                    ))
                })?;
                manager
                    .create_goal(agena::session::SessionGoalCreateRequest {
                        session_id,
                        objective,
                        token_budget: token_budget.flatten(),
                    })
                    .await?
            };
            let session = manager.get_session(session_id).await?;
            let resource = state
                .service()
                .session_goal_resource(manager.as_ref(), &session, &goal)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::SessionGoal(session_goal_from_http(resource)))
        }
        Command::CompleteSessionGoal(CompleteSessionGoalParams { session_id }) => {
            let goal = manager.complete_goal(session_id).await?;
            let session = manager.get_session(session_id).await?;
            let resource = state
                .service()
                .session_goal_resource(manager.as_ref(), &session, &goal)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::SessionGoal(session_goal_from_http(resource)))
        }
        Command::ClearSessionGoal(ClearSessionGoalParams { session_id }) => {
            let cleared = manager.clear_goal(session_id).await?;
            if !cleared {
                return Err(ServerError::NotFound(format!(
                    "session {session_id} goal not found"
                )));
            }
            Ok(CommandResult::SessionGoalCleared { session_id })
        }
        Command::SubmitTurn(SubmitTurnParams {
            session_id,
            options,
            parts,
        }) => {
            let request = SessionUserTurnRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
                parts,
            };
            let session = manager.submit_user_turn(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ContinueRun(ContinueRunParams {
            session_id,
            options,
        }) => {
            let request = SessionContinueRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
            };
            let session = manager.continue_session(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::CompactSession(CompactSessionParams {
            session_id,
            options,
        }) => {
            let request = SessionCompactRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
            };
            let session = manager.compact_session(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::CancelTurn(CancelTurnParams { session_id }) => {
            // Best-effort: if the turn just finished moments before the
            // cancel arrived, NoActiveTurn is normal — surface as Ack so
            // the client doesn't spin on it.
            match manager.cancel_active_turn(session_id).await {
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
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
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
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ListSessionTree(ListSessionTreeParams { root_id }) => {
            let summaries = manager.list_session_tree(root_id).await?;
            let resources: Vec<SessionResource> =
                summaries.into_iter().map(SessionResource::from).collect();
            Ok(CommandResult::SessionTree(resources))
        }
        Command::ListRewindCheckpoints(ListRewindCheckpointsParams { session_id }) => {
            let checkpoints = manager.list_rewind_checkpoints(session_id).await?;
            Ok(CommandResult::RewindCheckpoints(
                checkpoints.into_iter().map(Into::into).collect(),
            ))
        }
        Command::ExportSession(ExportSessionParams { session_id }) => {
            let jsonl = manager.export_session_jsonl(session_id).await?;
            Ok(CommandResult::SessionExport { jsonl })
        }
        Command::ImportSession(ImportSessionParams { jsonl }) => {
            let session = manager.import_session_jsonl(&jsonl).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ReplyPermission(ReplyPermissionParams {
            session_id,
            options,
            reply,
        }) => {
            let request = SessionPermissionReplyRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
                reply,
                operator: Some("jsonrpc".to_string()),
            };
            let session = manager.reply_permission(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ReplyUserInput(ReplyUserInputParams {
            session_id,
            options,
            reply,
        }) => {
            let request = SessionUserInputReplyRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
                reply,
            };
            let session = manager.reply_user_input(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
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
                    .map_err(server_error_from_http)?;
            }
            let session = state
                .service()
                .replace_session(session_id, SessionReplaceRequest { title, parent_id })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Session(session_from_http(session)))
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
                    .map_err(server_error_from_http)?;
            }
            state
                .service()
                .delete_session(session_id)
                .await
                .map_err(server_error_from_http)?;
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
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRule(permission_rule_from_http(
                rule,
            )))
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
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRule(permission_rule_from_http(
                rule,
            )))
        }
        Command::RevokePermissionRule(RevokePermissionRuleParams { rule_id, reason }) => {
            let rule = state
                .service()
                .revoke_permission_rule(rule_id, reason)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRule(permission_rule_from_http(
                rule,
            )))
        }
        Command::DeletePermissionRule(DeletePermissionRuleParams { rule_id }) => {
            state
                .service()
                .delete_permission_rule(rule_id)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRuleDeleted { id: rule_id })
        }
    }
}
