use anyhow::{Context, anyhow};

impl Backend {
    pub async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats> {
        self.application
            .session_query_service()
            .map_err(|error| anyhow!(error.to_string()))?
            .usage_stats(query)
            .await
            .map_err(|error| anyhow!(error.to_string()))
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
            Err(agena_application::ApplicationError::NotFound(_)) => Ok(None),
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
    ) -> Result<Vec<agena_runtime::RuntimeTimelineEvent>> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let events = self
            .application
            .event_query_service()
            .map_err(|error| anyhow!(error.to_string()))?
            .list_timeline_events_before(
                &EventFilter::new(Scope::Session { session_id }),
                agena_runtime::RuntimeReverseEventRange {
                    before_seq_global: None,
                    limit,
                },
            )
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        let mut all = events;
        all.sort_by_key(|event| event.meta.seq_global);
        Ok(all)
    }

    pub async fn list_all_messages(&self, session_id: i64) -> Result<Vec<MessageResource>> {
        let mut cursor = None;
        let mut messages = Vec::new();

        loop {
            let page = self
                .list_messages_with_parts(session_id, cursor.clone(), 200, PartLoadMode::Full)
                .await
                .context("failed to list full session message history")?;
            cursor = page.page.next_cursor.clone();
            messages.extend(page.items);
            if cursor.is_none() {
                break;
            }
        }

        messages.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(messages)
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
            .map_err(|error| anyhow!(error.to_string()))
            .with_context(|| {
                format!("failed to load execution context for session {session_id}")
            })?;
        let agent_name = execution.execution.agent_profile.clone();
        let agent_permission = agent_name
            .as_deref()
            .and_then(|name| self.get_agent_profile(name))
            .map(|profile| profile.permission);
        Ok(SessionPermissionStudioState {
            session_id,
            session_title: execution.session.title.clone(),
            agent_name,
            agent_permission,
            permission: execution_context.selected_permission,
            effective_permission: serde_json::from_value(
                serde_json::to_value(&execution.execution.effective_permission)
                    .context("failed to serialize effective permission resource")?,
            )
            .context("failed to decode effective permission resource")?,
        })
    }

    pub async fn list_messages(
        &self,
        session_id: i64,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<MessageResource>> {
        // The transcript view should render history with the same shape and
        // styling as live events, so it always loads full parts here.
        self.list_messages_with_parts(session_id, cursor, limit, PartLoadMode::Full)
            .await
    }

    pub async fn list_messages_with_parts(
        &self,
        session_id: i64,
        cursor: Option<String>,
        limit: u64,
        parts: PartLoadMode,
    ) -> Result<PaginatedResponse<MessageResource>> {
        match dispatch::dispatch_query(
            &self.application,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor,
                limit: Some(limit),
                parts,
            }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::Messages(page) => Ok(page),
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to list session messages")
    }

    pub async fn refresh_session(
        &self,
        session_id: i64,
        after_seq: Option<i64>,
        latest_message_limit: u64,
        force: bool,
    ) -> Result<SessionRefresh> {
        let events = self
            .application
            .event_query_service()
            .map_err(|error| anyhow!(error.to_string()))?;
        let filter = EventFilter::new(Scope::Session { session_id });
        let latest_event_seq = events
            .list_events_before(
                &filter,
                agena_runtime::RuntimeReverseEventRange {
                    before_seq_global: None,
                    limit: 1,
                },
            )
            .await
            .map_err(|error| anyhow!(error.to_string()))?
            .into_iter()
            .map(|event| event.meta.seq_global)
            .max();
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
                latest_messages: None,
            });
        }

        let event_count = match after_seq {
            Some(after) => events
                .list_events(
                    &filter,
                    agena_runtime::RuntimeEventRange {
                        after_seq_global: after,
                        limit: 256,
                    },
                )
                .await
                .map_err(|error| anyhow!(error.to_string()))?
                .len(),
            None => 0,
        };

        let execution = self.get_session_state(session_id).await?;
        let latest_messages = self
            .list_messages_with_parts(session_id, None, latest_message_limit, PartLoadMode::Full)
            .await
            .context("failed to refresh latest message window")?;

        Ok(SessionRefresh {
            latest_event_seq,
            event_count,
            execution: Some(execution),
            latest_messages: Some(latest_messages),
        })
    }

    /// Subscribe through the Runtime-owned typed presentation stream. Generic
    /// transport events remain available separately for timeline consumers.
    pub fn subscribe_session_events(
        &self,
        session_id: i64,
    ) -> Option<mpsc::UnboundedReceiver<LiveEvent>> {
        let stream = self.application.event_stream_service().ok()?;
        let queries = self.application.session_execution_services().ok()?.queries;
        let (tx, rx) = mpsc::unbounded_channel::<LiveEvent>();
        // Interactive requests raised by delegated child sessions are exposed
        // through the selected parent's execution resource. Listen globally
        // and turn relevant descendant events into refresh-only signals.
        let mut subscription =
            stream.subscribe_presentation_events(EventFilter::new(Scope::Global))?;
        tokio::spawn(async move {
            while let Some(item) = subscription.recv().await {
                let event = match item {
                    agena_runtime::RuntimeLivePresentationSubscriptionItem::Event(event) => {
                        Some(*event)
                    }
                    agena_runtime::RuntimeLivePresentationSubscriptionItem::Lagged(_) => None,
                };
                if event.is_none() {
                    let live = LiveEvent {
                        event: None,
                        triggers_refresh: true,
                        force_refresh: true,
                    };
                    if tx.send(live).is_err() {
                        break;
                    }
                    continue;
                }
                let event = event.expect("event should exist after lag handling");
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
                    if tx
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
                let (triggers_refresh, force_refresh) = match &event.kind {
                    agena_runtime::RuntimePresentationEventKind::Refresh { force_refresh } => {
                        (true, *force_refresh)
                    }
                    _ => (false, false),
                };
                let live = LiveEvent {
                    event: Some(event),
                    triggers_refresh,
                    // A permission event is itself the state transition the
                    // UI needs to observe. Do not let an already-recorded
                    // event watermark turn this into an empty refresh.
                    force_refresh,
                };
                if tx.send(live).is_err() {
                    break;
                }
            }
        });
        Some(rx)
    }

    pub async fn submit_parts_message_with_options(
        &self,
        session_id: i64,
        parts: Vec<MessagePartContent>,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::SubmitMessage(SubmitMessageParams {
                session_id,
                options: request,
                parts,
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

    pub async fn switch_session_agent(
        &self,
        session_id: i64,
        agent_name: String,
    ) -> Result<SessionExecutionResource> {
        self.application
            .session_execution_services()
            .map_err(|error| anyhow!(error.to_string()))?
            .commands
            .set_session_agent(session_id, Some(agent_name))
            .await
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to switch session agent")?;
        self.get_session_state(session_id)
            .await
            .context("failed to reload session after switching agent")
    }

    pub fn prepare_attachment_from_path(&self, path: &Path) -> Result<AttachmentItem> {
        let resolved = self.resolve_workspace_path(path);

        if !resolved.exists() {
            return Err(anyhow!(
                "attachment path does not exist: {}",
                resolved.display()
            ));
        }
        if !resolved.is_file() {
            return Err(anyhow!(
                "attachment path is not a file: {}",
                resolved.display()
            ));
        }

        let bytes = fs::read(&resolved)
            .with_context(|| format!("failed to read attachment {}", resolved.display()))?;
        if bytes.is_empty() {
            return Err(anyhow!("attachment is empty: {}", resolved.display()));
        }
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(anyhow!(
                "attachments larger than {} bytes are not supported: {}",
                MAX_ATTACHMENT_BYTES,
                resolved.display()
            ));
        }

        let filename = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| resolved.display().to_string());
        let mime = detect_mime(&resolved, &bytes);
        let kind = AttachmentKind::detect(mime.as_str(), Some(filename.as_str()));
        let (width, height) = match kind {
            AttachmentKind::Image => detect_dimensions(&bytes),
            _ => (None, None),
        };

        Ok(AttachmentItem {
            kind,
            mime,
            source: AttachmentSource::Base64 {
                data: STANDARD.encode(&bytes),
            },
            filename: Some(filename),
            title: None,
            size_bytes: Some(bytes.len() as u64),
            sha256: None,
            width,
            height,
            duration_ms: None,
            page_count: None,
        })
    }

    pub fn resolve_workspace_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        }
    }

    pub fn memory_index_path(&self) -> Result<PathBuf> {
        self.application
            .service()
            .memory_index_path()
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub fn memory_entry_path(&self, name: &str) -> Result<PathBuf> {
        self.application
            .service()
            .memory_entry_path(name)
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub fn forget_memory(&self, name: &str) -> Result<()> {
        self.application
            .service()
            .forget_memory(name)
            .map_err(|error| anyhow!(error.to_string()))
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
            .map_err(|error| anyhow!(error.to_string()))?
            .commands
            .set_session_permission(session_id, permission)
            .await
            .map_err(|error| anyhow!(error.to_string()))
            .with_context(|| format!("failed to set permission for session {session_id}"))?;
        self.get_session_state(session_id).await
    }

    /// Best-effort cancel of the active execution for `session_id`. The shared
    /// Application command deliberately acknowledges a run that completed just
    /// before cancellation, so terminal and API consumers have one policy.
    pub async fn cancel_run(&self, session_id: i64) -> Result<()> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::CancelRun(agena_api::commands::CancelRunParams { session_id }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Ack => Ok(()),
            other => Err(anyhow!("unexpected command result: {other:?}")),
        }
        .context("failed to cancel active run")
    }

    /// Inject `parts` as a steer message into the active execution. Returns
    /// `Err` when there is no active run or the run is in a phase that
    /// no longer accepts steers (the caller should re-queue).
    pub async fn steer_input(&self, session_id: i64, parts: Vec<MessagePartContent>) -> Result<()> {
        let parts = parts
            .into_iter()
            .map(agena_application::session::session_user_message_part_from_wire)
            .collect();
        self.application
            .session_execution_services()
            .map_err(|error| anyhow!(error.to_string()))?
            .commands
            .steer_input(session_id, parts)
            .await
            .map_err(|error| anyhow!(error.to_string()))
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

    pub async fn rewind_session_to_message(
        &self,
        session_id: i64,
        message_id: i64,
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
                message_id,
                expected_version: Some(expected_version),
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to rewind session to message")
    }
}

pub fn message_attachment_to_wire(value: AttachmentItem) -> MessageAttachment {
    MessageAttachment {
        kind: match value.kind {
            AttachmentKind::Image => MessageAttachmentKind::Image,
            AttachmentKind::Audio => MessageAttachmentKind::Audio,
            AttachmentKind::Video => MessageAttachmentKind::Video,
            AttachmentKind::Pdf => MessageAttachmentKind::Pdf,
            AttachmentKind::File => MessageAttachmentKind::File,
        },
        mime: value.mime,
        source: match value.source {
            AttachmentSource::Url { url } => MessageAttachmentSource::Url { url },
            AttachmentSource::DataUrl { url } => MessageAttachmentSource::DataUrl { url },
            AttachmentSource::Base64 { data } => MessageAttachmentSource::Base64 { data },
            AttachmentSource::FileId { file_id } => MessageAttachmentSource::FileId { file_id },
            AttachmentSource::LocalPath { path } => MessageAttachmentSource::LocalPath { path },
        },
        filename: value.filename,
        title: value.title,
        size_bytes: value.size_bytes,
        sha256: value.sha256,
        width: value.width,
        height: value.height,
        duration_ms: value.duration_ms,
        page_count: value.page_count,
    }
}

use crate::Result;
use crate::{
    ApiCommand, AttachmentItem, AttachmentKind, AttachmentSource, Backend, CommandResult,
    CompactSessionParams, ContinueRunParams, EventFilter, GetSessionParams, HashSet,
    ListMessagesParams, ListSessionsParams, LiveEvent, MAX_ATTACHMENT_BYTES, MessageAttachment,
    MessageAttachmentKind, MessageAttachmentSource, MessageResource, PaginatedResponse,
    PartLoadMode, Path, PathBuf, PermissionReply, PermissionReplyKind, PermissionScope, Query,
    QueryResult, ReplyPermissionParams, ReplyUserInputParams, RewindSessionParams, RunOptions,
    STANDARD, Scope, SessionExecutionResource, SessionPermissionStudioState, SessionRefresh,
    SessionResource, SubmitMessageParams, UpdateSessionSelectionParams, UserInputReply, api_error,
    build_file_index, detect_dimensions, detect_mime, direct_path_candidate, dispatch,
    file_search_score, fs, mpsc,
};
use agena_api::resource::MessagePartContent;
use base64::Engine as _;
