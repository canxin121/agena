use std::path::{Path, PathBuf};

use agena_domain::{
    AccessKind, ExternalAgentArchiveFilter, ExternalAgentForkPoint, ExternalAgentSessionAction,
    ExternalAgentSessionInfo, ExternalAgentSessionResult, ExternalAgentWorkspaceAction,
    ExternalAgentWorkspaceResult, PermissionDecision,
};
use agena_storage::kernel::{ExternalAgentFork, ExternalAgentImport, ExternalAgentLink};

use super::*;
use crate::session::manager::session_prompt::{PromptInputs, resolve_configured_prompts};

mod index;

pub(super) fn apply_info_update(state: &mut ExternalAgentState, update: &Value) -> bool {
    let Some(next) = update.as_object() else {
        return false;
    };
    let mut metadata = state.metadata.clone();
    if let Some(title) = next.get("title") {
        metadata.title = acp::bounded_title(title);
    }
    if let Some(updated) = next.get("updatedAt") {
        metadata.updated_at = acp::bounded_string(updated, 128);
    }
    if state.metadata == metadata {
        return false;
    }
    state.metadata = metadata;
    true
}

pub(super) async fn directories_for_agent(
    executor: &ToolExecutor,
    cwd: &Path,
    directories: &[PathBuf],
) -> Result<Vec<PathBuf>, AppError> {
    if directories.len() > 16 {
        return Err(AppError::Config(
            "Specify at most 16 additional workspace directories.".into(),
        ));
    }
    let mut resolved = Vec::with_capacity(directories.len());
    for directory in directories {
        let path = if directory.is_absolute() {
            directory.clone()
        } else {
            cwd.join(directory)
        };
        let directory = canonical_directory(&path).await?;
        if directory == cwd || resolved.contains(&directory) {
            continue;
        }
        if !matches!(
            executor
                .principal()
                .authorize_path_access(AccessKind::Read, cwd, &directory),
            PermissionDecision::Allow
        ) {
            return Err(AppError::Config(format!(
                "Allow read access to {} before adding it to this agent's workspace.",
                directory.display(),
            )));
        }
        resolved.push(directory);
    }
    Ok(resolved)
}

async fn canonical_directory(path: &Path) -> Result<PathBuf, AppError> {
    let canonical = tokio::fs::canonicalize(path).await.map_err(|error| {
        AppError::Config(format!(
            "Could not open workspace directory {}: {error}",
            path.display()
        ))
    })?;
    if !tokio::fs::metadata(&canonical)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(AppError::Config(format!(
            "{} is not a directory.",
            canonical.display()
        )));
    }
    Ok(canonical)
}

impl SessionManager {
    pub(in crate::session::manager) async fn manage_external_agent_workspace(
        &self,
        workspace_id: i64,
        action: ExternalAgentWorkspaceAction,
    ) -> Result<ExternalAgentWorkspaceResult, AppError> {
        if !self.is_acp_scope() {
            return Err(AppError::Config("Open the project in ACP sessions.".into()));
        }
        let (agent, cursor, import, refresh) = match action {
            ExternalAgentWorkspaceAction::List { agent, cursor } => (agent, cursor, None, false),
            ExternalAgentWorkspaceAction::Import { agent, native_id } => {
                (agent, None, Some(native_id), false)
            }
            ExternalAgentWorkspaceAction::Open { agent, native_id } => {
                (agent, None, Some(native_id), true)
            }
            action => return self.manage_native_index(workspace_id, action).await,
        };
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > acp::MAX_CURSOR_BYTES)
            || import.as_deref().is_some_and(invalid_native_string)
        {
            return Err(AppError::Config(
                "Invalid external agent conversation ID or cursor.".into(),
            ));
        }
        let cwd = canonical_directory(&self.workspace_root_by_id(workspace_id).await?).await?;
        let _permit = self.reserve_execution()?;
        let current_view = if let Some(native_id) = import.as_ref().filter(|_| refresh) {
            let link = ExternalAgentLink {
                native_id: native_id.clone(),
                cwd: cwd.clone(),
                incarnation: None,
                resume_at: None,
            };
            self.kernel_runtime
                .acp_view(workspace_id, agent, &link)
                .await
                .map_err(kernel_error)?
                .map(|id| (id, self.session_dispatch(id)))
        } else {
            None
        };
        // Attach to live work without opening a second peer on its native
        // history. Recheck idleness under the same locks as submit and run.
        let held = if let Some((id, dispatch)) = current_view.as_ref() {
            let attached = ExternalAgentWorkspaceResult::Opened {
                session_id: id.get(),
            };
            let Ok(submission) = dispatch.submission.try_lock() else {
                return Ok(attached);
            };
            let Ok(execution) = dispatch.execution.try_lock() else {
                return Ok(attached);
            };
            match self.kernel().reader.control_snapshot(*id, 0).await {
                Ok(snapshot)
                    if snapshot.summary.running_executions != 0
                        || snapshot.summary.queued_executions != 0
                        || snapshot.configuration.external_agent_inputs.is_some() =>
                {
                    return Ok(attached);
                }
                Ok(_) => {
                    self.close_idle_external_agent_for_management(*id).await;
                    Some((*id, submission, execution))
                }
                Err(KernelError::NotFound(_)) => None,
                Err(error) => return Err(kernel_error(error)),
            }
        } else {
            None
        };
        let cancel = self.shutdown.child_token();
        let mut client = acp::AcpClient::spawn(agent, &cwd)?;
        let outcome = async {
            let Some(native_id) = import else {
                let page = client
                    .list_native(
                        &cwd,
                        cursor.as_deref(),
                        ExternalAgentArchiveFilter::Unarchived,
                        &cancel,
                    )
                    .await?;
                return Ok(ExternalAgentWorkspaceResult::Page {
                    sessions: page.sessions.into_iter().map(|row| row.session).collect(),
                    next_cursor: page.next_cursor,
                });
            };
            let filter = if client.supports_native_archive(&cancel).await? {
                ExternalAgentArchiveFilter::All
            } else {
                ExternalAgentArchiveFilter::Unarchived
            };
            let record = match client
                .find_native(&cwd, &native_id, filter, &cancel)
                .await?
            {
                Some(row) => row.session,
                None if filter == ExternalAgentArchiveFilter::Unarchived => {
                    self.index_import_record(agent, &cwd, &native_id, &cancel)
                        .await?
                }
                None => {
                    return Err(AppError::Config(
                        "That native conversation is not listed in this workspace.".into(),
                    ));
                }
            };
            let (turns, mut state) = client
                .replay_native(&cwd, &native_id, &acp::SessionContext::default(), &cancel)
                .await?;
            if client.lifecycle_capabilities(&cancel).await?.close {
                let _ = client.release_native(&native_id, false, &cancel).await;
            }
            let title = state
                .metadata
                .title
                .clone()
                .or_else(|| record.title.clone())
                .unwrap_or_else(|| format!("{} conversation", agent.label()));
            state.metadata.title = state.metadata.title.or(record.title);
            state.metadata.updated_at = state.metadata.updated_at.or(record.updated_at);
            let link = ExternalAgentLink {
                native_id,
                cwd: cwd.clone(),
                incarnation: None,
                resume_at: None,
            };
            let request = RequestId::new();
            let import = ExternalAgentImport {
                workspace_id,
                agent,
                title,
                link,
                turns,
                state,
                replace_view: held.as_ref().map(|(id, _, _)| *id),
            };
            let created = persist(|| {
                self.kernel()
                    .sessions
                    .import_external_agent(request, import.clone())
            })
            .await
            .map_err(kernel_error)?;
            let session_id = created.value.id.get();
            Ok(if refresh {
                ExternalAgentWorkspaceResult::Opened { session_id }
            } else {
                ExternalAgentWorkspaceResult::Imported { session_id }
            })
        }
        .await;
        client.close().await;
        outcome
    }

    pub(in crate::session::manager) async fn manage_external_agent_session(
        &self,
        id: i64,
        action: ExternalAgentSessionAction,
    ) -> Result<ExternalAgentSessionResult, AppError> {
        if !self.is_acp_scope() {
            return Err(AppError::Config(
                "Open the native conversation in ACP sessions.".into(),
            ));
        }
        let session = session_id(id)?;
        self.require_writable_bot_history(id).await?;
        if matches!(action, ExternalAgentSessionAction::Fork { .. }) {
            self.require_bot_branch_allowed(id).await?;
        }
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 0)
            .await
            .map_err(kernel_error)?;
        let configuration = &snapshot.configuration;
        let agent = configuration
            .external_agent
            .ok_or_else(|| AppError::Config("This session has no external agent.".into()))?;
        let running = snapshot.summary.running_executions != 0;
        let mutation = !matches!(action, ExternalAgentSessionAction::Status);
        if mutation && (running || snapshot.summary.queued_executions != 0) {
            return Err(AppError::Config("Finish or stop the active turn and clear its queue before changing the native session.".into()));
        }
        // The execution row can become terminal before its driver has parked
        // the peer. Wait for that handoff before taking or closing the peer.
        let _execution = if mutation {
            Some(dispatch.execution.lock().await)
        } else {
            None
        };
        let state = self.state_for_session(session).await?;
        let context = self
            .execution_context_for_execution(session, &state, None)
            .await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let cwd = canonical_directory(executor.workspace_root()).await?;
        let link = configuration.native_session_link();
        if matches!(action, ExternalAgentSessionAction::Delete) && link.is_none() {
            return Ok(ExternalAgentSessionResult::Deleted);
        }
        if matches!(action, ExternalAgentSessionAction::Close) && link.is_none() {
            return Ok(ExternalAgentSessionResult::Closed);
        }
        let _permit = self.reserve_execution()?;
        let cancel = self.shutdown.child_token();
        let mut client = acp::AcpClient::spawn(agent, &cwd)?;
        let outcome = async {
            let capabilities = client.lifecycle_capabilities(&cancel).await?;
            if matches!(action, ExternalAgentSessionAction::Status) {
                let native_status = match &link {
                    Some(_) if running => Some("open".to_owned()),
                    Some(link) if link.cwd == cwd => client.native_status(&link.native_id, &cancel).await?,
                    _ => None,
                };
                let mut fork_points = Vec::new();
                if let Some(link) = link.as_ref().filter(|link| link.cwd == cwd)
                    && capabilities.fork_before_message
                {
                    fork_points = self.kernel().reader.external_agent_fork_points(session, link, None)
                        .await.map_err(kernel_error)?.into_iter()
                        .map(|(cursor, label)| ExternalAgentForkPoint { message_id: cursor.message_id.get(), label })
                        .collect();
                }
                return Ok(ExternalAgentSessionResult::Status {
                    session: link.map(|link| ExternalAgentSessionInfo {
                        native_id: link.native_id,
                        cwd: link.cwd,
                        title: configuration.external_agent_state.metadata.title.clone(),
                        updated_at: configuration.external_agent_state.metadata.updated_at.clone(),
                    }),
                    native_status,
                    additional_directories: configuration.external_agent_directories.clone(),
                    capabilities,
                    fork_points,
                });
            }
            if let ExternalAgentSessionAction::SetDirectories { directories } = action {
                let directories = directories_for_agent(&executor, &cwd, &directories).await?;
                if !directories.is_empty() && !capabilities.additional_directories {
                    return Err(AppError::Config("The installed agent does not support additional workspace directories over ACP.".into()));
                }
                self.close_idle_external_agent(session).await;
                let request = RequestId::new();
                persist(|| {
                    self.kernel().sessions.configure(request, session, SessionConfigurationChange {
                        external_agent_directories: Some(directories.clone()),
                        ..Default::default()
                    })
                }).await.map_err(kernel_error)?;
                return Ok(ExternalAgentSessionResult::Directories { directories });
            }
            let link = link.filter(|link| link.cwd == cwd).ok_or_else(|| {
                AppError::Config("This session has no native conversation in its current workspace.".into())
            })?;
            match action {
                ExternalAgentSessionAction::Close => {
                    if capabilities.close {
                        self.close_idle_external_agent_for_management(session).await;
                    }
                    client.release_native(&link.native_id, false, &cancel).await?;
                    Ok(ExternalAgentSessionResult::Closed)
                }
                ExternalAgentSessionAction::Delete => {
                    if capabilities.delete {
                        self.close_idle_external_agent(session).await;
                    }
                    client.release_native(&link.native_id, true, &cancel).await?;
                    let request = RequestId::new();
                    persist(|| self.kernel().sessions.detach_external_agent(request, session, link.clone()))
                        .await.map_err(kernel_error)?;
                    Ok(ExternalAgentSessionResult::Deleted)
                }
                ExternalAgentSessionAction::SetTitle { title } => {
                    let title = validated_title(title)?;
                    if capabilities.title {
                        self.close_idle_external_agent(session).await;
                    }
                    let title = client.set_native_title(&link.native_id, &title, &cancel).await?;
                    let request = RequestId::new();
                    persist(|| self.kernel().sessions.update_metadata(request, session, MetadataChange {
                        title: Some(title.clone()),
                        sync_external_agent_title: true,
                        ..Default::default()
                    })).await.map_err(kernel_error)?;
                    Ok(ExternalAgentSessionResult::Title { title })
                }
                ExternalAgentSessionAction::Fork { title, before_message_id } => {
                    let title = title.map(validated_title).transpose()?;
                    let history = self.kernel().reader.history(session, None, 1).await.map_err(kernel_error)?;
                    let head = history.entries.first().map(|entry| entry.id).ok_or_else(|| {
                        AppError::Config("Cannot fork a session without history.".into())
                    })?;
                    let before = if let Some(message_id) = before_message_id {
                        if !capabilities.fork_before_message {
                            return Err(AppError::Config("The installed agent does not support forking before a message over ACP.".into()));
                        }
                        let message = entry_id(message_id)?;
                        let (cursor, _) = self.kernel().reader.external_agent_fork_points(session, &link, Some(message))
                            .await.map_err(kernel_error)?.pop().ok_or_else(|| {
                                AppError::Config("That message has no native fork point in this conversation. See native session status for available messages.".into())
                            })?;
                        Some(cursor)
                    } else {
                        if configuration.external_agent_resume.is_none()
                            && link.resume_at.as_ref().is_none_or(|anchor| anchor.tail != Some(head))
                        {
                            return Err(AppError::Config("The current transcript is not aligned with a retained native continuation.".into()));
                        }
                        None
                    };
                    let directories = directories_for_agent(&executor, &cwd, &configuration.external_agent_directories).await?;
                    let context = session_context(agent, session, &cwd, &state, &context, directories);
                    if capabilities.fork {
                        self.close_idle_external_agent(session).await;
                    }
                    let new_id = client.fork_native(
                        &cwd, &link.native_id, before.as_ref().map(|cursor| cursor.cursor_id.as_str()), &context, &cancel,
                    ).await?;
                    let request = RequestId::new();
                    let through = before.as_ref().map_or(head, |cursor| cursor.message_id);
                    let fork = ExternalAgentFork {
                        expected_membership: snapshot.metadata.membership_generation,
                        expected_head: head,
                        expected_agent: agent,
                        expected_native: link,
                        before,
                        new_native_id: new_id.clone(),
                        title,
                    };
                    let result = persist(|| self.kernel().sessions.fork_external_agent(request, session, through, fork.clone()))
                        .await.map_err(kernel_error);
                    if capabilities.close {
                        let _ = client.release_native(&new_id, false, &cancel).await;
                    }
                    Ok(ExternalAgentSessionResult::Forked { session_id: result?.value.id.get() })
                }
                ExternalAgentSessionAction::Status | ExternalAgentSessionAction::SetDirectories { .. } => unreachable!(),
            }
        }.await;
        client.close().await;
        outcome
    }
}

pub(super) fn session_context(
    agent: ExternalAgent,
    session: SessionId,
    cwd: &Path,
    state: &ExecutionState,
    context: &crate::session::model::SessionExecutionContext,
    additional_directories: Vec<PathBuf>,
) -> acp::SessionContext {
    let model = agent.run_identity();
    let system_prompt = resolve_configured_prompts(
        state,
        &PromptInputs {
            profile: &context.profile_prompts,
            project_instructions_on_demand: context.project_instructions_on_demand,
            preload_skills: &context.preload_skills,
            assignment: &context.agent_assignment,
            profile_from_bot: context.profile_from_bot,
            session: &context.prompts,
            bot: None,
            request_layer: None,
            model: &model,
            session_id: session.get(),
            workspace: cwd.to_string_lossy().into_owned(),
            global: false,
            tools_enabled: true,
        },
    );
    acp::SessionContext {
        system_prompt,
        additional_directories,
    }
}

fn invalid_native_string(value: &str) -> bool {
    value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
}

fn validated_title(title: String) -> Result<String, AppError> {
    let title = title.trim();
    if invalid_native_string(title) {
        return Err(AppError::Config(
            "Enter a non-empty conversation title.".into(),
        ));
    }
    Ok(title.to_owned())
}
