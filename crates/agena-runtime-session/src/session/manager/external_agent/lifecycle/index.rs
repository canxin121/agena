use std::collections::BTreeSet;

use super::*;

static INDEX_WATCH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
const DEFAULT_WAIT_SECONDS: u16 = 20;
const MAX_WAIT_SECONDS: u16 = 30;

impl SessionManager {
    pub(super) async fn manage_native_index(
        &self,
        workspace: i64,
        action: ExternalAgentWorkspaceAction,
    ) -> Result<ExternalAgentWorkspaceResult, AppError> {
        use ExternalAgentWorkspaceAction as Action;
        let agent = match &action {
            Action::IndexList { agent, cursor, .. } => {
                if cursor
                    .as_ref()
                    .is_some_and(|value| value.len() > acp::MAX_CURSOR_BYTES + 64)
                {
                    return Err(AppError::Config(
                        "Invalid external agent history cursor.".into(),
                    ));
                }
                agent.clone()
            }
            Action::BatchGet { agent, native_ids } => {
                if native_ids.is_empty()
                    || native_ids.len() > acp::MAX_BATCH_IDS
                    || native_ids.iter().any(|id| invalid_index_id(id))
                    || native_ids.iter().collect::<BTreeSet<_>>().len() != native_ids.len()
                {
                    return Err(AppError::Config(format!(
                        "Specify 1 to {} distinct native conversation IDs.",
                        acp::MAX_BATCH_IDS,
                    )));
                }
                agent.clone()
            }
            Action::SetArchived {
                agent, native_id, ..
            } => {
                if invalid_native_string(native_id) {
                    return Err(AppError::Config(
                        "Invalid external agent conversation ID.".into(),
                    ));
                }
                agent.clone()
            }
            Action::SetTitle {
                agent,
                native_id,
                title,
            } => {
                if invalid_native_string(native_id) {
                    return Err(AppError::Config(
                        "Invalid external agent conversation ID.".into(),
                    ));
                }
                let title = validated_title(title.clone())?;
                return self
                    .mutate_workspace_native_session(
                        workspace,
                        agent.clone(),
                        native_id.clone(),
                        WorkspaceNativeMutation::SetTitle(title),
                    )
                    .await;
            }
            Action::Delete { agent, native_id } => {
                if invalid_native_string(native_id) {
                    return Err(AppError::Config(
                        "Invalid external agent conversation ID.".into(),
                    ));
                }
                return self
                    .mutate_workspace_native_session(
                        workspace,
                        agent.clone(),
                        native_id.clone(),
                        WorkspaceNativeMutation::Delete,
                    )
                    .await;
            }
            Action::Watch {
                agent,
                wait_seconds,
            } => {
                let wait = wait_seconds.unwrap_or(DEFAULT_WAIT_SECONDS);
                if !(1..=MAX_WAIT_SECONDS).contains(&wait) {
                    return Err(AppError::Config(format!(
                        "Wait must be 1 to {MAX_WAIT_SECONDS} seconds."
                    )));
                }
                let cwd = canonical_directory(&self.workspace_root_by_id(workspace).await?).await?;
                return self
                    .watch_native_index(agent.clone(), cwd, Duration::from_secs(wait.into()))
                    .await;
            }
            Action::List { .. }
            | Action::Import { .. }
            | Action::Open { .. }
            | Action::PreviewHistory { .. } => {
                unreachable!("ordinary native lifecycle request")
            }
        };
        let spec = self.resolve_external_agent(&agent)?;
        let cwd = canonical_directory(&self.workspace_root_by_id(workspace).await?).await?;
        let _permit = self.reserve_execution()?;
        let cancel = self.shutdown.child_token();
        let _archive_lock = if let Action::SetArchived { native_id, .. } = &action {
            let lock = native_replay_lock(&cwd, &agent, native_id);
            let held = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                held = lock.lock_owned() => held,
            };
            let link = ExternalAgentLink {
                native_id: native_id.clone(),
                cwd: cwd.clone(),
                incarnation: None,
                resume_at: None,
            };
            if let Some(id) = self
                .kernel_runtime
                .acp_view(workspace, agent.clone(), &link)
                .await
                .map_err(kernel_error)?
            {
                let dispatch = self.session_dispatch(id);
                let _submission = dispatch.submission.try_lock().map_err(|_| {
                    AppError::Config(
                        "Finish or stop the active turn before changing archive state.".into(),
                    )
                })?;
                let _execution = dispatch.execution.try_lock().map_err(|_| {
                    AppError::Config(
                        "Finish or stop the active turn before changing archive state.".into(),
                    )
                })?;
                if let Ok(snapshot) = self.kernel().reader.control_snapshot(id, 0).await
                    && (snapshot.summary.running_executions != 0
                        || snapshot.summary.queued_executions != 0)
                {
                    return Err(AppError::Config(
                        "Finish or stop the active turn before changing archive state.".into(),
                    ));
                }
                self.close_idle_external_agent(id).await;
            }
            Some(held)
        } else {
            None
        };
        let fallback = matches!(
            action,
            Action::IndexList {
                archived: ExternalAgentArchiveFilter::Unarchived,
                ..
            }
        );
        if let Action::IndexList {
            cursor, archived, ..
        } = action
        {
            let allow_without_archive = agent != ExternalAgent::ClaudeCode && fallback;
            let mut negotiation_error = None;
            match self
                .external_control_plane
                .list_standard_history(
                    &spec,
                    &cwd,
                    cursor.as_deref(),
                    archived,
                    allow_without_archive,
                    &self.control_tasks,
                    &self.shutdown,
                    &cancel,
                )
                .await
            {
                Ok(control_plane::ControlListOutcome::Page {
                    sessions,
                    next_cursor,
                    capabilities,
                }) => {
                    return Ok(ExternalAgentWorkspaceResult::IndexPage {
                        sessions,
                        next_cursor,
                        capabilities,
                    });
                }
                Ok(control_plane::ControlListOutcome::NeedsAirIndexFallback) => {
                    if agent != ExternalAgent::ClaudeCode {
                        return Err(unsupported_index());
                    }
                }
                Err(error)
                    if agent == ExternalAgent::ClaudeCode
                        && !matches!(error, AppError::Cancelled) =>
                {
                    negotiation_error = Some(error);
                }
                Err(error) => return Err(error),
            }
            let opened = match acp::AcpClient::open_index(spec.clone(), &cwd, &cancel).await {
                Ok(opened) => opened,
                Err(error) if fallback && !matches!(error, AppError::Cancelled) => {
                    tracing::debug!(%error, "could not negotiate optional Claude session index");
                    None
                }
                Err(error) => return Err(error),
            };
            let Some((mut client, capabilities)) = opened else {
                if let Some(error) = negotiation_error {
                    return Err(error);
                }
                if fallback {
                    return match self
                        .external_control_plane
                        .list_standard_history(
                            &spec,
                            &cwd,
                            cursor.as_deref(),
                            archived,
                            true,
                            &self.control_tasks,
                            &self.shutdown,
                            &cancel,
                        )
                        .await?
                    {
                        control_plane::ControlListOutcome::Page {
                            sessions,
                            next_cursor,
                            capabilities,
                        } => Ok(ExternalAgentWorkspaceResult::IndexPage {
                            sessions,
                            next_cursor,
                            capabilities,
                        }),
                        control_plane::ControlListOutcome::NeedsAirIndexFallback => {
                            Err(unsupported_index())
                        }
                    };
                }
                return Err(unsupported_index());
            };
            let outcome = async {
                if archived != ExternalAgentArchiveFilter::Unarchived && !capabilities.archive {
                    return Err(unsupported_index());
                }
                let (sessions, next_cursor) = client
                    .list_indexed(&cwd, cursor.as_deref(), archived, &cancel)
                    .await?;
                Ok(ExternalAgentWorkspaceResult::IndexPage {
                    sessions,
                    next_cursor,
                    capabilities,
                })
            }
            .await;
            self.retire_external_agent_client(client).await;
            return outcome;
        }
        let is_archive_mutation = matches!(action, Action::SetArchived { .. });
        let mut standard = match &action {
            Action::SetArchived { .. } => Some(self.spawn_external_agent(spec.clone(), &cwd)?),
            _ => None,
        };
        let outcome = async {
            let mut negotiation_error = None;
            if let Some(client) = standard.as_mut() {
                match client.supports_native_archive(&cancel).await {
                    Ok(true) => {
                        self.record_external_agent_memo(&spec, client);
                        return standard_history(client, &cwd, action, &cancel).await;
                    }
                    Ok(false) => {
                        self.record_external_agent_memo(&spec, client);
                        if !fallback
                            && let Some(client) = standard.take()
                        {
                            self.retire_external_agent_client(client).await;
                        }
                    }
                    Err(error)
                        if agent == ExternalAgent::ClaudeCode
                            && !matches!(error, AppError::Cancelled) =>
                    {
                        negotiation_error = Some(error);
                        if let Some(client) = standard.take() {
                            self.retire_external_agent_client(client).await;
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            let opened = match acp::AcpClient::open_index(spec.clone(), &cwd, &cancel).await {
                Ok(opened) => opened,
                Err(error) if fallback && !matches!(error, AppError::Cancelled) => {
                    tracing::debug!(%error, "could not negotiate optional Claude session index");
                    None
                }
                Err(error) => return Err(error),
            };
            let Some((mut client, capabilities)) = opened else {
                if let Some(error) = negotiation_error {
                    return Err(error);
                }
                if fallback && let Some(client) = standard.as_mut() {
                    return standard_history(client, &cwd, action, &cancel).await;
                }
                return Err(unsupported_index());
            };
            let outcome = async {
                match action {
                    Action::BatchGet { native_ids, .. } if capabilities.batch_get => {
                        let sessions = client.batch_indexed(&cwd, &native_ids, &cancel).await?;
                        Ok(ExternalAgentWorkspaceResult::Batch { sessions })
                    }
                    Action::SetArchived {
                        native_id,
                        archived,
                        ..
                    } if capabilities.archive => {
                        if invalid_index_id(&native_id) {
                            return Err(AppError::Config(
                                "Invalid external agent conversation ID.".into(),
                            ));
                        }
                        client
                            .set_archived_indexed(&cwd, native_id.clone(), archived, &cancel)
                            .await?;
                        Ok(ExternalAgentWorkspaceResult::Archive {
                            native_id,
                            archived,
                        })
                    }
                    _ => Err(unsupported_index()),
                }
            }
            .await;
            self.retire_external_agent_client(client).await;
            outcome
        }
        .await;
        if let Some(client) = standard {
            self.retire_external_agent_client(client).await;
        }
        if is_archive_mutation && outcome.is_ok() {
            self.invalidate_external_agent_control_plane(&agent, true)
                .await;
        }
        outcome
    }

    pub(super) async fn index_import_record(
        &self,
        spec: &agena_domain::ResolvedExternalAgent,
        cwd: &Path,
        native_id: &str,
        cancel: &CancellationToken,
    ) -> Result<ExternalAgentSessionInfo, AppError> {
        let not_found =
            || AppError::Config("That native conversation is not listed in this workspace.".into());
        if invalid_index_id(native_id) {
            return Err(not_found());
        }
        let opened = match acp::AcpClient::open_index(spec.clone(), cwd, cancel).await {
            Ok(opened) => opened,
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                tracing::debug!(%error, "optional native history lookup was unavailable");
                None
            }
        };
        let Some((mut client, capabilities)) = opened else {
            return Err(not_found());
        };
        let row = if capabilities.batch_get {
            client
                .batch_indexed(cwd, &[native_id.to_owned()], cancel)
                .await
        } else {
            Ok(Vec::new())
        };
        client.close().await;
        Ok(row?.into_iter().next().ok_or_else(not_found)?.session)
    }

    async fn watch_native_index(
        &self,
        agent: ExternalAgent,
        cwd: PathBuf,
        wait: Duration,
    ) -> Result<ExternalAgentWorkspaceResult, AppError> {
        let spec = self.resolve_external_agent(&agent)?;
        if agent != ExternalAgent::ClaudeCode {
            return Ok(ExternalAgentWorkspaceResult::Watch {
                supported: false,
                changed: false,
            });
        }
        let permit = INDEX_WATCH_SLOTS.try_acquire().map_err(|_| {
            AppError::Config("Too many native history subscriptions are open.".into())
        })?;
        let cancel = self.shutdown.child_token();
        let _disconnect = cancel.clone().drop_guard();
        self.control_tasks
            .spawn(async move {
                let _permit = permit;
                let Some((mut client, capabilities)) =
                    acp::AcpClient::open_index(spec, &cwd, &cancel).await?
                else {
                    return Ok(ExternalAgentWorkspaceResult::Watch {
                        supported: false,
                        changed: false,
                    });
                };
                let result = if capabilities.watch {
                    client.watch_indexed(&cwd, wait, &cancel).await
                } else {
                    Ok(false)
                };
                client.close().await;
                Ok(ExternalAgentWorkspaceResult::Watch {
                    supported: capabilities.watch,
                    changed: result?,
                })
            })
            .await
            .map_err(|error| {
                AppError::Internal(format!("Native history subscription stopped: {error}"))
            })?
    }

    async fn mutate_workspace_native_session(
        &self,
        workspace: i64,
        agent: ExternalAgent,
        native_id: String,
        mutation: WorkspaceNativeMutation,
    ) -> Result<ExternalAgentWorkspaceResult, AppError> {
        let spec = self.resolve_external_agent(&agent)?;
        let cwd = canonical_directory(&self.workspace_root_by_id(workspace).await?).await?;
        let _permit = self.reserve_execution()?;
        let cancel = self.shutdown.child_token();
        let lock = native_replay_lock(&cwd, &agent, &native_id);
        let _held = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AppError::Cancelled),
            held = lock.lock_owned() => held,
        };
        let lookup = ExternalAgentLink {
            native_id: native_id.clone(),
            cwd: cwd.clone(),
            incarnation: None,
            resume_at: None,
        };
        let open_view = self
            .kernel_runtime
            .acp_view(workspace, agent.clone(), &lookup)
            .await
            .map_err(kernel_error)?;
        let busy_message = match &mutation {
            WorkspaceNativeMutation::SetTitle(_) => {
                "Finish or stop the active turn before renaming this conversation."
            }
            WorkspaceNativeMutation::Delete => {
                "Finish or stop the active turn before deleting this conversation."
            }
        };
        let view_dispatch = open_view.map(|id| (id, self.session_dispatch(id)));
        let _view_guards = if let Some((id, dispatch)) = view_dispatch.as_ref() {
            let submission = dispatch
                .submission
                .try_lock()
                .map_err(|_| AppError::Config(busy_message.into()))?;
            let execution = dispatch
                .execution
                .try_lock()
                .map_err(|_| AppError::Config(busy_message.into()))?;
            if let Ok(snapshot) = self.kernel().reader.control_snapshot(*id, 0).await
                && (snapshot.summary.running_executions != 0
                    || snapshot.summary.queued_executions != 0)
            {
                return Err(AppError::Config(busy_message.into()));
            }
            self.close_idle_external_agent(*id).await;
            Some((submission, execution))
        } else {
            None
        };
        let mut client = self.spawn_external_agent(spec.clone(), &cwd)?;
        if matches!(mutation, WorkspaceNativeMutation::Delete) {
            // Claude v1 maps Delete to Archive on legacy AIR connections.
            client.use_plain_delete();
        }
        let outcome = async {
            match mutation {
                WorkspaceNativeMutation::SetTitle(title) => {
                    let saved = client.set_native_title(&native_id, &title, &cancel).await?;
                    if let Some(id) = open_view {
                        let request = RequestId::new();
                        persist(|| {
                            self.kernel().sessions.update_metadata(
                                request,
                                id,
                                MetadataChange {
                                    title: Some(saved.clone()),
                                    sync_external_agent_title: true,
                                    ..Default::default()
                                },
                            )
                        })
                        .await
                        .map_err(kernel_error)?;
                    }
                    Ok(ExternalAgentWorkspaceResult::Title {
                        native_id,
                        title: saved,
                    })
                }
                WorkspaceNativeMutation::Delete => {
                    client.release_native(&native_id, true, &cancel).await?;
                    if let Some(id) = open_view {
                        let stored_link = self
                            .kernel()
                            .reader
                            .configuration(id)
                            .await
                            .ok()
                            .and_then(|config| config.native_session_link())
                            .unwrap_or(lookup);
                        let request = RequestId::new();
                        let _ = persist(|| {
                            self.kernel()
                                .sessions
                                .detach_external_agent(request, id, stored_link.clone())
                        })
                        .await;
                    }
                    Ok(ExternalAgentWorkspaceResult::Deleted { native_id })
                }
            }
        }
        .await;
        self.record_external_agent_memo(&spec, &client);
        self.retire_external_agent_client(client).await;
        if outcome.is_ok() {
            self.invalidate_external_agent_control_plane(&agent, true)
                .await;
        }
        outcome
    }
}

enum WorkspaceNativeMutation {
    SetTitle(String),
    Delete,
}

async fn standard_history(
    client: &mut acp::AcpClient,
    cwd: &Path,
    action: ExternalAgentWorkspaceAction,
    cancel: &CancellationToken,
) -> Result<ExternalAgentWorkspaceResult, AppError> {
    match action {
        ExternalAgentWorkspaceAction::IndexList {
            cursor, archived, ..
        } => {
            let caps = client.lifecycle_capabilities(cancel).await?;
            let archive = client.supports_native_archive(cancel).await?;
            let page = client
                .list_workspace_native(cwd, cursor.as_deref(), archived, cancel)
                .await?;
            Ok(ExternalAgentWorkspaceResult::IndexPage {
                sessions: page.sessions,
                next_cursor: page.next_cursor,
                capabilities: agena_domain::ExternalAgentIndexCapabilities {
                    archive,
                    title: caps.title,
                    delete: caps.delete,
                    fork: caps.fork,
                    ..Default::default()
                },
            })
        }
        ExternalAgentWorkspaceAction::SetArchived {
            native_id,
            archived,
            ..
        } => {
            client
                .set_native_archived(cwd, &native_id, archived, cancel)
                .await?;
            Ok(ExternalAgentWorkspaceResult::Archive {
                native_id,
                archived,
            })
        }
        _ => unreachable!("operation without a standard ACP history equivalent"),
    }
}

fn unsupported_index() -> AppError {
    AppError::Config("The installed agent does not support this native history operation.".into())
}

fn invalid_index_id(value: &str) -> bool {
    value.trim() != value || invalid_native_string(value)
}
