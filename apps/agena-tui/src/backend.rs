use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena::{
    event::{DomainEvent, EventKind},
    message::{AttachmentItem, AttachmentKind, AttachmentSource, PartContent, UserInputReply},
    model::ModelRef,
    permission::{PermissionReply, PermissionReplyKind},
    provider::ProviderModel,
    runtime::AgenaRuntime,
    session::{
        SessionContinueRequest, SessionPermissionReplyRequest, SessionRewindRequest,
        SessionUserInputReplyRequest, SessionUserTurnRequest,
    },
};
use agena::event::{EventFilter, Scope, bus::SubscriptionItem};
use agena_http_api::{
    ApiError, ApiService, MessageListQuery, MessageResource, PaginatedResponse, PartLoadMode,
    ProviderSummaryResource, SessionCreateRequest, SessionEventListQuery, SessionExecutionResource,
    SessionListQuery, SessionReplaceRequest, SessionResource, SessionRunOptionsRequest,
    WorkspaceResolveRequest,
};
use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ignore::WalkBuilder;
use mime_guess::MimeGuess;
use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;

const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SessionRefresh {
    pub latest_event_seq: Option<i64>,
    pub event_count: usize,
    pub execution: Option<SessionExecutionResource>,
    pub latest_messages: Option<PaginatedResponse<MessageResource>>,
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
    api: ApiService,
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
            runtime,
            api: ApiService::new(db),
            workspace_root,
            file_index: Arc::new(OnceLock::new()),
        }
    }

    pub async fn list_workspace_sessions(&self, roots_only: bool) -> Result<Vec<SessionResource>> {
        let workspace_id = self.current_workspace_id().await?;
        self.list_sessions_query(SessionListQuery {
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
            .api
            .resolve_workspace(WorkspaceResolveRequest {
                path: self.workspace_root.to_string_lossy().to_string(),
                create_if_missing: true,
            })
            .await
            .map_err(api_error)
            .context("failed to resolve workspace for agena-tui")?;

        self.api
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title,
                parent_id,
            })
            .await
            .map_err(api_error)
            .context("failed to create session")
    }

    pub async fn rename_session(&self, session_id: i64, title: String) -> Result<SessionResource> {
        let existing = self
            .get_session(session_id)
            .await
            .context("failed to load session before rename")?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;

        self.api
            .replace_session(
                session_id,
                SessionReplaceRequest {
                    title,
                    parent_id: existing.parent_id,
                },
            )
            .await
            .map_err(api_error)
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
        self.list_sessions_query(SessionListQuery {
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
        self.api
            .get_session(session_id)
            .await
            .map_err(api_error)
            .context("failed to fetch session")
    }

    pub async fn list_session_subtree(&self, session_id: i64) -> Result<Vec<SessionResource>> {
        let root = self.resolve_session_root(session_id).await?;
        let mut items = vec![root.clone()];
        let mut seen = HashSet::from([root.id]);
        let mut stack = vec![root.id];

        while let Some(parent_id) = stack.pop() {
            let children = self
                .list_sessions_query(SessionListQuery {
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
        self.api
            .list_session_events(
                manager.as_ref(),
                session_id,
                SessionEventListQuery {
                    cursor: None,
                    limit: Some(limit),
                },
            )
            .await
            .map_err(api_error)
            .map(|page| page.items)
            .context("failed to list session events")
    }

    pub async fn list_all_messages(&self, session_id: i64) -> Result<Vec<MessageResource>> {
        let mut cursor = None;
        let mut messages = Vec::new();

        let manager = self.runtime.session_manager().ok_or_else(|| {
            anyhow::anyhow!("session runtime is not available")
        })?;
        loop {
            let page = self
                .api
                .list_messages(
                    manager.as_ref(),
                    session_id,
                    MessageListQuery {
                        cursor: cursor.clone(),
                        limit: Some(200),
                        parts: PartLoadMode::Full,
                    },
                )
                .await
                .map_err(api_error)
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
        let session = self
            .session_manager()?
            .get_session(session_id)
            .await
            .context("failed to load session state")?;
        self.api
            .session_execution_resource(self.session_manager()?.as_ref(), &session)
            .await
            .map_err(api_error)
            .context("failed to materialize session execution resource")
    }

    pub async fn list_messages(
        &self,
        session_id: i64,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<MessageResource>> {
        let manager = self.runtime.session_manager().ok_or_else(|| {
            anyhow::anyhow!("session runtime is not available")
        })?;
        self.api
            .list_messages(
                manager.as_ref(),
                session_id,
                MessageListQuery {
                    cursor,
                    limit: Some(limit),
                    parts: PartLoadMode::Full,
                },
            )
            .await
            .map_err(api_error)
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
        let latest_event_seq = self
            .api
            .latest_session_event_seq(manager.as_ref(), session_id)
            .await
            .map_err(api_error)
            .context("failed to fetch latest session event sequence")?;
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
            Some(after) => self
                .api
                .list_session_events_after(manager.as_ref(), session_id, after, Some(256))
                .await
                .map_err(api_error)
                .context("failed to fetch incremental session events")?
                .len(),
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
                let live = LiveEvent {
                    triggers_refresh,
                };
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
        request: SessionRunOptionsRequest,
    ) -> Result<SessionExecutionResource> {
        let options = self.resolve_run_options(session_id, request).await?;
        let session = self
            .session_manager()?
            .submit_user_turn(SessionUserTurnRequest {
                session_id,
                options,
                parts,
            })
            .await
            .context("failed to submit user turn")?;

        self.api
            .session_execution_resource(self.session_manager()?.as_ref(), &session)
            .await
            .map_err(api_error)
            .context("failed to materialize updated session state")
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
        request: SessionRunOptionsRequest,
    ) -> Result<SessionExecutionResource> {
        let options = self.resolve_run_options(session_id, request).await?;
        let session = self
            .session_manager()?
            .continue_session(SessionContinueRequest {
                session_id,
                options,
            })
            .await
            .context("failed to continue session")?;

        self.api
            .session_execution_resource(self.session_manager()?.as_ref(), &session)
            .await
            .map_err(api_error)
            .context("failed to materialize continued session state")
    }

    pub async fn reply_permission_with_options(
        &self,
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        request: SessionRunOptionsRequest,
    ) -> Result<SessionExecutionResource> {
        let options = self.resolve_run_options(session_id, request).await?;
        let session = self
            .session_manager()?
            .reply_permission(SessionPermissionReplyRequest {
                session_id,
                options,
                reply: PermissionReply {
                    request_id,
                    kind,
                    reason: None,
                    scope: None,
                },
            })
            .await
            .context("failed to reply to permission request")?;

        self.api
            .session_execution_resource(self.session_manager()?.as_ref(), &session)
            .await
            .map_err(api_error)
            .context("failed to materialize permission reply result")
    }

    pub async fn reply_user_input_with_options(
        &self,
        session_id: i64,
        reply: UserInputReply,
        request: SessionRunOptionsRequest,
    ) -> Result<SessionExecutionResource> {
        let options = self.resolve_run_options(session_id, request).await?;
        let session = self
            .session_manager()?
            .reply_user_input(SessionUserInputReplyRequest {
                session_id,
                options,
                reply,
            })
            .await
            .context("failed to submit user input reply")?;

        self.api
            .session_execution_resource(self.session_manager()?.as_ref(), &session)
            .await
            .map_err(api_error)
            .context("failed to materialize user input reply result")
    }

    pub async fn rewind_session_to_message(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<SessionExecutionResource> {
        let session = self
            .session_manager()?
            .rewind_session(SessionRewindRequest {
                session_id,
                message_id,
            })
            .await
            .context("failed to rewind session to message")?;

        self.api
            .session_execution_resource(self.session_manager()?.as_ref(), &session)
            .await
            .map_err(api_error)
            .context("failed to materialize rewound session state")
    }

    async fn resolve_run_options(
        &self,
        session_id: i64,
        request: SessionRunOptionsRequest,
    ) -> Result<agena::session::SessionRunOptions> {
        let snapshot = self.runtime.current_snapshot();
        let manager = self.runtime.session_manager().ok_or_else(|| {
            anyhow::anyhow!("session runtime is not available")
        })?;
        self.api
            .resolve_run_options(
                snapshot.provider_registry().as_ref(),
                manager.as_ref(),
                session_id,
                request,
            )
            .await
            .map_err(api_error)
            .context("failed to resolve run options")
    }

    pub fn resolve_model_target(&self, target: &str, model: Option<&str>) -> Result<ModelRef> {
        self.runtime
            .current_snapshot()
            .resolve_model_target(target, model)
            .context("failed to resolve model target")
    }

    async fn current_workspace_id(&self) -> Result<i64> {
        let workspace = self
            .api
            .resolve_workspace(WorkspaceResolveRequest {
                path: self.workspace_root.to_string_lossy().to_string(),
                create_if_missing: true,
            })
            .await
            .map_err(api_error)
            .context("failed to resolve current workspace")?;
        Ok(workspace.id)
    }

    async fn list_sessions_query(&self, query: SessionListQuery) -> Result<Vec<SessionResource>> {
        let mut cursor = query.cursor.clone();
        let limit = query.limit.unwrap_or(200);
        let mut items = Vec::new();

        loop {
            let page = self
                .api
                .list_sessions(SessionListQuery {
                    cursor: cursor.clone(),
                    limit: Some(limit),
                    workspace_id: query.workspace_id,
                    parent_id: query.parent_id,
                    roots: query.roots,
                    search: query.search.clone(),
                })
                .await
                .map_err(api_error)
                .context("failed to list session page")?;
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

    fn session_manager(&self) -> Result<Arc<agena::session::SessionManager>> {
        self.runtime
            .session_manager()
            .ok_or_else(|| anyhow!("session runtime is not available"))
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

fn api_error(error: ApiError) -> anyhow::Error {
    anyhow!("{error:?}")
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
