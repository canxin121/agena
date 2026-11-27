use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, Weak},
};

use agena_domain::{
    AccessKind, ExternalAgentArchiveFilter, ExternalAgentForkPoint, ExternalAgentSessionAction,
    ExternalAgentSessionInfo, ExternalAgentSessionResult, ExternalAgentWorkspaceAction,
    ExternalAgentWorkspaceResult, PermissionDecision,
};
use agena_storage::kernel::{ExternalAgentFork, ExternalAgentImport, ExternalAgentLink};

use super::*;
use crate::session::manager::session_prompt::external_agent_system_prompt;

mod index;

static NATIVE_REPLAYS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
type NativeReplayKey = (PathBuf, String, String);
type NativeReplayLocks = StdMutex<HashMap<NativeReplayKey, Weak<tokio::sync::Mutex<()>>>>;
static NATIVE_REPLAY_LOCKS: LazyLock<NativeReplayLocks> = LazyLock::new(Default::default);

fn native_replay_lock(
    cwd: &Path,
    agent: &ExternalAgent,
    native_id: &str,
) -> Arc<tokio::sync::Mutex<()>> {
    let key = (cwd.to_owned(), agent.id().to_owned(), native_id.to_owned());
    let mut locks = NATIVE_REPLAY_LOCKS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    locks.retain(|_, lock| lock.strong_count() != 0);
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryAccess {
    Import,
    Open,
    Preview(Option<u64>),
}

fn active_history_view(
    access: HistoryAccess,
    session: SessionId,
) -> Result<ExternalAgentWorkspaceResult, AppError> {
    match access {
        HistoryAccess::Open => Ok(ExternalAgentWorkspaceResult::Opened {
            session_id: session.get(),
        }),
        _ => Err(AppError::Config(
            "Finish or stop the active turn before previewing native history.".into(),
        )),
    }
}

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

pub(super) async fn canonical_directory(path: &Path) -> Result<PathBuf, AppError> {
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
        let (agent, cursor, native) = match action {
            ExternalAgentWorkspaceAction::List { agent, cursor } => (agent, cursor, None),
            ExternalAgentWorkspaceAction::Import { agent, native_id } => {
                (agent, None, Some((native_id, HistoryAccess::Import)))
            }
            ExternalAgentWorkspaceAction::Open { agent, native_id } => {
                (agent, None, Some((native_id, HistoryAccess::Open)))
            }
            ExternalAgentWorkspaceAction::PreviewHistory {
                agent,
                native_id,
                before_turn,
            } => {
                if before_turn.is_some_and(|turn| turn <= 1) {
                    return Err(AppError::Config(
                        "Invalid native history turn boundary.".into(),
                    ));
                }
                (
                    agent,
                    None,
                    Some((native_id, HistoryAccess::Preview(before_turn))),
                )
            }
            action => return self.manage_native_index(workspace_id, action).await,
        };
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > acp::MAX_CURSOR_BYTES + 64)
            || native
                .as_ref()
                .is_some_and(|(id, _)| invalid_native_string(id))
        {
            return Err(AppError::Config(
                "Invalid external agent conversation ID or cursor.".into(),
            ));
        }
        let spec = self.resolve_external_agent(&agent)?;
        let cwd = canonical_directory(&self.workspace_root_by_id(workspace_id).await?).await?;
        let _permit = self.reserve_execution()?;
        let cancel = self.shutdown.child_token();
        let _native_replay = if let Some((native_id, _)) = native.as_ref() {
            // Serialize even the first replay, before a local session ID
            // exists to coordinate clicks from different clients.
            let lock = native_replay_lock(&cwd, &agent, native_id);
            Some(tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                held = lock.lock_owned() => held,
            })
        } else {
            None
        };
        let current_view = if let Some((native_id, access)) = native.as_ref() {
            let link = ExternalAgentLink {
                native_id: native_id.clone(),
                cwd: cwd.clone(),
                incarnation: None,
                resume_at: None,
            };
            let existing = self
                .kernel_runtime
                .acp_view(workspace_id, agent.clone(), &link)
                .await
                .map_err(kernel_error)?;
            if let Some(id) = existing
                && *access == HistoryAccess::Import
            {
                return Ok(ExternalAgentWorkspaceResult::Imported {
                    session_id: id.get(),
                });
            }
            existing.map(|id| (id, self.session_dispatch(id), *access))
        } else {
            None
        };
        // Attach to live work without opening a second peer on its native
        // history. Recheck idleness under the same locks as submit and run.
        let held = if let Some((id, dispatch, access)) = current_view.as_ref() {
            let Ok(submission) = dispatch.submission.try_lock() else {
                return active_history_view(*access, *id);
            };
            let Ok(execution) = dispatch.execution.try_lock() else {
                return active_history_view(*access, *id);
            };
            match self.kernel().reader.control_snapshot(*id, 0).await {
                Ok(snapshot)
                    if snapshot.summary.running_executions != 0
                        || snapshot.summary.queued_executions != 0
                        || snapshot.configuration.external_agent_inputs.is_some() =>
                {
                    return active_history_view(*access, *id);
                }
                Ok(snapshot) => Some((
                    *id,
                    submission,
                    execution,
                    snapshot.configuration.external_agent_directories.clone(),
                )),
                Err(KernelError::NotFound(_)) => None,
                Err(error) => return Err(kernel_error(error)),
            }
        } else {
            None
        };
        let _replay_slot = if native.is_some() {
            Some(
                NATIVE_REPLAYS
                    .try_acquire()
                    .map_err(|_| AppError::PersistenceBusy)?,
            )
        } else {
            None
        };
        let replay_context = if let Some((id, _, _, directories)) = &held {
            let state = self.state_for_session(*id).await?;
            let context = self
                .execution_context_for_execution(*id, &state, None)
                .await?;
            let executor = state.executor.for_session_context_async(&context).await;
            let directories = directories_for_agent(&executor, &cwd, directories).await?;
            session_context(agent.clone(), *id, &executor, &state, &context, directories, None).await
        } else {
            acp::SessionContext::default()
        };
        let mut client = acp::AcpClient::spawn_with_directories(
            spec.clone(),
            &cwd,
            &replay_context.additional_directories,
        )?;
        let outcome = async {
            let Some((native_id, access)) = native else {
                let page = client
                    .list_workspace_native(
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
                .find_workspace_native(&cwd, &native_id, filter, &cancel)
                .await
            {
                Ok(Some(row)) => row.session,
                Ok(None) if filter == ExternalAgentArchiveFilter::Unarchived => {
                    self.index_import_record(&spec, &cwd, &native_id, &cancel)
                        .await?
                }
                Err(error)
                    if filter == ExternalAgentArchiveFilter::Unarchived
                        && !matches!(error, AppError::Cancelled) =>
                {
                    match self
                        .index_import_record(&spec, &cwd, &native_id, &cancel)
                        .await
                    {
                        Ok(row) => row,
                        Err(_) => return Err(error),
                    }
                }
                Ok(None) => {
                    return Err(AppError::Config(
                        "That native conversation is not listed in this workspace.".into(),
                    ));
                }
                Err(error) => return Err(error),
            };
            if let Some((id, _, _, _)) = &held {
                // Browsing needs the connection slot. A session/close would
                // also stop the native session's independent background work.
                self.close_idle_external_agent(*id).await;
            }
            if let HistoryAccess::Preview(before_turn) = access {
                let preview = client
                    .preview_native_history(&cwd, &native_id, &replay_context, before_turn, &cancel)
                    .await?;
                return Ok(ExternalAgentWorkspaceResult::HistoryPreview { preview });
            }
            let (turns, mut state) = client
                .replay_native(&cwd, &native_id, &replay_context, &cancel)
                .await?;
            let title = state
                .metadata
                .title
                .clone()
                .or_else(|| record.title.clone())
                .unwrap_or_else(|| format!("{} conversation", spec.name));
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
                turns: turns.into(),
                state,
                replace_view: held.as_ref().map(|(id, _, _, _)| *id),
            };
            let created = persist(|| {
                self.kernel()
                    .sessions
                    .import_external_agent(request, import.clone())
            })
            .await
            .map_err(kernel_error)?;
            let session_id = created.value.id.get();
            Ok(if access == HistoryAccess::Open {
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
            .clone()
            .ok_or_else(|| AppError::Config("This session has no external agent.".into()))?;
        let spec = self.resolve_external_agent(&agent)?;
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
        let native_directories = if matches!(
            action,
            ExternalAgentSessionAction::Fork { .. } | ExternalAgentSessionAction::Rewind { .. }
        ) {
            directories_for_agent(&executor, &cwd, &configuration.external_agent_directories)
                .await?
        } else {
            Vec::new()
        };
        let mut client =
            acp::AcpClient::spawn_with_directories(spec, &cwd, &native_directories)?;
        if matches!(action, ExternalAgentSessionAction::Delete) {
            // Claude v1 maps Delete to Archive on legacy AIR connections.
            client.use_plain_delete();
        }
        let outcome = async {
            let capabilities = client.lifecycle_capabilities(&cancel).await?;
            if matches!(action, ExternalAgentSessionAction::Status) {
                let native_status = match &link {
                    Some(_) if running => Some("open".to_owned()),
                    Some(link) if link.cwd == cwd => client.native_status(&link.native_id, &cancel).await?,
                    _ => None,
                };
                let mut fork_points = Vec::new();
                let mut rewind_points = Vec::new();
                if let Some(link) = link.as_ref().filter(|link| link.cwd == cwd)
                    && (capabilities.fork_before_message || capabilities.rewind_before_message)
                {
                    for (cursor, label) in self.kernel().reader.external_agent_fork_points(session, link, None)
                        .await.map_err(kernel_error)? {
                        let point = ExternalAgentForkPoint { message_id: cursor.message_id.get(), label };
                        if cursor.fork && capabilities.fork_before_message {
                            fork_points.push(point.clone());
                        }
                        if cursor.rewind && capabilities.rewind_before_message {
                            rewind_points.push(point);
                        }
                    }
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
                    rewind_points,
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
                ExternalAgentSessionAction::Rewind { before_message_id } => {
                    if !capabilities.rewind_before_message {
                        return Err(AppError::Config("The installed agent does not offer native rewind over this ACP connection.".into()));
                    }
                    let message = entry_id(before_message_id)?;
                    self.close_idle_external_agent(session).await;
                    let (before, _) = self.kernel().reader.external_agent_fork_points(session, &link, Some(message))
                        .await.map_err(kernel_error)?.into_iter().find(|(cursor, _)| cursor.rewind).ok_or_else(|| {
                            AppError::Config("That message has no native rewind point. Reopen the native history to refresh available positions.".into())
                        })?;
                    let cursor = before.cursor_id.as_deref().ok_or_else(|| AppError::Internal("Missing native rewind cursor.".into()))?;
                    let context = session_context(agent, session, &executor, &state, &context, native_directories, None).await;
                    client.rewind_native(&cwd, &link.native_id, cursor, &context, &cancel).await?;
                    let rewind = agena_storage::kernel::ExternalAgentRewind {
                        expected_membership: snapshot.metadata.membership_generation,
                        expected_native: link,
                        before,
                    };
                    let request = RequestId::new();
                    persist(|| self.kernel().sessions.rewind_external_agent(request, session, rewind.clone()))
                        .await.map_err(kernel_error)?;
                    Ok(ExternalAgentSessionResult::Rewound { session_id: id })
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
                            .await.map_err(kernel_error)?.into_iter().find(|(cursor, _)| cursor.fork).ok_or_else(|| {
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
                    let mut context = session_context(agent.clone(), session, &executor, &state, &context, native_directories, None).await;
                    context.selected_options.clone_from(&configuration.external_agent_selection);
                    if capabilities.fork {
                        self.close_idle_external_agent(session).await;
                    }
                    let new_id = client.fork_native(
                        &cwd, &link.native_id, before.as_ref().and_then(|cursor| cursor.cursor_id.as_deref()), &context, &cancel,
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
                    if result.is_err() {
                        if capabilities.delete {
                            let _ = client.release_native(&new_id, true, &cancel).await;
                        } else if capabilities.close {
                            let _ = client.release_native(&new_id, false, &cancel).await;
                        }
                    } else if capabilities.close {
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

pub(super) async fn session_context(
    agent: ExternalAgent,
    session: SessionId,
    executor: &ToolExecutor,
    state: &ExecutionState,
    context: &crate::session::model::SessionExecutionContext,
    additional_directories: Vec<PathBuf>,
    system: Option<&str>,
) -> acp::SessionContext {
    let system_prompt =
        external_agent_system_prompt(executor, state, context, agent, session, system)
            .await
            .text;
    acp::SessionContext {
        system_prompt,
        additional_directories,
        ..Default::default()
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
