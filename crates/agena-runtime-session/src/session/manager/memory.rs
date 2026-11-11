//! Bounded, source-backed memory maintenance after a conversation becomes idle.
//!
//! The kernel remains the authority for execution and transcript history. These
//! files are disposable semantic projections: a session-only checkpoint and a
//! historical episode. They never become user instructions or permission grants.
use super::*;
use agena_domain::{AccessKind, ContentKind, PermissionDecision};
use agena_storage::{
    MemoryKind, MemoryOrigin, MemoryRecord, MemoryScope, MemorySource, MemoryStatus, MemoryStore,
    MemoryWriteOptions, MemoryWriteSnapshot, NewMemory,
};
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::time::Instant;

const PROMPT: &str = include_str!("memory_prompt.txt");
const CONTEXT_BYTES: usize = 32 * 1024;
const CHECKPOINT_BYTES: usize = 64 * 1024;
const MAX_EVIDENCE: usize = 48;
const MAX_ITEM_BYTES: usize = 2048;
const MAX_SUMMARY_BYTES: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct MemoryWork {
    running: bool,
    pending: bool,
    requested_at: Option<Instant>,
    not_before: Option<Instant>,
    retries: u8,
}

#[derive(Debug, PartialEq, Eq)]
enum MaintenanceOutcome {
    Complete,
    Defer(Duration),
    Retry,
}

struct Worker {
    dispatch: Arc<SessionDispatch>,
    active: bool,
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.active {
            self.dispatch
                .memory
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .running = false;
        }
    }
}

// The owning plugin validates these settings. The runtime repeats bounds at the
// effect boundary so malformed configuration cannot expand background budgets.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct MaintenanceConfig {
    enabled: bool,
    idle_seconds: u64,
    min_user_messages: u64,
    cooldown_seconds: u64,
    max_output_tokens: u32,
}
impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_seconds: 30,
            min_user_messages: 1,
            cooldown_seconds: 300,
            max_output_tokens: 1536,
        }
    }
}
impl MaintenanceConfig {
    fn for_state(state: &ExecutionState) -> Option<Self> {
        let plugin = state
            .executor
            .plugin_manager()
            .plugins()
            .iter()
            .find(|plugin| {
                plugin.manifest.namespace == "agena"
                    && plugin.manifest.name == "memory"
                    && matches!(
                        plugin.configured_plugin.package,
                        agena_plugin_host::PluginPackage::Static {}
                    )
            })?;
        let settings = &plugin.configured_plugin.settings;
        let mut config: Self = match settings.get("maintenance") {
            Some(value) => serde_json::from_value(value.clone()).ok()?,
            None => Self::default(),
        };
        if !config.enabled {
            return None;
        }
        config.idle_seconds = config.idle_seconds.clamp(5, 3600);
        config.min_user_messages = config.min_user_messages.clamp(1, 100);
        config.cooldown_seconds = config.cooldown_seconds.min(86_400);
        config.max_output_tokens = config.max_output_tokens.clamp(256, 8192);
        Some(config)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceKind {
    User,
    Assistant,
    ToolResult,
    Checkpoint,
    MemoryForget,
}
#[derive(Clone, Debug, Serialize)]
struct Evidence {
    entry_id: i64,
    source_session_id: i64,
    revision: i64,
    kind: EvidenceKind,
    text: String,
}
struct EpisodeContext {
    evidence: Vec<Evidence>,
    watermark: EntryId,
    membership: MembershipGeneration,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodeDraft {
    title: String,
    #[serde(default)]
    decisions: Vec<MemoryItem>,
    #[serde(default)]
    outcomes: Vec<MemoryItem>,
    #[serde(default)]
    open_work: Vec<MemoryItem>,
    #[serde(default)]
    lessons: Vec<MemoryItem>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryItem {
    text: String,
    evidence: Vec<i64>,
}

impl SessionManager {
    /// Stable database identity for session-scoped memory and source pointers.
    pub fn memory_instance_id(&self) -> &str {
        &self.kernel_runtime.instance_id
    }

    fn memory_record_name(&self, kind: &str, session: SessionId) -> String {
        format!(
            "{kind}-{}-session-{}",
            self.memory_instance_id(),
            session.get()
        )
    }

    /// Installed by runtime composition, not by a model or plugin callback.
    /// Missing preference authority disables automatic persistent learning.
    pub fn install_memory_preferences(
        &self,
        repository: Arc<dyn agena_assistant::CompanionRepository>,
    ) {
        let _ = self.memory_preferences.set(repository);
    }

    pub(super) fn schedule_memory(&self, session: SessionId) {
        if self.shutdown.is_cancelled() || self.memory_preferences.get().is_none() {
            return;
        }
        let dispatch = self.session_dispatch(session);
        let mut work = dispatch.memory.lock().unwrap_or_else(|p| p.into_inner());
        work.pending = true;
        work.requested_at = Some(Instant::now());
        work.retries = 0;
        if work.running {
            drop(work);
            dispatch.memory_changed.notify_one();
            return;
        }
        work.running = true;
        drop(work);
        let manager = self.clone();
        // Shutdown cancels debounce and model work; the tracker also includes
        // the final bounded storage commit in the runtime's drain boundary.
        self.control_tasks.spawn(async move {
            let mut worker = Worker { dispatch, active: true };
            loop {
                let Ok(state) = manager.state_for_session(session).await else {
                    return;
                };
                let Some(config) = MaintenanceConfig::for_state(&state) else {
                    return;
                };
                let deadline = {
                    let work = worker.dispatch.memory.lock().unwrap_or_else(|p| p.into_inner());
                    let idle = work.requested_at.unwrap_or_else(Instant::now)
                        + Duration::from_secs(config.idle_seconds);
                    work.not_before.map_or(idle, |retry| idle.max(retry))
                };
                tokio::select! {
                    biased;
                    _ = manager.shutdown.cancelled() => return,
                    _ = worker.dispatch.memory_changed.notified() => continue,
                    _ = tokio::time::sleep_until(deadline) => {},
                }
                {
                    let mut work = worker.dispatch.memory.lock().unwrap_or_else(|p| p.into_inner());
                    if work.requested_at.is_some_and(|at| at + Duration::from_secs(config.idle_seconds) > Instant::now()) {
                        continue;
                    }
                    if work.not_before.is_some_and(|at| at > Instant::now()) {
                        continue;
                    }
                    work.pending = false;
                    work.not_before = None;
                }
                let result = std::panic::AssertUnwindSafe(manager.maintain_memory(session))
                    .catch_unwind()
                    .await;
                let mut work = worker.dispatch.memory.lock().unwrap_or_else(|p| p.into_inner());
                let delay = match result {
                    Ok(Ok(MaintenanceOutcome::Complete)) => None,
                    Ok(Ok(MaintenanceOutcome::Defer(delay))) => Some(delay),
                    Ok(Ok(MaintenanceOutcome::Retry)) if work.retries < 2 => {
                        work.retries += 1;
                        Some(Duration::from_secs(5))
                    },
                    Ok(Ok(MaintenanceOutcome::Retry)) => {
                        tracing::debug!(session_id = session.get(), "memory maintenance retry budget exhausted; kernel history retained");
                        None
                    },
                    Ok(Err(error)) => {
                        tracing::debug!(session_id = session.get(), %error, "memory maintenance stopped; kernel history retained");
                        None
                    },
                    Err(_) => {
                        tracing::warn!(session_id = session.get(), "memory maintenance worker panicked");
                        None
                    },
                };
                if let Some(delay) = delay {
                    work.pending = true;
                    work.not_before = Some(Instant::now() + delay);
                }
                if !work.pending || manager.shutdown.is_cancelled() {
                    work.running = false;
                    worker.active = false;
                    return;
                }
            }
        });
    }

    async fn memory_target(
        &self,
        session: SessionId,
        state: &ExecutionState,
    ) -> Result<Option<(MemoryStore, ToolExecutor)>, AppError> {
        if self.shutdown.is_cancelled() || MaintenanceConfig::for_state(state).is_none() {
            return Ok(None);
        }
        let Some(preferences) = self.memory_preferences.get() else {
            return Ok(None);
        };
        let control = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        // A fork or worker cannot quietly promote its inherited transcript into
        // a foreground user's workspace memory.
        if control.configuration.relation_kind.is_subagent()
            || control.configuration.conversation.is_some()
            || control
                .configuration
                .channel_authority
                .as_ref()
                .is_some_and(|authority| !authority.grant.private_memory)
            || control
                .configuration
                .capability_denied_tool_names
                .iter()
                .any(|name| matches!(name.as_str(), "memory.write" | "agena.memory.write"))
        {
            return Ok(None);
        }
        let workspace = control.metadata.workspace_id;
        drop(control);
        if !preferences
            .profile_scoped(workspace)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .preferences
            .memory_enabled
        {
            return Ok(None);
        }
        let context = self.execution_context_with_state(session, state).await?;
        // Fork/rewind membership retains its inherited prefix permanently even
        // after it leaves the bounded recent window. Do not relabel that
        // history with this session ID and evade a parent source suppression.
        // An independent Child (or an empty rewind) has only its own entries.
        let first = self
            .kernel()
            .reader
            .history_after(session, None, 1)
            .await
            .map_err(kernel_error)?;
        if first
            .entries
            .first()
            .is_some_and(|entry| entry.origin_session_id != session)
        {
            tracing::debug!(
                session_id = session.get(),
                "background memory skipped: inherited history requires original source attribution"
            );
            return Ok(None);
        }
        drop(first);
        let root = match &context.channel_authority {
            Some(authority) => authority.memory_workspace_root.clone(),
            None if workspace.is_none() => self.global_memory_root().to_path_buf(),
            None => match context.effective_workspace_root.clone() {
                Some(root) => root,
                None => return Ok(None),
            },
        };
        let executor = state.executor.for_session_context_async(&context).await;
        // Background persistence is subject to the same tool capability,
        // model profile, channel allowlist and remembered rules as memory.write.
        // This is a read-only policy preflight, never a tool invocation.
        let checks = match executor
            .collect_permission_checks_for_invocation_in_session(
                &agena_domain::ToolInvocation::new("agena.memory.write", Default::default()),
                Some(session.get()),
            )
            .await
        {
            Ok(checks) => checks,
            Err(error) => {
                tracing::debug!(session_id = session.get(), %error, "background memory skipped: write capability unavailable");
                return Ok(None);
            }
        };
        for check in checks {
            if !matches!(
                self.query_host_permission(Some(session.get()), check)
                    .await?,
                PermissionDecision::Allow
            ) {
                tracing::debug!(
                    session_id = session.get(),
                    "background memory skipped: write tool needs authorization"
                );
                return Ok(None);
            }
        }
        Ok(Some((MemoryStore::for_workspace(&root), executor)))
    }

    async fn memory_paths_allowed(
        &self,
        session: SessionId,
        executor: &ToolExecutor,
        store: &MemoryStore,
        name: &str,
    ) -> Result<bool, AppError> {
        let paths = [
            store.dir().to_path_buf(),
            store.dir().join(format!("{name}.md")),
            store.dir().join("MEMORY.md"),
        ]
        .into_iter()
        .chain(store.internal_paths())
        .collect::<Vec<_>>();
        self.memory_effect_paths_allowed(session, executor, &paths)
            .await
    }

    async fn memory_effect_paths_allowed(
        &self,
        session: SessionId,
        executor: &ToolExecutor,
        paths: &[std::path::PathBuf],
    ) -> Result<bool, AppError> {
        for path in paths {
            for access in [AccessKind::Read, AccessKind::Write] {
                let check = executor.path_permission_check(access, &path.to_string_lossy());
                if !matches!(
                    self.query_host_permission(Some(session.get()), check)
                        .await?,
                    PermissionDecision::Allow
                ) {
                    tracing::debug!(session_id = session.get(), path = %path.display(), "background memory skipped: path needs authorization");
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    async fn prepare_memory_store(
        &self,
        session: SessionId,
        state: &Arc<ExecutionState>,
        store: &MemoryStore,
        executor: &ToolExecutor,
    ) -> Result<bool, AppError> {
        // The journal and lock themselves require authorization before reading
        // the pending transaction. Its actual targets are approved separately.
        let internal = [store.dir().to_path_buf(), store.dir().join("MEMORY.md")]
            .into_iter()
            .chain(store.internal_paths())
            .collect::<Vec<_>>();
        if !self
            .memory_effect_paths_allowed(session, executor, &internal)
            .await?
        {
            return Ok(false);
        }
        let pending_store = store.clone();
        let pending = tokio::task::spawn_blocking(move || pending_store.pending_recovery_paths())
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if pending.is_empty() {
            return Ok(true);
        }
        if !Arc::ptr_eq(state, &self.state_for_session(session).await?) {
            return Ok(false);
        }
        let Some((current, executor)) = self.memory_target(session, state).await? else {
            return Ok(false);
        };
        if current.dir() != store.dir()
            || !self
                .memory_effect_paths_allowed(session, &executor, &pending)
                .await?
        {
            return Ok(false);
        }
        let allowed = pending
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        let recovery_store = store.clone();
        tokio::task::spawn_blocking(move || {
            recovery_store.recover_pending(|path| allowed.contains(path))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(true)
    }

    async fn memory_source_is_suppressed(
        &self,
        session: SessionId,
        store: &MemoryStore,
    ) -> Result<bool, AppError> {
        let source = MemorySource {
            session_id: session.get(),
            instance_id: self.memory_instance_id().to_owned(),
            ..Default::default()
        };
        let store = store.clone();
        tokio::task::spawn_blocking(move || store.source_is_suppressed(&source))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .map_err(|e| AppError::Internal(e.to_string()))
    }

    async fn maintain_memory(&self, session: SessionId) -> Result<MaintenanceOutcome, AppError> {
        let state = self.state_for_session(session).await?;
        let Some(config) = MaintenanceConfig::for_state(&state) else {
            return Ok(MaintenanceOutcome::Complete);
        };
        let Some((store, executor)) = self.memory_target(session, &state).await? else {
            return Ok(MaintenanceOutcome::Complete);
        };
        let control = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        if control.summary.running_executions > 0 || control.summary.queued_executions > 0 {
            return Ok(MaintenanceOutcome::Complete);
        }
        let messages = control.summary.user_message_count;
        let selection = control.metadata.selection.clone();
        drop(control);

        if !self
            .prepare_memory_store(session, &state, &store, &executor)
            .await?
            || self.memory_source_is_suppressed(session, &store).await?
        {
            tracing::debug!(
                session_id = session.get(),
                "memory maintenance skipped: protected store or suppressed source session"
            );
            return Ok(MaintenanceOutcome::Complete);
        }

        // Compaction has already paid for this semantic projection. Preserve it
        // for the same session without another model call or a global promotion.
        match self
            .persist_memory_checkpoint(session, &state, &store, &executor)
            .await?
        {
            MaintenanceOutcome::Complete => {}
            retry => return Ok(retry),
        }
        if messages < config.min_user_messages {
            return Ok(MaintenanceOutcome::Complete);
        }
        let name = self.memory_record_name("episode", session);
        if !self
            .memory_paths_allowed(session, &executor, &store, &name)
            .await?
        {
            return Ok(MaintenanceOutcome::Complete);
        }
        let (snapshot, previous) = read_snapshot(store.clone(), name.clone()).await?;
        if snapshot.is_tombstoned
            || previous
                .as_ref()
                .is_some_and(|record| !writable_derived(record))
        {
            return Ok(MaintenanceOutcome::Complete);
        }
        let Some(context) = self.memory_evidence(session, previous.as_ref()).await? else {
            return Ok(MaintenanceOutcome::Complete);
        };
        if previous.as_ref().is_some_and(|record| {
            record.metadata.source.as_ref().is_some_and(|source| {
                source.session_id == session.get()
                    && source
                        .entry_id
                        .is_some_and(|id| id >= context.watermark.get())
            })
        }) {
            return Ok(MaintenanceOutcome::Complete);
        }
        if context
            .evidence
            .iter()
            .any(|source| source.kind == EvidenceKind::MemoryForget)
        {
            tracing::debug!(
                session_id = session.get(),
                "episode extraction skipped after explicit memory forget"
            );
            return Ok(MaintenanceOutcome::Complete);
        }
        // One substantial tool-backed task is sufficient. A greeting without
        // observed work is not a useful episode, regardless of configured floor.
        if messages < 3
            && !context
                .evidence
                .iter()
                .any(|source| source.kind == EvidenceKind::ToolResult)
        {
            return Ok(MaintenanceOutcome::Complete);
        }
        if let Some(previous) = &previous {
            let cooldown_ms =
                i64::try_from(config.cooldown_seconds.saturating_mul(1000)).unwrap_or(i64::MAX);
            let elapsed_ms = Utc::now()
                .timestamp_millis()
                .saturating_sub(previous.metadata.updated_at_ms)
                .max(0);
            if elapsed_ms < cooldown_ms {
                return Ok(MaintenanceOutcome::Defer(Duration::from_millis(
                    (cooldown_ms - elapsed_ms) as u64,
                )));
            }
        }
        // Bounded retries keep the final turn eligible when helper capacity is
        // temporarily occupied, without accumulating queued model requests.
        let Ok(_permit) = self.auxiliary_tasks.clone().try_acquire_owned() else {
            return Ok(MaintenanceOutcome::Retry);
        };
        let prompt = serde_json::json!({
            "session_id": session.get(),
            "historical_episode": true,
            "previous_episode": previous.as_ref().map(|record| truncate(&record.body, 12 * 1024)),
            "evidence": context.evidence,
        })
        .to_string();
        let response = self
            .complete_auxiliary_with_state(
                &state,
                selection.as_ref(),
                PROMPT.into(),
                prompt,
                config.max_output_tokens,
            )
            .await?;
        let Some((description, body)) = parse_episode(&response.text, session, &context.evidence)?
        else {
            return Ok(MaintenanceOutcome::Complete);
        };
        if !self
            .memory_commit_allowed(session, &state, &store, &name)
            .await?
            || !self.memory_evidence_current(session, &context).await?
        {
            return Ok(MaintenanceOutcome::Complete);
        }
        let entry = NewMemory {
            name: name.clone(),
            description: description.clone(),
            kind: MemoryKind::Episode,
            body,
        };
        let options = MemoryWriteOptions {
            scope: Some(if store.dir().starts_with(self.global_memory_root()) {
                MemoryScope::Global
            } else {
                MemoryScope::Workspace
            }),
            source: Some(MemorySource {
                session_id: session.get(),
                instance_id: self.memory_instance_id().to_owned(),
                entry_id: Some(context.watermark.get()),
                task_id: Some(format!(
                    "instance:{}:session:{}",
                    self.memory_instance_id(),
                    session.get()
                )),
                ..Default::default()
            }),
            ..Default::default()
        };
        save_projection(store, entry, snapshot, options).await
    }

    async fn memory_commit_allowed(
        &self,
        session: SessionId,
        state: &Arc<ExecutionState>,
        store: &MemoryStore,
        name: &str,
    ) -> Result<bool, AppError> {
        if self.shutdown.is_cancelled()
            || !Arc::ptr_eq(state, &self.state_for_session(session).await?)
        {
            return Ok(false);
        }
        let Some((current, executor)) = self.memory_target(session, state).await? else {
            return Ok(false);
        };
        if current.dir() != store.dir() {
            return Ok(false);
        }
        Ok(self
            .memory_paths_allowed(session, &executor, store, name)
            .await?
            && !self.memory_source_is_suppressed(session, store).await?)
    }

    async fn persist_memory_checkpoint(
        &self,
        session: SessionId,
        state: &Arc<ExecutionState>,
        store: &MemoryStore,
        executor: &ToolExecutor,
    ) -> Result<MaintenanceOutcome, AppError> {
        let status = self
            .kernel()
            .reader
            .prompt_window_status(session)
            .await
            .map_err(kernel_error)?;
        let Some(checkpoint) = status.checkpoint_entry_id else {
            return Ok(MaintenanceOutcome::Complete);
        };
        drop(status);
        let name = self.memory_record_name("task", session);
        if !self
            .memory_paths_allowed(session, executor, store, &name)
            .await?
        {
            return Ok(MaintenanceOutcome::Complete);
        }
        let (snapshot, previous) = read_snapshot(store.clone(), name.clone()).await?;
        if snapshot.is_tombstoned
            || previous.as_ref().is_some_and(|record| {
                !writable_derived(record)
                    || record.metadata.source.as_ref().is_some_and(|source| {
                        source.session_id == session.get()
                            && source.entry_id == Some(checkpoint.get())
                    })
            })
        {
            return Ok(MaintenanceOutcome::Complete);
        }
        let entry = self
            .kernel()
            .reader
            .entry(session, checkpoint)
            .await
            .map_err(kernel_error)?;
        if entry.origin_session_id != session || !entry.compaction_committed {
            tracing::debug!(
                session_id = session.get(),
                "task projection skipped: checkpoint belongs to another source session"
            );
            return Ok(MaintenanceOutcome::Complete);
        }
        let EntryPayload::Compaction { summary, through } = &entry.payload else {
            return Ok(MaintenanceOutcome::Complete);
        };
        let (summary, through) = (*summary, *through);
        drop(entry);
        let text = self
            .kernel()
            .reader
            .text_range(session, summary, None, CHECKPOINT_BYTES)
            .await
            .map_err(kernel_error)?;
        // Native replay metadata lives on the round, never in this text stream.
        // An opaque-only checkpoint has no portable task state to project.
        if text.bytes.iter().all(u8::is_ascii_whitespace) {
            tracing::debug!(
                session_id = session.get(),
                "task projection skipped: no portable checkpoint text"
            );
            return Ok(MaintenanceOutcome::Complete);
        }
        let body = format!(
            "# Current task checkpoint\n\nDerived from committed session {session}, checkpoint entry {checkpoint}, covering history through entry {through}. The live kernel remains authoritative for execution status.\n\n{}{}\n",
            String::from_utf8_lossy(&text.bytes),
            if text.has_more {
                "\n\n[Checkpoint excerpt; read the original session entry for the remaining context.]"
            } else {
                ""
            },
            session = session.get(),
            checkpoint = checkpoint.get(),
            through = through.get(),
        );
        drop(text);
        let current = self
            .kernel()
            .reader
            .prompt_window_status(session)
            .await
            .map_err(kernel_error)?;
        if current.checkpoint_entry_id != Some(checkpoint) {
            return Ok(MaintenanceOutcome::Complete);
        }
        drop(current);
        if !self
            .memory_commit_allowed(session, state, store, &name)
            .await?
        {
            return Ok(MaintenanceOutcome::Complete);
        }
        let options = MemoryWriteOptions {
            scope: Some(MemoryScope::Session {
                session_id: session.get(),
                instance_id: self.memory_instance_id().to_owned(),
            }),
            source: Some(MemorySource {
                session_id: session.get(),
                instance_id: self.memory_instance_id().to_owned(),
                entry_id: Some(checkpoint.get()),
                task_id: Some(format!(
                    "instance:{}:session:{}",
                    self.memory_instance_id(),
                    session.get()
                )),
                ..Default::default()
            }),
            ..Default::default()
        };
        save_projection(
            store.clone(),
            NewMemory {
                name: name.clone(),
                description: "Current task checkpoint; read for this session's continuation".into(),
                kind: MemoryKind::Task,
                body,
            },
            snapshot,
            options,
        )
        .await
    }

    async fn memory_evidence(
        &self,
        session: SessionId,
        previous: Option<&MemoryRecord>,
    ) -> Result<Option<EpisodeContext>, AppError> {
        let page = self
            .kernel()
            .reader
            .message_history(session, None, 12, 24)
            .await
            .map_err(kernel_error)?;
        let membership = page.metadata.membership_generation;
        let Some(watermark) = page.entries.iter().map(|entry| entry.id).max() else {
            return Ok(None);
        };
        let entries = page.entries.clone();
        drop(page);
        // Reserve room to re-read the initial goal and older sources retained
        // by the previous episode. A short recent window is not the whole task.
        let mut remaining = CONTEXT_BYTES - 8 * 1024;
        let mut evidence = Vec::new();
        for entry in entries.iter().rev() {
            if remaining < 256 || evidence.len() >= MAX_EVIDENCE - 12 {
                break;
            }
            if let Some(item) = self
                .read_memory_evidence(session, entry, remaining.min(3072))
                .await?
            {
                remaining = remaining.saturating_sub(item.text.len());
                evidence.push(item);
            }
        }
        remaining += 8 * 1024;
        let first = self
            .kernel()
            .reader
            .history_after(session, None, 1)
            .await
            .map_err(kernel_error)?;
        let first = first.entries.first().cloned();
        let mut retained = previous
            .map(|record| previous_sources(&record.body, session))
            .unwrap_or_default();
        if let Some(first) = first {
            retained.insert(0, first.id.get());
        }
        for id in retained {
            if remaining < 256 || evidence.len() >= MAX_EVIDENCE {
                break;
            }
            if evidence.iter().any(|source| source.entry_id == id) {
                continue;
            }
            let Ok(id) = EntryId::try_from(id) else {
                continue;
            };
            let entry = match self.kernel().reader.entry(session, id).await {
                Ok(entry) => entry,
                // Removed or no-longer-visible sources cannot justify keeping
                // their old derived claims in a new episode.
                Err(KernelError::NotFound(_)) => continue,
                Err(error) => return Err(kernel_error(error)),
            };
            if let Some(item) = self
                .read_memory_evidence(session, &entry, remaining.min(2048))
                .await?
            {
                remaining = remaining.saturating_sub(item.text.len());
                evidence.push(item);
            }
        }
        evidence.sort_by_key(|source| source.entry_id);
        if evidence
            .iter()
            .any(|source| source.source_session_id != session.get())
        {
            tracing::debug!(
                session_id = session.get(),
                "episode extraction skipped: mixed source-session evidence"
            );
            return Ok(None);
        }
        if evidence.is_empty() {
            return Ok(None);
        }
        Ok(Some(EpisodeContext {
            evidence,
            watermark,
            membership,
        }))
    }

    async fn read_memory_evidence(
        &self,
        session: SessionId,
        entry: &EntryView,
        max: usize,
    ) -> Result<Option<Evidence>, AppError> {
        let item = match &entry.payload {
            EntryPayload::User { document, .. } => {
                Some((EvidenceKind::User, document_text(document, max)))
            }
            EntryPayload::Text { content_id } => {
                let range = self
                    .kernel()
                    .reader
                    .text_range(session, *content_id, None, max)
                    .await
                    .map_err(kernel_error)?;
                Some((
                    EvidenceKind::Assistant,
                    String::from_utf8_lossy(&range.bytes).into_owned(),
                ))
            }
            EntryPayload::Tool { tool_id } => {
                let tool = self
                    .kernel()
                    .reader
                    .tool(session, *tool_id)
                    .await
                    .map_err(kernel_error)?;
                if tool.finished_at_ms.is_none() {
                    return Ok(None);
                }
                // A read or write of our own derived notes is not independent
                // evidence. A successful forget suppresses this source window.
                let stored_name = tool.call.name.as_str();
                let target = if stored_name == "tools_call" {
                    tool.call
                        .input
                        .get("tool")
                        .and_then(serde_json::Value::as_str)
                } else if stored_name.starts_with("tools_") {
                    // Tool discovery describes capabilities; it does not verify task outcomes.
                    None
                } else {
                    Some(stored_name)
                };
                let Some(target) = target else {
                    return Ok(None);
                };
                let tool_name = target.strip_prefix("agena.").unwrap_or(target);
                if tool_name == "memory.delete" && tool.phase == ToolPhase::Succeeded {
                    return Ok(Some(Evidence {
                        entry_id: entry.id.get(),
                        source_session_id: entry.origin_session_id.get(),
                        revision: entry.revision.get(),
                        kind: EvidenceKind::MemoryForget,
                        text: "User-authorized memory deletion occurred in this source window."
                            .into(),
                    }));
                }
                if tool_name.starts_with("memory.") {
                    return Ok(None);
                }
                let mut text = format!(
                    "Tool {} ({:?})\nInput: {}\nResult: {}",
                    target,
                    tool.phase,
                    bounded_json(&tool.call.input, 512),
                    bounded_json(
                        tool.result.as_ref().unwrap_or(&serde_json::Value::Null),
                        1536
                    )
                );
                let output = tool
                    .outputs
                    .iter()
                    .find(|output| output.kind == ContentKind::Text)
                    .map(|output| output.id);
                drop(tool);
                if let Some(output) = output {
                    let range = self
                        .kernel()
                        .reader
                        .text_range(session, output, None, 1024)
                        .await
                        .map_err(kernel_error)?;
                    text.push_str("\nOutput: ");
                    text.push_str(&String::from_utf8_lossy(&range.bytes));
                }
                Some((EvidenceKind::ToolResult, truncate(&text, max).to_owned()))
            }
            EntryPayload::Compaction { summary, .. } if entry.compaction_committed => {
                let range = self
                    .kernel()
                    .reader
                    .text_range(session, *summary, None, max)
                    .await
                    .map_err(kernel_error)?;
                Some((
                    EvidenceKind::Checkpoint,
                    String::from_utf8_lossy(&range.bytes).into_owned(),
                ))
            }
            // Reasoning and injected notices are not user evidence.
            _ => None,
        };
        Ok(item
            .filter(|(_, text)| !text.trim().is_empty())
            .map(|(kind, text)| Evidence {
                entry_id: entry.id.get(),
                source_session_id: entry.origin_session_id.get(),
                revision: entry.revision.get(),
                kind,
                text,
            }))
    }

    async fn memory_evidence_current(
        &self,
        session: SessionId,
        context: &EpisodeContext,
    ) -> Result<bool, AppError> {
        let control = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        if control.summary.running_executions > 0
            || control.summary.queued_executions > 0
            || control.metadata.membership_generation != context.membership
        {
            return Ok(false);
        }
        drop(control);
        let latest = self
            .kernel()
            .reader
            .history(session, None, 1)
            .await
            .map_err(kernel_error)?;
        if latest.entries.iter().map(|entry| entry.id).max() != Some(context.watermark) {
            return Ok(false);
        }
        drop(latest);
        for source in &context.evidence {
            let id = EntryId::try_from(source.entry_id)
                .map_err(|error| AppError::Config(format!("invalid memory source: {error}")))?;
            let entry = self
                .kernel()
                .reader
                .entry(session, id)
                .await
                .map_err(kernel_error)?;
            if entry.revision.get() != source.revision
                || entry.origin_session_id.get() != source.source_session_id
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

async fn read_snapshot(
    store: MemoryStore,
    name: String,
) -> Result<(MemoryWriteSnapshot, Option<MemoryRecord>), AppError> {
    tokio::task::spawn_blocking(move || {
        let snapshot = store.write_snapshot(&name)?;
        let previous = match store.get(&name) {
            Ok(record) => Some(record),
            Err(agena_storage::MemoryError::NotFound(_)) => None,
            Err(error) => return Err(error),
        };
        Ok::<_, agena_storage::MemoryError>((snapshot, previous))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(|e| AppError::Internal(e.to_string()))
}

fn writable_derived(record: &MemoryRecord) -> bool {
    record.metadata.origin == MemoryOrigin::Derived
        && record.metadata.provenance_verified
        && record.metadata.status == MemoryStatus::Active
}

async fn save_projection(
    store: MemoryStore,
    entry: NewMemory,
    snapshot: MemoryWriteSnapshot,
    options: MemoryWriteOptions,
) -> Result<MaintenanceOutcome, AppError> {
    let name = entry.name.clone();
    let source = options.source.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        match store.save_derived(entry, &snapshot, options) {
            Ok(_) => Ok(MaintenanceOutcome::Complete),
            Err(error) => {
                // An unrelated memory edit invalidates the global extraction
                // snapshot. Retry from fresh source evidence, never reuse the
                // generated draft under a new generation.
                if let Some(source) = &source
                    && store.source_is_suppressed(source)?
                {
                    return Err(error);
                }
                let current = store.write_snapshot(&name)?;
                let unchanged_target = !current.is_tombstoned
                    && current.generation != snapshot.generation
                    && current.expected_sha256 == snapshot.expected_sha256;
                let still_derived = match store.get(&name) {
                    Ok(record) => writable_derived(&record),
                    Err(agena_storage::MemoryError::NotFound(_)) => {
                        snapshot.expected_sha256.is_none()
                    }
                    Err(_) => false,
                };
                if unchanged_target && still_derived {
                    Ok(MaintenanceOutcome::Retry)
                } else {
                    Err(error)
                }
            }
        }
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(|e: agena_storage::MemoryError| AppError::Internal(e.to_string()))?;
    Ok(outcome)
}

fn parse_episode(
    raw: &str,
    session: SessionId,
    evidence: &[Evidence],
) -> Result<Option<(String, String)>, AppError> {
    let session = session.get();
    if raw.len() > MAX_SUMMARY_BYTES {
        return Err(AppError::Config("episode exceeds its byte budget".into()));
    }
    let draft: EpisodeDraft = serde_json::from_str(raw)
        .map_err(|e| AppError::Config(format!("invalid memory episode: {e}")))?;
    if draft.title.trim().is_empty()
        || draft.title.len() > 240
        || draft.title.contains(['\r', '\n'])
    {
        return Err(AppError::Config(
            "episode title must contain 1–240 bytes on one line".into(),
        ));
    }
    let sources: BTreeMap<_, _> = evidence
        .iter()
        .map(|source| (source.entry_id, source.kind))
        .collect();
    let groups = [
        ("Decisions and constraints", &draft.decisions, false),
        ("Observed outcomes", &draft.outcomes, true),
        ("Open work at this point", &draft.open_work, false),
        ("Reusable lessons", &draft.lessons, false),
    ];
    let count: usize = groups.iter().map(|(_, items, _)| items.len()).sum();
    if count == 0 {
        return Ok(None);
    }
    if count > 24 {
        return Err(AppError::Config("episode has too many claims".into()));
    }
    let mut body = format!(
        "# {}\n\nHistorical episode from session {session}; later user corrections and current environment observations take precedence. Claims cite kernel entry IDs for verification.\n",
        draft.title.trim()
    );
    for (heading, items, require_tool) in groups {
        if items.is_empty() {
            continue;
        }
        body.push_str(&format!("\n## {heading}\n"));
        for item in items {
            if item.text.trim().is_empty()
                || item.text.len() > MAX_ITEM_BYTES
                || item.evidence.is_empty()
                || item.evidence.len() > 12
                || item.evidence.iter().any(|id| !sources.contains_key(id))
                || (require_tool
                    && !item
                        .evidence
                        .iter()
                        .any(|id| sources.get(id) == Some(&EvidenceKind::ToolResult)))
            {
                return Err(AppError::Config(
                    "episode claim lacks valid supporting evidence".into(),
                ));
            }
            let ids = item
                .evidence
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            body.push_str(&format!(
                "- {} [session {session}; entries {ids}]\n",
                item.text.trim()
            ));
        }
    }
    if body.len() > MAX_SUMMARY_BYTES {
        return Err(AppError::Config(
            "rendered episode exceeds its byte budget".into(),
        ));
    }
    Ok(Some((draft.title.trim().to_owned(), body)))
}

fn previous_sources(body: &str, session: SessionId) -> Vec<i64> {
    let prefix = format!("[session {}; entries ", session.get());
    let mut ids = Vec::new();
    for line in body.lines().take(128) {
        let Some(line) = line.strip_suffix(']') else {
            continue;
        };
        let Some((_, references)) = line.rsplit_once(&prefix) else {
            continue;
        };
        for id in references.split(',').take(12) {
            if let Ok(id) = id.trim().parse::<i64>() {
                if id > 0 && !ids.contains(&id) {
                    ids.push(id);
                }
                if ids.len() >= 24 {
                    return ids;
                }
            }
        }
    }
    ids
}

fn truncate(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn document_text(document: &ComposerDocument, max: usize) -> String {
    let mut text = String::new();
    for node in &document.0 {
        if let ComposerNode::Text { text: part } = node {
            if text.len() >= max {
                break;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(truncate(part, max.saturating_sub(text.len())));
        }
    }
    text
}
fn bounded_json(value: &serde_json::Value, max: usize) -> String {
    struct Writer {
        bytes: Vec<u8>,
        max: usize,
    }
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let remaining = self.max.saturating_sub(self.bytes.len());
            let count = bytes.len().min(remaining);
            self.bytes.extend_from_slice(&bytes[..count]);
            if count == 0 && !bytes.is_empty() {
                return Err(std::io::Error::other("memory excerpt full"));
            }
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        bytes: Vec::with_capacity(max),
        max,
    };
    let truncated = serde_json::to_writer(&mut writer, value).is_err();
    let mut text = String::from_utf8_lossy(&writer.bytes).into_owned();
    if truncated {
        text.push_str(" [excerpt]");
    }
    text
}

#[cfg(test)]
mod tests;
