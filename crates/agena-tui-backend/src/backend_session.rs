use anyhow::{Context, anyhow};

impl Backend {
    pub async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats> {
        self.application
            .session_query_service()
            .map_err(anyhow::Error::new)?
            .usage_stats(query)
            .await
            .map_err(anyhow::Error::new)
            .context("failed to load usage statistics")
    }

    pub async fn list_child_sessions(&self, parent_id: i64) -> Result<Vec<SessionResource>> {
        let workspace_id = self.current_workspace_id().await?;
        self.list_sessions_query(ListSessionsParams {
            cursor: None,
            limit: Some(200),
            workspace_id: Some(workspace_id),
            parent_id: Some(parent_id),
            roots: false,
            search: None,
        })
        .await
        .context("failed to list child sessions")
    }

    pub async fn get_session(&self, session_id: i64) -> Result<Option<SessionResource>> {
        match dispatch::dispatch_query(
            &self.application,
            Query::GetSession(GetSessionParams { session_id }),
        )
        .await
        {
            Ok(QueryResult::Session(session)) => Ok(Some(session)),
            Ok(other) => Err(anyhow!("unexpected query result: {:?}", other))
                .context("failed to fetch session"),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(api_error(error).context("failed to fetch session")),
        }
    }

    pub async fn list_session_subtree(&self, session_id: i64) -> Result<Vec<SessionResource>> {
        let root = self.resolve_session_root(session_id).await?;
        let mut items = vec![root.clone()];
        let mut seen = HashSet::from([root.id]);
        let mut stack = vec![root.id];

        while let Some(parent_id) = stack.pop() {
            let children = self
                .list_sessions_query(ListSessionsParams {
                    cursor: None,
                    limit: Some(200),
                    workspace_id: Some(root.workspace_id),
                    parent_id: Some(parent_id),
                    roots: false,
                    search: None,
                })
                .await
                .with_context(|| {
                    format!("failed to list subtree children for session {parent_id}")
                })?;
            for child in children {
                if seen.insert(child.id) {
                    stack.push(child.id);
                    items.push(child);
                }
            }
        }

        Ok(items)
    }

    pub async fn list_session_timeline(
        &self,
        session_id: i64,
        limit: u64,
    ) -> Result<Vec<SessionTimelineEntry>> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let view = self
            .application
            .session_store_facade()
            .map_err(anyhow::Error::new)?
            .load(session_id)
            .await
            .map_err(anyhow::Error::new)?;
        let visible = view
            .parts
            .into_iter()
            .filter(|part| part.visibility.visible_to_user())
            .collect::<Vec<_>>();
        let skip = visible.len().saturating_sub(limit);
        Ok(visible
            .into_iter()
            .skip(skip)
            .map(|part| SessionTimelineEntry {
                part_id: part.part_id,
                kind: part.kind,
                role: part.role.as_str().to_owned(),
                state: part.state.as_str().to_owned(),
                summary: part.summary,
                content: part.content,
                rendered_markdown: part.rendered_markdown,
                parent_part_id: part.parent_part_id,
                run_id: part.run_id,
                revision: part.revision,
                created_at_ms: part.created_at_ms,
                updated_at_ms: part.updated_at_ms,
            })
            .collect())
    }

    pub async fn get_session_state(&self, session_id: i64) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_query(
            &self.application,
            Query::GetSessionState(GetSessionParams { session_id }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::SessionState(state) => Ok(state),
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to load session state")
    }

    /// Lazily fetch the human-facing detail of one tool Activity on expansion.
    pub async fn get_operation_detail(
        &self,
        session_id: i64,
        activity_id: ActivityId,
    ) -> Result<OperationDetailResource> {
        match dispatch::dispatch_query(
            &self.application,
            Query::GetOperationDetail(GetOperationDetailParams {
                session_id,
                activity_id,
            }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::OperationDetail(detail) => Ok(detail),
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to load operation detail")
    }

    pub async fn get_session_permission_studio_state(
        &self,
        session_id: i64,
    ) -> Result<SessionPermissionStudioState> {
        let execution = self.get_session_state(session_id).await?;
        let execution_context = self
            .application
            .session_execution_services()?
            .queries
            .execution_context(session_id)
            .await
            .map_err(anyhow::Error::new)
            .with_context(|| {
                format!("failed to load execution context for session {session_id}")
            })?;
        Ok(SessionPermissionStudioState {
            session_id,
            session_title: execution.session.title.clone(),
            permission: execution_context.selected_permission,
            effective_permission: serde_json::from_value(
                serde_json::to_value(&execution.execution.effective_permission)
                    .context("failed to serialize effective permission resource")?,
            )
            .context("failed to decode effective permission resource")?,
        })
    }

    pub async fn refresh_session(
        &self,
        session_id: i64,
        after_seq: Option<i64>,
        force: bool,
    ) -> Result<SessionRefresh> {
        let queries = self
            .application
            .session_query_service()
            .map_err(anyhow::Error::new)?;
        let latest_event_seq = queries
            .latest_event_seq(session_id)
            .await
            .map_err(anyhow::Error::new)?;
        let changed = force
            || match (after_seq, latest_event_seq) {
                (None, Some(_)) => true,
                (Some(after), Some(current)) => current > after,
                _ => false,
            };

        if !changed {
            return Ok(SessionRefresh {
                latest_event_seq,
                event_count: 0,
                execution: None,
            });
        }

        let event_count = after_seq
            .zip(latest_event_seq)
            .map(|(after, current)| current.saturating_sub(after).clamp(0, 256) as usize)
            .unwrap_or(0);

        let execution = self.get_session_state(session_id).await?;
        Ok(SessionRefresh {
            latest_event_seq,
            event_count,
            execution: Some(execution),
        })
    }

    /// Subscribe through the Runtime-owned typed presentation stream. Generic
    /// transport events remain available separately for timeline consumers.
    pub fn subscribe_session_events(
        &self,
        session_id: i64,
    ) -> Option<mpsc::UnboundedReceiver<LiveEvent>> {
        let store = self.application.session_store_facade().ok()?;
        let queries = self.application.session_execution_services().ok()?.queries;
        let (tx, rx) = mpsc::unbounded_channel::<LiveEvent>();
        let (change_tx, mut change_rx) = mpsc::unbounded_channel();
        let subscription = store.subscribe_all(std::sync::Arc::new(move |change| {
            let _ = change_tx.send(change);
        }));
        let change_output = tx.clone();
        tokio::spawn(async move {
            let _subscription = subscription;
            loop {
                let change = tokio::select! {
                    change = change_rx.recv() => change,
                    _ = change_output.closed() => break,
                };
                let Some(change) = change else { break };
                let event = presentation_event_from_session_change(change);
                if event.meta.session_id != Some(session_id) {
                    let Some(descendant_id) = event.meta.session_id else {
                        continue;
                    };
                    if !event.invalidates_ancestor_projection {
                        continue;
                    }
                    let is_descendant = queries
                        .is_descendant_session(descendant_id, session_id)
                        .await
                        .unwrap_or(false);
                    if !is_descendant {
                        continue;
                    }
                    if change_output
                        .send(LiveEvent {
                            event: None,
                            triggers_refresh: true,
                            force_refresh: true,
                        })
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                let live = LiveEvent {
                    event: Some(event),
                    triggers_refresh: true,
                    force_refresh: false,
                };
                if change_output.send(live).is_err() {
                    break;
                }
            }
        });
        if let Ok(signals) = self.application.live_signal_service() {
            let mut subscription = signals.subscribe();
            tokio::spawn(async move {
                loop {
                    let item = tokio::select! {
                        item = subscription.recv() => item,
                        _ = tx.closed() => break,
                    };
                    let Some(item) = item else { break };
                    let live = live_event_from_runtime_signal(item, session_id);
                    if let Some(live) = live
                        && tx.send(live).is_err()
                    {
                        break;
                    }
                }
            });
        }
        Some(rx)
    }

    pub async fn submit_document_with_options(
        &self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::SubmitMessage(SubmitRunParams {
                session_id,
                options: request,
                document,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to submit user message")
    }

    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::UpdateSessionSelection(UpdateSessionSelectionParams {
                session_id,
                options,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to update session model selection")
    }

    pub fn resolve_workspace_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        }
    }

    pub fn search_workspace_files(&self, query: &str, limit: usize) -> Result<Vec<PathBuf>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let trimmed = query.trim();
        let query_lower = trimmed.to_lowercase();
        let index = self
            .file_index
            .get_or_init(|| build_file_index(&self.workspace_root));

        let mut matches = index
            .iter()
            .filter_map(|path| {
                let score = file_search_score(path, query_lower.as_str())?;
                Some((score, path.clone()))
            })
            .collect::<Vec<_>>();

        if let Some(path) = direct_path_candidate(&self.workspace_root, trimmed) {
            let already_present = matches.iter().any(|(_, existing)| existing == &path);
            if !already_present {
                matches.push(((0, 0, 0), path));
            }
        }

        matches.sort_by(|(score_a, path_a), (score_b, path_b)| {
            score_a
                .cmp(score_b)
                .then_with(|| {
                    path_a
                        .components()
                        .count()
                        .cmp(&path_b.components().count())
                })
                .then_with(|| path_a.as_os_str().len().cmp(&path_b.as_os_str().len()))
                .then_with(|| path_a.cmp(path_b))
        });
        matches.truncate(limit);
        Ok(matches.into_iter().map(|(_, path)| path).collect())
    }

    pub async fn continue_session_with_options(
        &self,
        session_id: i64,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::ContinueRun(ContinueRunParams {
                session_id,
                options: request,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to continue session")
    }

    pub async fn compact_session_with_options(
        &self,
        session_id: i64,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::CompactSession(CompactSessionParams {
                session_id,
                options: request,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to compact session")
    }

    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> Result<SessionExecutionResource> {
        self.application
            .session_execution_services()
            .map_err(anyhow::Error::new)?
            .commands
            .set_session_permission(session_id, permission)
            .await
            .map_err(anyhow::Error::new)
            .with_context(|| format!("failed to set permission for session {session_id}"))?;
        self.get_session_state(session_id).await
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::CancelRun(agena_api::commands::CancelRunParams {
                session_id,
                execution_id,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Cancellation(result) => Ok(result),
            other => Err(anyhow!("unexpected command result: {other:?}")),
        }
        .context("failed to cancel active run")
    }

    /// Inject `parts` as a steer message into the active execution. Returns
    /// `Err` when there is no active run or the run is in a phase that
    /// no longer accepts steers (the caller should re-queue).
    pub async fn steer_input(
        &self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
    ) -> Result<()> {
        self.application
            .session_execution_services()
            .map_err(anyhow::Error::new)?
            .commands
            .steer_input(session_id, document)
            .await
            .map_err(anyhow::Error::new)
            .context("failed to steer run")
    }

    pub async fn reply_permission_with_options(
        &self,
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::ReplyPermission(ReplyPermissionParams {
                session_id,
                options: request,
                reply: PermissionReply {
                    request_id,
                    kind: match kind {
                        PermissionReplyKind::AllowOnce => {
                            agena_api::resource::PermissionReplyKind::AllowOnce
                        }
                        PermissionReplyKind::AllowAlways => {
                            agena_api::resource::PermissionReplyKind::AllowAlways
                        }
                        PermissionReplyKind::DenyOnce => {
                            agena_api::resource::PermissionReplyKind::DenyOnce
                        }
                        PermissionReplyKind::DenyAlways => {
                            agena_api::resource::PermissionReplyKind::DenyAlways
                        }
                        PermissionReplyKind::AutoApprove => {
                            agena_api::resource::PermissionReplyKind::AutoApprove
                        }
                    },
                    reason: None,
                    scope: scope.map(|scope| match scope {
                        PermissionScope::Session => agena_api::resource::PermissionScope::Session,
                        PermissionScope::Workspace => {
                            agena_api::resource::PermissionScope::Workspace
                        }
                        PermissionScope::Global => agena_api::resource::PermissionScope::Global,
                    }),
                },
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to reply to permission request")
    }

    pub async fn reply_user_input_with_options(
        &self,
        session_id: i64,
        reply: UserInputReply,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::ReplyUserInput(ReplyUserInputParams {
                session_id,
                options: request,
                reply: agena_api::resource::UserInputReply {
                    request_id: reply.request_id,
                    kind: match reply.kind {
                        agena_domain::UserInputReplyKind::Submit => {
                            agena_api::resource::UserInputReplyKind::Submit
                        }
                        agena_domain::UserInputReplyKind::Cancel => {
                            agena_api::resource::UserInputReplyKind::Cancel
                        }
                        agena_domain::UserInputReplyKind::Timeout => {
                            agena_api::resource::UserInputReplyKind::Timeout
                        }
                    },
                    answers: reply.answers,
                    reason: reply.reason,
                },
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to submit user input reply")
    }

    pub async fn rewind_session_to_turn(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
    ) -> Result<SessionExecutionResource> {
        let expected_version = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?
            .version;
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::RewindSession(RewindSessionParams {
                session_id,
                turn_id,
                expected_version: Some(expected_version),
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to rewind session to turn")
    }

    /// Clone a session's full history into a new child session — a real
    /// fork, unlike `create_session`, which starts an empty child. Used by
    /// the `/side` command to start a side conversation that inherits the
    /// parent's context while the parent run keeps going untouched.
    pub async fn fork_session(
        &self,
        session_id: i64,
        title: Option<String>,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::ForkSession(ForkSessionParams {
                session_id,
                at_message_id: None,
                title,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to fork session")
    }

    /// Durable, idempotent acknowledgement that an interactive user-input
    /// request has been shown to the user. The session manager persists the
    /// presentation, so a never-presented request still auto-popups after a
    /// restart or on another client, while a presented-but-unanswered request
    /// is surfaced through a persistent attention hint instead of a forced
    /// modal. Fire-and-forget friendly: replaying the same request is a no-op.
    pub async fn present_interactive_request(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::MarkInteractiveRequestPresented(MarkInteractiveRequestPresentedParams {
                session_id,
                request_id,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to mark interactive request presented")
    }
}

fn presentation_event_from_session_change(
    change: agena_storage::store::SessionChange,
) -> agena_runtime::RuntimePresentationEvent {
    let (session_id, workspace_id, seq_global, seq_session, created_at_ms) = match &change {
        agena_storage::store::SessionChange::PartAdded { session_id, part }
        | agena_storage::store::SessionChange::PartUpdated { session_id, part } => (
            *session_id,
            None,
            part.updated_at_ms,
            Some(part.revision),
            part.updated_at_ms,
        ),
        agena_storage::store::SessionChange::PartRemoved {
            session_id,
            part_id,
        } => (
            *session_id,
            None,
            *part_id,
            None,
            chrono::Utc::now().timestamp_millis(),
        ),
        agena_storage::store::SessionChange::SessionMetaUpdated { session_id, meta } => (
            *session_id,
            Some(meta.workspace_id),
            meta.version,
            Some(meta.version),
            meta.updated_at_ms,
        ),
    };
    agena_runtime::RuntimePresentationEvent {
        meta: agena_runtime::RuntimePresentationEventMeta {
            id: uuid::Uuid::new_v4(),
            seq_global,
            seq_session,
            session_id: Some(session_id),
            workspace_id,
            created_at: chrono::DateTime::from_timestamp_millis(created_at_ms)
                .unwrap_or(chrono::DateTime::UNIX_EPOCH),
            causation_id: None,
            correlation_id: None,
            envelope_schema: 1,
        },
        invalidates_ancestor_projection: true,
        durable: true,
        kind: agena_runtime::RuntimePresentationEventKind::PartPatch(Box::new(change)),
    }
}

fn live_event_from_runtime_signal(
    item: agena_runtime::RuntimeLiveSignalItem,
    selected_session_id: i64,
) -> Option<LiveEvent> {
    let signal = match item {
        agena_runtime::RuntimeLiveSignalItem::Lagged(_) => {
            return Some(LiveEvent {
                event: None,
                triggers_refresh: true,
                force_refresh: true,
            });
        }
        agena_runtime::RuntimeLiveSignalItem::Signal(signal) => signal,
    };
    let now = chrono::Utc::now();
    let (session_id, invalidates_ancestor_projection, kind) = match signal {
        agena_runtime::RuntimeLiveSignal::Activity(activity) => {
            let session_id = activity.activity.session_id;
            let invalidates_ancestor = activity.activity.parent_session_id
                == Some(selected_session_id)
                && session_id != Some(selected_session_id);
            if session_id != Some(selected_session_id) && !invalidates_ancestor {
                return None;
            }
            (
                session_id,
                invalidates_ancestor,
                agena_runtime::RuntimePresentationEventKind::ActivityChanged {
                    activity: Box::new(activity.activity),
                    reason: activity.reason,
                },
            )
        }
        agena_runtime::RuntimeLiveSignal::Plugin { session_id, .. } => {
            if session_id != Some(selected_session_id) {
                return None;
            }
            (
                session_id,
                false,
                agena_runtime::RuntimePresentationEventKind::Refresh {
                    force_refresh: false,
                },
            )
        }
        agena_runtime::RuntimeLiveSignal::ToolRegistryChanged(_) => return None,
    };
    Some(LiveEvent {
        event: Some(agena_runtime::RuntimePresentationEvent {
            meta: agena_runtime::RuntimePresentationEventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: now.timestamp_millis(),
                seq_session: None,
                session_id,
                workspace_id: None,
                created_at: now,
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            invalidates_ancestor_projection,
            durable: false,
            kind,
        }),
        triggers_refresh: true,
        force_refresh: false,
    })
}

use crate::Result;
use crate::{
    ActivityId, ApiCommand, Backend, CommandResult, CompactSessionParams, ContinueRunParams,
    ForkSessionParams, GetOperationDetailParams, GetSessionParams, HashSet, ListSessionsParams,
    LiveEvent, MarkInteractiveRequestPresentedParams, OperationDetailResource, Path, PathBuf,
    PermissionReply, PermissionReplyKind, PermissionScope, Query, QueryResult,
    ReplyPermissionParams, ReplyUserInputParams, RewindSessionParams, RunOptions,
    SessionExecutionResource, SessionPermissionStudioState, SessionRefresh, SessionResource,
    SessionTimelineEntry, SubmitRunParams, UpdateSessionSelectionParams, UserInputReply, api_error,
    build_file_index, direct_path_candidate, dispatch, file_search_score, mpsc,
};
