use anyhow::{Context, anyhow};

impl Backend {
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
            &self.app_state,
            Query::GetSession(GetSessionParams { session_id }),
        )
        .await
        {
            Ok(QueryResult::Session(session)) => Ok(Some(session)),
            Ok(other) => Err(anyhow!("unexpected query result: {:?}", other))
                .context("failed to fetch session"),
            Err(agena_api_server::error::ServerError::NotFound(_)) => Ok(None),
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
    ) -> Result<Vec<DomainEvent>> {
        let manager = self.session_manager()?;
        let mut all = manager
            .list_session_events(session_id)
            .await
            .context("failed to list session events")?;
        all.sort_by(|a, b| a.meta.seq_global.cmp(&b.meta.seq_global));
        if all.len() > limit as usize {
            all = all.split_off(all.len() - limit as usize);
        }
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
            &self.app_state,
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
        let session = self
            .session_manager()?
            .get_session(session_id)
            .await
            .with_context(|| format!("failed to load session {session_id}"))?;
        let agent_name = execution.execution.agent_profile.clone();
        let agent_permission = agent_name
            .as_deref()
            .and_then(|name| self.get_agent_profile(name))
            .map(|profile| profile.frontmatter.permission);
        Ok(SessionPermissionStudioState {
            session_id,
            session_title: execution.session.title.clone(),
            agent_name,
            agent_permission,
            permission: session.runtime().execution.selection.permission.clone(),
            effective_permission: execution.execution.effective_permission.clone(),
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
            &self.app_state,
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
        let manager = self.session_manager()?;
        let all_events = manager
            .list_session_events(session_id)
            .await
            .context("failed to fetch latest session event sequence")?;
        let latest_event_seq = all_events.iter().map(|event| event.meta.seq_global).max();
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
            Some(after) => all_events
                .iter()
                .filter(|event| event.meta.seq_global > after)
                .take(256)
                .count(),
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

    /// Subscribe to live events for a session via the unified
    /// [`agena_event::EventBus`]. Callers receive a [`LiveEvent`] for every
    /// domain event the session emits, in real time.
    ///
    /// Returns `None` when the runtime has no session manager configured
    /// (e.g. a database-less smoke test).
    pub fn subscribe_session_events(
        &self,
        session_id: i64,
    ) -> Option<mpsc::UnboundedReceiver<LiveEvent>> {
        let manager = self.runtime.session_manager()?;
        let bus = manager.event_bus();
        let (tx, rx) = mpsc::unbounded_channel::<LiveEvent>();
        let mut subscription = bus.subscribe(EventFilter::new(Scope::Session { session_id }));
        tokio::spawn(async move {
            while let Some(item) = subscription.recv().await {
                let event = match item {
                    SubscriptionItem::Event(event) => Some((*event).clone()),
                    SubscriptionItem::Lagged(_) => None,
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
                let triggers_refresh = session_event_requires_refresh(&event.kind);
                let force_refresh = permission_event_requires_forced_refresh(&event.kind);
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
        parts: Vec<PartContent>,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.app_state,
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
        let store = self.memory_store();
        store
            .ensure_exists()
            .context("failed to create memory directory")?;
        let path = store.dir().join("MEMORY.md");
        if !path.exists() {
            fs::write(&path, "")
                .with_context(|| format!("failed to create memory index {}", path.display()))?;
        }
        Ok(path)
    }

    pub fn memory_entry_path(&self, name: &str) -> Result<PathBuf> {
        self.memory_store()
            .get(name)
            .with_context(|| format!("failed to load memory `{name}`"))
            .map(|entry| entry.path)
    }

    pub fn forget_memory(&self, name: &str) -> Result<()> {
        self.memory_store()
            .forget(name)
            .with_context(|| format!("failed to forget memory `{name}`"))
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
            &self.app_state,
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
            &self.app_state,
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
        permission: agena::agent::PermissionConfig,
    ) -> Result<SessionExecutionResource> {
        self.session_manager()?
            .set_session_permission(session_id, permission)
            .await
            .with_context(|| format!("failed to set permission for session {session_id}"))?;
        self.get_session_state(session_id).await
    }

    /// Best-effort cancel of the in-flight run for `session_id`. Forwards
    /// to `SessionManager::cancel_active_run`; the manager owns the
    /// `CancellationToken` for the spawned run task. If no run is
    /// active this is a no-op.
    pub async fn cancel_run(&self, session_id: i64) -> Result<()> {
        self.session_manager()?
            .cancel_active_run(session_id)
            .await
            .context("failed to cancel active run")
    }

    /// Inject `parts` as a steer message into the in-flight run. Returns
    /// `Err` when there is no active run or the run is in a phase that
    /// no longer accepts steers (the caller should re-queue).
    pub async fn steer_input(&self, session_id: i64, parts: Vec<PartContent>) -> Result<()> {
        self.session_manager()?
            .steer_input(session_id, parts)
            .await
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
            &self.app_state,
            ApiCommand::ReplyPermission(ReplyPermissionParams {
                session_id,
                options: request,
                reply: PermissionReply {
                    request_id,
                    kind,
                    reason: None,
                    scope,
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
            &self.app_state,
            ApiCommand::ReplyUserInput(ReplyUserInputParams {
                session_id,
                options: request,
                reply,
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
            &self.app_state,
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
use crate::backend::Result;
use crate::backend::{
    ApiCommand, AttachmentItem, AttachmentKind, AttachmentSource, Backend, CommandResult,
    CompactSessionParams, ContinueRunParams, DomainEvent, EventFilter, GetSessionParams, HashSet,
    ListMessagesParams, ListSessionsParams, LiveEvent, MAX_ATTACHMENT_BYTES, MessageResource,
    PaginatedResponse, PartContent, PartLoadMode, Path, PathBuf, PermissionReply,
    PermissionReplyKind, PermissionScope, Query, QueryResult, ReplyPermissionParams,
    ReplyUserInputParams, RewindSessionParams, RunOptions, STANDARD, Scope,
    SessionExecutionResource, SessionPermissionStudioState, SessionRefresh, SessionResource,
    SubmitMessageParams, SubscriptionItem, UserInputReply, api_error, build_file_index,
    detect_dimensions, detect_mime, direct_path_candidate, dispatch, file_search_score, fs, mpsc,
    permission_event_requires_forced_refresh, session_event_requires_refresh,
};
use base64::Engine as _;
