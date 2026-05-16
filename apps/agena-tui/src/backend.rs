use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, OnceLock},
};

use agena::event::{EventFilter, Scope, bus::SubscriptionItem};
use agena::permission::PermissionScope;
use agena::{
    event::{DomainEvent, EventKind},
    memory::MemoryStore,
    message::{
        AttachmentItem, AttachmentKind, AttachmentSource, EnterWorktreeToolInput,
        ExitWorktreeToolInput, BundledToolInput, BundledToolOutput, PartContent,
        ToolInvocation, UserInputReply,
    },
    model::ModelRef,
    permission::PermissionReplyKind,
    provider::ProviderModel,
    runtime::AgenaRuntime,
    tool,
};
use agena_api::{
    commands::{
        Command as ApiCommand, CommandResult, ContinueRunParams, CreateSessionParams,
        ReplacePermissionRuleParams, ReplyPermissionParams, ReplyUserInputParams,
        RewindSessionParams, SubmitTurnParams, UpdateSessionParams, UpsertPermissionRuleParams,
    },
    pagination::PaginatedResponse,
    queries::{
        GetSessionParams, ListMessagesParams, ListPermissionRulesParams, ListSessionsParams, Query,
        QueryResult,
    },
    resource::{
        MessageResource, PartLoadMode, PermissionReply, PermissionRuleResource,
        ProviderSummaryResource, RunOptions, SessionExecutionResource, SessionResource,
        WorkspaceResource,
    },
};
use agena_api_server::{dispatch, state::AppState};
use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ignore::WalkBuilder;
use mime_guess::MimeGuess;
use sea_orm::DatabaseConnection;
use serde_json::json;
use tokio::sync::mpsc;

const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SessionRefresh {
    pub latest_event_seq: Option<i64>,
    pub event_count: usize,
    pub execution: Option<SessionExecutionResource>,
    pub latest_messages: Option<PaginatedResponse<MessageResource>>,
}

#[derive(Debug, Clone)]
struct GitStatusResource {
    workspace_root: String,
    git_available: bool,
    repo: bool,
    gh_available: bool,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    staged_files: u64,
    unstaged_files: u64,
    untracked_files: u64,
    changed_files: u64,
    clean: bool,
    worktree_active_sessions: u64,
    worktree_managed_dirs: u64,
}

#[derive(Debug, Clone)]
pub struct InspectorRow {
    pub label: String,
    pub detail: String,
}

/// Push notification emitted by the unified bus for the active session.
/// Indicates whether the change requires reloading messages.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// True for events that materially change session state — the UI should
    /// trigger a `refresh_session` after handling.
    pub triggers_refresh: bool,
}

#[derive(Clone)]
pub struct Backend {
    runtime: AgenaRuntime,
    app_state: AppState,
    workspace_root: PathBuf,
    file_index: Arc<OnceLock<Vec<PathBuf>>>,
}

impl Backend {
    pub fn new(
        runtime: AgenaRuntime,
        db: Arc<DatabaseConnection>,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            app_state: AppState::new(runtime.clone(), db),
            runtime,
            workspace_root,
            file_index: Arc::new(OnceLock::new()),
        }
    }

    pub async fn list_workspace_sessions(&self, roots_only: bool) -> Result<Vec<SessionResource>> {
        let workspace_id = self.current_workspace_id().await?;
        self.list_sessions_query(ListSessionsParams {
            cursor: None,
            limit: Some(200),
            workspace_id: Some(workspace_id),
            parent_id: None,
            roots: roots_only,
            search: None,
        })
        .await
        .context("failed to list workspace sessions")
    }

    pub async fn create_session(
        &self,
        title: String,
        parent_id: Option<i64>,
    ) -> Result<SessionResource> {
        let workspace = self
            .resolve_workspace_resource(true)
            .await
            .context("failed to resolve workspace for agena-tui")?;

        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::CreateSession(CreateSessionParams {
                workspace_id: workspace.id,
                title,
                parent_id,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Session(session) => Ok(session),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to create session")
    }

    pub async fn rename_session(&self, session_id: i64, title: String) -> Result<SessionResource> {
        let existing = self
            .get_session(session_id)
            .await
            .context("failed to load session before rename")?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;

        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::UpdateSession(UpdateSessionParams {
                session_id,
                title,
                parent_id: existing.parent_id,
                expected_version: Some(existing.version),
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Session(session) => Ok(session),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to rename session")
    }

    pub fn list_providers(&self) -> Vec<ProviderSummaryResource> {
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let mut providers = registry
            .provider_ids()
            .into_iter()
            .filter_map(|provider_id| {
                registry
                    .get(provider_id.as_str())
                    .map(|provider| ProviderSummaryResource {
                        default_model_ref: format!("{provider_id}/{}", provider.default_model()),
                        default_model: provider.default_model().to_string(),
                        catalog_default_model: None,
                        provider_id,
                    })
            })
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        providers
    }

    pub async fn list_provider_models(&self, provider_id: &str) -> Result<Vec<ProviderModel>> {
        self.runtime
            .current_snapshot()
            .list_provider_models(provider_id)
            .await
            .context("failed to list provider models")
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
                .list_messages(session_id, cursor.clone(), 200)
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

    pub async fn list_messages(
        &self,
        session_id: i64,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<MessageResource>> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor,
                limit: Some(limit),
                parts: PartLoadMode::Full,
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
            .list_messages(session_id, None, latest_message_limit)
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
    /// [`agena_event::EventBus`]. Replaces the legacy 250ms REST polling
    /// loop: callers receive a [`LiveEvent`] for every domain event the
    /// session emits, in real time.
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
                let SubscriptionItem::Event(event) = item else {
                    // `Lagged(n)` => UI should ask for a forced refresh; we
                    // emit a stub LiveEvent with `triggers_refresh = true`
                    // so the existing refresh handler runs.
                    continue;
                };
                let triggers_refresh = matches!(
                    event.kind,
                    EventKind::AssistantMessageCompleted(_)
                        | EventKind::ToolCallCompleted(_)
                        | EventKind::TurnCompleted(_)
                        | EventKind::TurnAborted(_)
                        | EventKind::SystemNoticeAppended(_)
                        | EventKind::MessageRevised(_)
                        | EventKind::RunFailed(_)
                );
                let live = LiveEvent { triggers_refresh };
                if tx.send(live).is_err() {
                    break;
                }
            }
        });
        Some(rx)
    }

    pub async fn submit_parts_turn_with_options(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::SubmitTurn(SubmitTurnParams {
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
        .context("failed to submit user turn")
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
        let options = self
            .resolve_session_run_options(session_id, request)
            .await?;
        self.session_manager()?
            .compact_session(session_id, options)
            .await
            .context("failed to compact session")?;
        self.get_session_state(session_id).await
    }

    /// Best-effort cancel of the in-flight turn for `session_id`. Forwards
    /// to `SessionManager::cancel_active_turn`; the manager owns the
    /// `CancellationToken` for the spawned turn task. If no turn is
    /// active this is a no-op.
    pub async fn cancel_turn(&self, session_id: i64) -> Result<()> {
        self.session_manager()?
            .cancel_active_turn(session_id)
            .await
            .context("failed to cancel active turn")
    }

    /// Inject `parts` as a steer message into the in-flight turn. Returns
    /// `Err` when there is no active turn or the turn is in a phase that
    /// no longer accepts steers (the caller should re-queue).
    pub async fn steer_input(&self, session_id: i64, parts: Vec<PartContent>) -> Result<()> {
        self.session_manager()?
            .steer_input(session_id, parts)
            .await
            .context("failed to steer turn")
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

    pub fn plugin_statusline_segments(&self) -> Vec<agena::plugin::HostStatuslineSegment> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .statusline_segments()
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn workspace_name(&self) -> String {
        self.workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.workspace_root.display().to_string())
    }

    pub fn plugin_theme_palettes(&self) -> Vec<agena::plugin::HostThemePalette> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .theme_palettes()
    }

    pub fn plugin_statuses(&self) -> Vec<agena::plugin::status::PluginStatus> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_statuses()
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<agena::plugin::PluginInspect> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_inspect(plugin_id)
    }

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena::plugin::PluginLogEntry> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_logs(plugin_id, after_seq, limit)
    }

    pub fn runtime_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let resolution = snapshot.config_resolution();
        let mut rows = vec![
            InspectorRow {
                label: "generation".to_string(),
                detail: snapshot.generation().to_string(),
            },
            InspectorRow {
                label: "workspace_root".to_string(),
                detail: self.workspace_root.display().to_string(),
            },
            InspectorRow {
                label: "config_path".to_string(),
                detail: resolution.meta.config_path.display().to_string(),
            },
            InspectorRow {
                label: "providers".to_string(),
                detail: snapshot.provider_registry().provider_ids().join(", "),
            },
            InspectorRow {
                label: "plugins".to_string(),
                detail: snapshot.plugin_manager().plugins().len().to_string(),
            },
            InspectorRow {
                label: "watch_paths".to_string(),
                detail: snapshot
                    .watch_paths()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" | "),
            },
        ];
        if let Some(manager) = snapshot.session_manager() {
            let stats = manager.cache_stats();
            rows.push(InspectorRow {
                label: "session_cache".to_string(),
                detail: format!(
                    "entries={} hits={} misses={} evictions={}",
                    stats.entry_count, stats.hits, stats.misses, stats.evictions
                ),
            });
        }
        rows
    }

    pub fn mcp_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let mut rows = snapshot
            .config_resolution()
            .config
            .mcp
            .servers
            .iter()
            .map(|(name, config)| InspectorRow {
                label: name.clone(),
                detail: match config {
                    agena::config::McpServerConfig::Stdio { command, args, .. } => {
                        format!("stdio {} {}", command, args.join(" "))
                    }
                    agena::config::McpServerConfig::Http { url, mode, .. } => {
                        format!("http {} {:?}", url, mode)
                    }
                },
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub fn lsp_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let mut rows = snapshot
            .config_resolution()
            .config
            .lsp
            .servers
            .iter()
            .map(|(name, config)| InspectorRow {
                label: name.clone(),
                detail: format!(
                    "{} | ext={} | roots={}",
                    config.command,
                    if config.file_extensions.is_empty() {
                        "all".to_string()
                    } else {
                        config.file_extensions.join(",")
                    },
                    if config.root_markers.is_empty() {
                        "workspace".to_string()
                    } else {
                        config.root_markers.join(",")
                    }
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub fn skills_inspector_rows(&self) -> Vec<InspectorRow> {
        let mut rows = self
            .runtime
            .current_snapshot()
            .plugin_manager()
            .entry_entries()
            .into_iter()
            .filter(|entry| entry.plugin_name == "agena.skills_fs")
            .map(|entry| InspectorRow {
                label: entry.exposed_name,
                detail: format!(
                    "{} | {}",
                    entry.plugin_name,
                    entry.decl.description.unwrap_or_default()
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub fn runtime_entry_rows(&self) -> Vec<InspectorRow> {
        let mut rows = self
            .runtime
            .current_snapshot()
            .plugin_manager()
            .entry_entries()
            .into_iter()
            .map(|entry| InspectorRow {
                label: entry.exposed_name,
                detail: format!(
                    "{} | {}",
                    entry.plugin_name,
                    entry.decl.description.unwrap_or_default()
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub async fn session_cost_inspector_rows(&self, session_id: i64) -> Result<Vec<InspectorRow>> {
        let session = self.session_manager()?.get_session(session_id).await?;
        let summary = agena::session::cost::summarize(&session.messages);
        let mut rows = vec![
            InspectorRow {
                label: "summary".to_string(),
                detail: summary.one_line(),
            },
            InspectorRow {
                label: "turns".to_string(),
                detail: summary.turns.to_string(),
            },
            InspectorRow {
                label: "input_tokens".to_string(),
                detail: summary.input_tokens.to_string(),
            },
            InspectorRow {
                label: "output_tokens".to_string(),
                detail: summary.output_tokens.to_string(),
            },
            InspectorRow {
                label: "reasoning_tokens".to_string(),
                detail: summary.reasoning_tokens.to_string(),
            },
            InspectorRow {
                label: "cache_write_tokens".to_string(),
                detail: summary.cache_write_tokens.to_string(),
            },
            InspectorRow {
                label: "cache_read_tokens".to_string(),
                detail: summary.cache_read_tokens.to_string(),
            },
            InspectorRow {
                label: "total_cost_usd".to_string(),
                detail: format!("{:.4}", summary.total_cost_usd),
            },
        ];
        rows.extend(summary.by_model.into_iter().map(|entry| InspectorRow {
            label: format!("{}/{}", entry.provider_id, entry.model_id),
            detail: format!(
                "turns={} in={} out={} reasoning={} cost=${:.4}",
                entry.turns,
                entry.input_tokens,
                entry.output_tokens,
                entry.reasoning_tokens,
                entry.total_cost_usd
            ),
        }));
        Ok(rows)
    }

    pub async fn list_permission_rules(&self) -> Result<Vec<PermissionRuleResource>> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::ListPermissionRules(ListPermissionRulesParams {
                cursor: None,
                limit: Some(200),
                search: None,
            }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::PermissionRules(page) => {
                let mut rules = page.items;
                rules.sort_by(|left, right| left.action_key.cmp(&right.action_key));
                Ok(rules)
            }
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to list permission rules")
    }

    pub async fn create_permission_rule(
        &self,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(&self.app_state, ApiCommand::UpsertPermissionRule(params))
            .await
            .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to create permission rule")
    }

    pub async fn replace_permission_rule(
        &self,
        rule_id: i64,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ReplacePermissionRule(ReplacePermissionRuleParams {
                rule_id,
                rule: params,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to replace permission rule")
    }

    pub async fn revoke_permission_rule(&self, rule_id: i64) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::RevokePermissionRule(agena_api::commands::RevokePermissionRuleParams {
                rule_id,
                reason: None,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to revoke permission rule")
    }

    pub fn config_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let resolution = snapshot.config_resolution();
        let mut rows = vec![
            InspectorRow {
                label: "config_path".to_string(),
                detail: resolution.meta.config_path.display().to_string(),
            },
            InspectorRow {
                label: "config_found".to_string(),
                detail: resolution.meta.config_found.to_string(),
            },
            InspectorRow {
                label: "provider_count".to_string(),
                detail: resolution.config.providers.len().to_string(),
            },
            InspectorRow {
                label: "plugin_entries".to_string(),
                detail: resolution
                    .config
                    .plugins
                    .list
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | "),
            },
            InspectorRow {
                label: "mcp_servers".to_string(),
                detail: resolution.config.mcp.servers.len().to_string(),
            },
            InspectorRow {
                label: "lsp_servers".to_string(),
                detail: resolution.config.lsp.servers.len().to_string(),
            },
        ];
        rows.retain(|row| !row.detail.is_empty());
        rows
    }

    pub fn worktree_inspector_rows(&self) -> Vec<InspectorRow> {
        let Some(manager) = self.runtime.session_manager() else {
            return vec![InspectorRow {
                label: "session_runtime".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let executor = manager.tool_executor();
        let Some(registry) = executor.worktree_registry() else {
            return vec![InspectorRow {
                label: "worktree_registry".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let active = tool::worktree_list_active(registry);
        let managed = tool::worktree_list_managed(&self.workspace_root, registry);
        let mut rows = vec![
            InspectorRow {
                label: "active_sessions".to_string(),
                detail: active.len().to_string(),
            },
            InspectorRow {
                label: "managed_dirs".to_string(),
                detail: managed.len().to_string(),
            },
        ];
        rows.extend(active.into_iter().map(|entry| InspectorRow {
            label: format!("session #{}", entry.session_id),
            detail: format!(
                "{} | branch={} | created_here={}",
                entry.path.display(),
                entry.branch,
                entry.created_here
            ),
        }));
        rows.extend(managed.into_iter().map(|entry| {
            let session_id = entry
                .session_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string());
            let branch = entry
                .branch
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let stale = entry.is_stale();
            InspectorRow {
                label: entry.path.display().to_string(),
                detail: format!(
                    "session={} | branch={} | git_registered={} | stale={}",
                    session_id, branch, entry.registered_with_git, stale
                ),
            }
        }));
        rows
    }

    pub async fn git_inspector_rows(&self) -> Result<Vec<InspectorRow>> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        Ok(git_status_inspector_rows(status))
    }

    pub fn enter_worktree(
        &self,
        session_id: i64,
        name: Option<String>,
        path: Option<String>,
    ) -> Result<BundledToolOutput> {
        let manager = self.session_manager()?;
        manager
            .tool_executor()
            .execute_bundled_output_for_session(
                &BundledToolInput::EnterWorktree(EnterWorktreeToolInput { name, path }),
                session_id,
            )
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub fn exit_worktree(
        &self,
        session_id: i64,
        action: String,
        discard_changes: bool,
    ) -> Result<BundledToolOutput> {
        let manager = self.session_manager()?;
        manager
            .tool_executor()
            .execute_bundled_output_for_session(
                &BundledToolInput::ExitWorktree(ExitWorktreeToolInput {
                    action,
                    discard_changes,
                }),
                session_id,
            )
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub fn runtime_entry_exists(&self, name: &str) -> bool {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .lookup_entry(name)
            .is_some()
    }

    pub fn runtime_entry_prompt(&self, session_id: i64, name: &str, args: &str) -> Result<String> {
        let manager = self.session_manager()?;
        let invocation = ToolInvocation::new(
            name.to_string(),
            serde_json::from_value::<agena::message::StructuredObject>(json!({
                "args": if args.trim().is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(args.trim().to_string())
                }
            }))
            .map_err(|error| anyhow!(error))?,
        );
        let execution = manager
            .tool_executor()
            .execute_invocation_detailed(&invocation, session_id, -1)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(execution.view.output_text)
    }

    pub async fn create_commit(&self, message: String) -> Result<(String, String)> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }
        if status.staged_files == 0 {
            return Err(anyhow!("no staged changes to commit"));
        }

        let output = Command::new("git")
            .args(["commit", "-m", message.as_str()])
            .current_dir(&self.workspace_root)
            .output()
            .context("failed to execute git commit")?;
        if !output.status.success() {
            return Err(anyhow!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let commit = git_command_output(&self.workspace_root, ["rev-parse", "HEAD"])?;
        let summary = git_command_output(&self.workspace_root, ["log", "-1", "--pretty=%s"])?;
        Ok((commit, summary))
    }

    pub async fn create_pr(
        &self,
        title: String,
        body: Option<String>,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<String> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.gh_available {
            return Err(anyhow!("gh is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }

        let branch = head
            .clone()
            .or(status.branch.clone())
            .ok_or_else(|| anyhow!("could not determine current branch"))?;

        let mut command = Command::new("gh");
        command.arg("pr").arg("create").arg("--title").arg(title);
        command.arg("--body").arg(body.unwrap_or_default());
        if let Some(base) = base {
            command.arg("--base").arg(base);
        }
        command.arg("--head").arg(branch);
        command.current_dir(&self.workspace_root);

        let output = command.output().context("failed to execute gh pr create")?;
        if !output.status.success() {
            return Err(anyhow!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn resolve_workspace_resource(
        &self,
        create_if_missing: bool,
    ) -> Result<WorkspaceResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ResolveWorkspace(agena_api::commands::ResolveWorkspaceParams {
                path: self.workspace_root.to_string_lossy().to_string(),
                create_if_missing,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Workspace(workspace) => Ok(workspace),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to resolve workspace")
    }

    async fn git_status(&self) -> Result<GitStatusResource> {
        let workspace_root = self.runtime.workspace_root().to_path_buf();
        let git_available = command_available("git");
        let gh_available = command_available("gh");

        let Some(manager) = self.runtime.session_manager() else {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions: 0,
                worktree_managed_dirs: 0,
            });
        };

        let executor = manager.tool_executor();
        let (worktree_active_sessions, worktree_managed_dirs) = match executor.worktree_registry() {
            Some(registry) => (
                tool::worktree_list_active(registry).len() as u64,
                tool::worktree_list_managed(&workspace_root, registry).len() as u64,
            ),
            None => (0, 0),
        };

        if !git_available {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions,
                worktree_managed_dirs,
            });
        }

        let repo = git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]);
        if !repo {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions,
                worktree_managed_dirs,
            });
        }

        let branch = git_command_output(&workspace_root, ["branch", "--show-current"])?;
        let upstream = git_command_output(
            &workspace_root,
            [
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .ok()
        .and_then(|value| non_empty(Some(value.as_str())).map(ToOwned::to_owned));
        let ahead_behind = upstream.as_ref().and_then(|_| {
            git_command_output(
                &workspace_root,
                ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )
            .ok()
        });
        let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
        let status = git_command_output(&workspace_root, ["status", "--porcelain"])?;
        let (staged_files, unstaged_files, untracked_files, changed_files) =
            summarize_git_status(status.as_str());

        Ok(GitStatusResource {
            workspace_root: workspace_root.display().to_string(),
            git_available,
            repo,
            gh_available,
            branch: non_empty(Some(branch.as_str())).map(ToOwned::to_owned),
            upstream,
            ahead,
            behind,
            staged_files,
            unstaged_files,
            untracked_files,
            changed_files,
            clean: changed_files == 0,
            worktree_active_sessions,
            worktree_managed_dirs,
        })
    }

    pub fn resolve_model_target(&self, target: &str, model: Option<&str>) -> Result<ModelRef> {
        self.runtime
            .current_snapshot()
            .resolve_model_target(target, model)
            .context("failed to resolve model target")
    }

    async fn current_workspace_id(&self) -> Result<i64> {
        Ok(self
            .resolve_workspace_resource(true)
            .await
            .context("failed to resolve current workspace")?
            .id)
    }

    async fn list_sessions_query(&self, query: ListSessionsParams) -> Result<Vec<SessionResource>> {
        let mut cursor = query.cursor.clone();
        let limit = query.limit.unwrap_or(200);
        let mut items = Vec::new();

        loop {
            let page = match dispatch::dispatch_query(
                &self.app_state,
                Query::ListSessions(ListSessionsParams {
                    cursor: cursor.clone(),
                    limit: Some(limit),
                    workspace_id: query.workspace_id,
                    parent_id: query.parent_id,
                    roots: query.roots,
                    search: query.search.clone(),
                }),
            )
            .await
            .map_err(api_error)?
            {
                QueryResult::Sessions(page) => page,
                other => return Err(anyhow!("unexpected query result: {:?}", other)),
            };
            cursor = page.page.next_cursor.clone();
            items.extend(page.items);
            if !page.page.has_more || cursor.is_none() {
                break;
            }
        }

        Ok(items)
    }

    async fn resolve_session_root(&self, session_id: i64) -> Result<SessionResource> {
        let mut current = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        while let Some(parent_id) = current.parent_id {
            current = self
                .get_session(parent_id)
                .await?
                .ok_or_else(|| anyhow!("session not found: {parent_id}"))?;
        }
        Ok(current)
    }

    async fn resolve_session_run_options(
        &self,
        session_id: i64,
        request: RunOptions,
    ) -> Result<agena::session::SessionRunOptions> {
        let RunOptions {
            model,
            variant,
            agent_profile,
            system,
            temperature,
            max_output_tokens,
            max_turn_loops,
        } = request;

        let mut options = self
            .session_manager()?
            .resolve_scheduled_run_options(session_id)
            .await
            .context("failed to resolve session run options")?;

        if let Some(model) = model {
            options.model = model;
        }
        if let Some(variant) = variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let variants = self
                .runtime
                .current_snapshot()
                .provider_registry()
                .model_variants(&options.model)
                .context("failed to resolve selected model variants")?;
            let definition = variants
                .get(variant)
                .ok_or_else(|| anyhow!("model {} has no variant {variant}", options.model))?;
            options.variant = Some(variant.to_string());
            options.thinking = definition.thinking.clone();
        }

        if let Some(system) = system {
            options.system = Some(system);
        }
        if let Some(temperature) = temperature {
            options.temperature = Some(temperature);
        }
        if let Some(max_output_tokens) = max_output_tokens {
            options.max_output_tokens = Some(max_output_tokens);
        }
        if let Some(agent_profile) = agent_profile {
            options.agent_profile = Some(agent_profile);
        }
        if let Some(max_turn_loops) = max_turn_loops {
            options.max_turn_loops = Some(max_turn_loops);
        }

        Ok(options)
    }

    fn session_manager(&self) -> Result<Arc<agena::session::SessionManager>> {
        self.runtime
            .session_manager()
            .ok_or_else(|| anyhow!("session runtime is not available"))
    }

    fn memory_store(&self) -> MemoryStore {
        MemoryStore::for_workspace(&self.workspace_root)
    }
}

fn build_file_index(workspace_root: &Path) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(workspace_root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .follow_links(false)
        .parents(true)
        .require_git(false);

    let mut files = builder
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(workspace_root)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<Vec<_>>();

    files.sort();
    files
}

fn file_search_score(path: &Path, query_lower: &str) -> Option<(u8, usize, usize)> {
    if query_lower.is_empty() {
        return Some((4, path.components().count(), path.as_os_str().len()));
    }

    let path_text = path.to_string_lossy();
    let path_lower = path_text.to_lowercase();
    let filename_lower = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if filename_lower == query_lower {
        return Some((0, filename_lower.len(), path_lower.len()));
    }
    if filename_lower.starts_with(query_lower) {
        return Some((1, filename_lower.len(), path_lower.len()));
    }
    if let Some(index) = filename_lower.find(query_lower) {
        return Some((2, index, path_lower.len()));
    }

    path_lower
        .find(query_lower)
        .map(|index| (3, index, path_lower.len()))
}

fn direct_path_candidate(workspace_root: &Path, query: &str) -> Option<PathBuf> {
    if query.is_empty() {
        return None;
    }

    let typed = Path::new(query);
    let resolved = if typed.is_absolute() {
        typed.to_path_buf()
    } else {
        workspace_root.join(typed)
    };
    if !resolved.is_file() {
        return None;
    }

    resolved
        .strip_prefix(workspace_root)
        .map(Path::to_path_buf)
        .ok()
        .or(Some(resolved))
}

fn api_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow!(error.to_string())
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_success<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn parse_ahead_behind(value: Option<&str>) -> (Option<u64>, Option<u64>) {
    let Some(value) = value else {
        return (None, None);
    };
    let mut parts = value.split_whitespace();
    let behind = parts.next().and_then(|part| part.parse::<u64>().ok());
    let ahead = parts.next().and_then(|part| part.parse::<u64>().ok());
    (ahead, behind)
}

fn summarize_git_status(status: &str) -> (u64, u64, u64, u64) {
    let mut staged = 0_u64;
    let mut unstaged = 0_u64;
    let mut untracked = 0_u64;
    let mut changed = 0_u64;

    for line in status.lines().filter(|line| !line.is_empty()) {
        changed += 1;
        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        if x == b'?' && y == b'?' {
            untracked += 1;
            continue;
        }
        if x != b' ' {
            staged += 1;
        }
        if y != b' ' {
            unstaged += 1;
        }
    }

    (staged, unstaged, untracked, changed)
}

fn git_command_output<const N: usize>(workspace_root: &Path, args: [&str; N]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .context("failed to execute git command")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_status_inspector_rows(status: GitStatusResource) -> Vec<InspectorRow> {
    let mut rows = vec![
        InspectorRow {
            label: "workspace_root".to_string(),
            detail: status.workspace_root,
        },
        InspectorRow {
            label: "git_available".to_string(),
            detail: status.git_available.to_string(),
        },
        InspectorRow {
            label: "repo".to_string(),
            detail: status.repo.to_string(),
        },
        InspectorRow {
            label: "gh_available".to_string(),
            detail: status.gh_available.to_string(),
        },
        InspectorRow {
            label: "branch".to_string(),
            detail: status.branch.unwrap_or_else(|| "unknown".to_string()),
        },
        InspectorRow {
            label: "upstream".to_string(),
            detail: status.upstream.unwrap_or_else(|| "none".to_string()),
        },
        InspectorRow {
            label: "ahead".to_string(),
            detail: status.ahead.unwrap_or_default().to_string(),
        },
        InspectorRow {
            label: "behind".to_string(),
            detail: status.behind.unwrap_or_default().to_string(),
        },
        InspectorRow {
            label: "staged_files".to_string(),
            detail: status.staged_files.to_string(),
        },
        InspectorRow {
            label: "unstaged_files".to_string(),
            detail: status.unstaged_files.to_string(),
        },
        InspectorRow {
            label: "untracked_files".to_string(),
            detail: status.untracked_files.to_string(),
        },
        InspectorRow {
            label: "changed_files".to_string(),
            detail: status.changed_files.to_string(),
        },
        InspectorRow {
            label: "clean".to_string(),
            detail: status.clean.to_string(),
        },
        InspectorRow {
            label: "worktree_active_sessions".to_string(),
            detail: status.worktree_active_sessions.to_string(),
        },
        InspectorRow {
            label: "worktree_managed_dirs".to_string(),
            detail: status.worktree_managed_dirs.to_string(),
        },
    ];
    rows.retain(|row| !row.detail.is_empty());
    rows
}

fn detect_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    match imagesize::blob_size(bytes) {
        Ok(size) => (
            u32::try_from(size.width).ok(),
            u32::try_from(size.height).ok(),
        ),
        Err(_) => (None, None),
    }
}

fn detect_mime(path: &Path, bytes: &[u8]) -> String {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png".to_string();
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return "image/jpeg".to_string();
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if bytes.starts_with(b"BM") {
        return "image/bmp".to_string();
    }
    if bytes.starts_with(b"%PDF-") {
        return "application/pdf".to_string();
    }
    if std::str::from_utf8(bytes).is_ok() {
        return MimeGuess::from_path(path)
            .first_raw()
            .filter(|mime| {
                mime.starts_with("text/")
                    || matches!(
                        *mime,
                        "application/json"
                            | "application/xml"
                            | "application/yaml"
                            | "application/x-yaml"
                            | "application/javascript"
                    )
            })
            .map(str::to_owned)
            .unwrap_or_else(|| "text/plain".to_string());
    }

    MimeGuess::from_path(path)
        .first_raw()
        .map(str::to_owned)
        .unwrap_or_else(|| "application/octet-stream".to_string())
}
