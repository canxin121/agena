//! Bounded read-only control plane for external ACP agents.
//!
//! Combines three low-overhead layers so listing and status checks stay fast
//! while keeping steady-state RSS strictly bounded (and 0 MB when idle):
//! 1. Protocol version & static capability memoization (skips repeated v2->v1
//!    double-spawns and avoids spawning peers for unsupported `session/status`).
//! 2. Short-lived read-only control-plane pool (at most 2 live slots, 60s idle
//!    auto-reaper, 15m max age) with per-slot singleflight serialization and a
//!    5s cross-workspace raw list snapshot so concurrent expanded workspaces
//!    share one cold boot and one RPC.
//! 3. Bounded asynchronous client retirement on `SessionManager::control_tasks`
//!    so `AcpClient::close()` timeouts do not block HTTP responses.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agena_domain::{
    ExternalAgent, ExternalAgentArchiveFilter, ExternalAgentAuth, ExternalAgentCommand,
    ExternalAgentHistoryPreview, ExternalAgentIndexCapabilities, ExternalAgentIndexedSession,
    ExternalAgentOption, ExternalAgentOptionChange, ExternalAgentOptionKind, ExternalAgentProviders,
    ExternalAgentSessionCapabilities, ExternalAgentSessionInfo, ResolvedExternalAgent,
};
use agena_storage::kernel::{
    ExternalAgentHistoryAnchor, ExternalAgentHistoryTurn, ExternalAgentLink, ExternalAgentState,
};
use sha2::{Digest, Sha256};
use tokio::{
    sync::{Mutex as AsyncMutex, Semaphore},
    task::AbortHandle,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{AppError, SessionManager, acp};

const PROTOCOL_MEMO_TTL: Duration = Duration::from_secs(600);
const MAX_MEMO_ENTRIES: usize = 64;
const MAX_CONTROL_SLOTS: usize = 2;
const CONTROL_IDLE_TTL: Duration = Duration::from_secs(60);
const CONTROL_MAX_AGE: Duration = Duration::from_secs(900);
const LIST_SNAPSHOT_TTL: Duration = Duration::from_secs(5);
const MAX_SNAPSHOT_ENTRIES: usize = 64;
const MAX_CLOSING_CLIENTS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct AgentSpecFingerprint {
    agent: ExternalAgent,
    program: PathBuf,
    args: Vec<String>,
    env_hash: [u8; 32],
    binary_stamp: Option<(u64, u128)>,
    client_tools: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SlotKey {
    spec: AgentSpecFingerprint,
    scoped_cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SnapshotKey {
    spec: AgentSpecFingerprint,
    filter: u8,
}

fn archive_filter_key(filter: ExternalAgentArchiveFilter) -> u8 {
    match filter {
        ExternalAgentArchiveFilter::Unarchived => 0,
        ExternalAgentArchiveFilter::Archived => 1,
        ExternalAgentArchiveFilter::All => 2,
    }
}

#[derive(Clone)]
struct MemoEntry {
    version: acp::Version,
    capabilities: Option<ExternalAgentSessionCapabilities>,
    air_index: Option<bool>,
    recorded_at: Instant,
}

#[derive(Clone)]
struct SnapshotEntry {
    recorded_at: Instant,
    capabilities: ExternalAgentIndexCapabilities,
    rows: Vec<ExternalAgentIndexedSession>,
    cross_workspace: bool,
    source_cwd: PathBuf,
}

struct PooledClient {
    client: acp::AcpClient,
    spawned_at: Instant,
    spawn_cwd: PathBuf,
}

struct ControlSlot {
    client: AsyncMutex<Option<PooledClient>>,
    last_used_ms: AtomicU64,
    reaper: StdMutex<Option<AbortHandle>>,
}

impl ControlSlot {
    fn new() -> Self {
        Self {
            client: AsyncMutex::new(None),
            last_used_ms: AtomicU64::new(now_millis()),
            reaper: StdMutex::new(None),
        }
    }

    fn touch(&self) {
        self.last_used_ms.store(now_millis(), Ordering::Relaxed);
    }

    fn cancel_reaper(&self) {
        if let Some(handle) = self
            .reaper
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            handle.abort();
        }
    }
}

impl Drop for ControlSlot {
    fn drop(&mut self) {
        self.cancel_reaper();
    }
}

struct ControlPlaneInner {
    memo: StdMutex<HashMap<AgentSpecFingerprint, MemoEntry>>,
    cwd_scoped: StdMutex<HashSet<AgentSpecFingerprint>>,
    snapshots: StdMutex<HashMap<SnapshotKey, SnapshotEntry>>,
    slots: AsyncMutex<HashMap<SlotKey, Arc<ControlSlot>>>,
    closing_permits: Arc<Semaphore>,
}

impl Default for ControlPlaneInner {
    fn default() -> Self {
        Self {
            memo: StdMutex::new(HashMap::new()),
            cwd_scoped: StdMutex::new(HashSet::new()),
            snapshots: StdMutex::new(HashMap::new()),
            slots: AsyncMutex::new(HashMap::new()),
            closing_permits: Arc::new(Semaphore::new(MAX_CLOSING_CLIENTS)),
        }
    }
}

#[derive(Clone, Default)]
pub(in crate::session::manager) struct ControlPlane {
    inner: Arc<ControlPlaneInner>,
}

pub(super) enum ControlListOutcome {
    Page {
        sessions: Vec<ExternalAgentIndexedSession>,
        next_cursor: Option<String>,
        capabilities: ExternalAgentIndexCapabilities,
    },
    NeedsAirIndexFallback,
}

pub(super) struct ControlStatusOutcome {
    pub capabilities: ExternalAgentSessionCapabilities,
    pub native_status: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ControlReplayMode {
    Replay,
    Preview(Option<u64>),
}

pub(super) enum ControlReplayOutcome {
    Replayed {
        record: ExternalAgentSessionInfo,
        turns: Vec<ExternalAgentHistoryTurn>,
        state: ExternalAgentState,
    },
    Previewed {
        preview: ExternalAgentHistoryPreview,
    },
    NeedsIndexFallback(Option<AppError>),
}

pub(super) struct ControlOptionsRequest<'a> {
    pub agent: &'a ExternalAgent,
    pub cwd: &'a Path,
    pub previous: Option<&'a str>,
    pub previous_state: &'a ExternalAgentState,
    pub selection: BTreeMap<ExternalAgentOptionKind, String>,
    pub change: Option<&'a ExternalAgentOptionChange>,
    pub model_changed: bool,
    pub preview: bool,
    pub native_linked: bool,
    pub session_context: Option<acp::SessionContext>,
}

pub(super) struct ControlOptionsOutcome {
    pub selection: BTreeMap<ExternalAgentOptionKind, String>,
    pub options: Vec<ExternalAgentOption>,
    pub preview_commands: Vec<ExternalAgentCommand>,
    pub new_link: Option<ExternalAgentLink>,
}

/// Whether the agent's read-only control-plane process can safely stay warm
/// while separate chat turn or mutating processes run concurrently.
/// Antigravity coordinates through a shared daemon, whereas agents such as
/// Claude Code (or single-connection test fixtures) expect exclusive access.
pub(super) fn supports_concurrent_control_plane(spec: &ResolvedExternalAgent) -> bool {
    spec.id == ExternalAgent::Antigravity
}

/// Read-only control-plane operations (such as `session/list` and `session/status`)
/// do not open chat turns or MCP bridges. For built-in Antigravity with its default
/// arguments, omitting `--process-per-session` avoids spawning a separate 183 MB
/// router process on top of the 238 MB server process (~44% RSS reduction and faster
/// cold startup) while leaving user-customized arguments and chat turns untouched.
pub(super) fn control_plane_spec(spec: &ResolvedExternalAgent) -> ResolvedExternalAgent {
    if spec.id == ExternalAgent::Antigravity
        && spec.args.len() == 2
        && spec.args[0] == "acp"
        && spec.args[1] == "--process-per-session"
    {
        let mut light = spec.clone();
        light.args = vec!["acp".into()];
        return light;
    }
    spec.clone()
}

pub(super) fn spec_fingerprint(spec: &ResolvedExternalAgent) -> AgentSpecFingerprint {
    let program = acp::resolved_program_path(spec);
    let binary_stamp = std::fs::metadata(&program).ok().map(|meta| {
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        (meta.len(), modified)
    });
    let mut hasher = Sha256::new();
    for (key, value) in &spec.env {
        hasher.update(key.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hasher.update([0xff]);
    }
    AgentSpecFingerprint {
        agent: spec.id.clone(),
        program,
        args: spec.args.clone(),
        env_hash: hasher.finalize().into(),
        binary_stamp,
        client_tools: spec.client_tools,
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

impl ControlPlane {
    pub(super) fn preferred_version(&self, spec: &ResolvedExternalAgent) -> Option<acp::Version> {
        let key = spec_fingerprint(spec);
        let mut memo = self
            .inner
            .memo
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = memo.get(&key)?;
        if entry.recorded_at.elapsed() >= PROTOCOL_MEMO_TTL {
            memo.remove(&key);
            return None;
        }
        Some(entry.version)
    }

    pub(super) fn cached_capabilities(
        &self,
        spec: &ResolvedExternalAgent,
    ) -> Option<ExternalAgentSessionCapabilities> {
        let key = spec_fingerprint(spec);
        let mut memo = self
            .inner
            .memo
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = memo.get(&key)?;
        if entry.recorded_at.elapsed() >= PROTOCOL_MEMO_TTL {
            memo.remove(&key);
            return None;
        }
        entry.capabilities
    }

    pub(super) fn cached_air_index(&self, spec: &ResolvedExternalAgent) -> Option<bool> {
        let key = spec_fingerprint(&control_plane_spec(spec));
        let mut memo = self
            .inner
            .memo
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = memo.get(&key)?;
        if entry.recorded_at.elapsed() >= PROTOCOL_MEMO_TTL {
            memo.remove(&key);
            return None;
        }
        entry.air_index
    }

    pub(super) fn record_air_index(&self, spec: &ResolvedExternalAgent, supported: bool) {
        for candidate in [control_plane_spec(spec), spec.clone()] {
            let key = spec_fingerprint(&candidate);
            let mut memo = self
                .inner
                .memo
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(entry) = memo.get_mut(&key) {
                entry.air_index = Some(supported);
                entry.recorded_at = Instant::now();
            }
        }
    }

    pub(super) fn record_client_memo(
        &self,
        spec: &ResolvedExternalAgent,
        client: &acp::AcpClient,
    ) {
        let Some(version) = client.negotiated_version() else {
            return;
        };
        let capabilities = client.cached_lifecycle_capabilities();
        self.record_memo_entry(spec, version, capabilities);
        let control = control_plane_spec(spec);
        if control.args != spec.args {
            self.record_memo_entry(&control, version, capabilities);
        }
    }

    fn record_memo_entry(
        &self,
        spec: &ResolvedExternalAgent,
        version: acp::Version,
        capabilities: Option<ExternalAgentSessionCapabilities>,
    ) {
        let key = spec_fingerprint(spec);
        let mut memo = self
            .inner
            .memo
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        memo.retain(|_, entry| entry.recorded_at.elapsed() < PROTOCOL_MEMO_TTL);
        if memo.len() >= MAX_MEMO_ENTRIES
            && !memo.contains_key(&key)
            && let Some(oldest) = memo
                .iter()
                .min_by_key(|(_, entry)| entry.recorded_at)
                .map(|(oldest_key, _)| oldest_key.clone())
        {
            memo.remove(&oldest);
        }
        let existing_caps = memo.get(&key).and_then(|entry| entry.capabilities);
        let existing_air = memo.get(&key).and_then(|entry| entry.air_index);
        memo.insert(
            key,
            MemoEntry {
                version,
                capabilities: capabilities.or(existing_caps),
                air_index: existing_air,
                recorded_at: Instant::now(),
            },
        );
    }

    fn clear_memo(&self, spec: &ResolvedExternalAgent) {
        let key = spec_fingerprint(spec);
        self.inner
            .memo
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&key);
    }

    fn is_cwd_scoped(&self, spec_fp: &AgentSpecFingerprint) -> bool {
        self.inner
            .cwd_scoped
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(spec_fp)
    }

    fn mark_cwd_scoped(&self, spec_fp: AgentSpecFingerprint) {
        let mut scoped = self
            .inner
            .cwd_scoped
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if scoped.len() >= MAX_MEMO_ENTRIES {
            scoped.clear();
        }
        scoped.insert(spec_fp);
    }

    fn lookup_snapshot(
        &self,
        spec_fp: &AgentSpecFingerprint,
        filter: ExternalAgentArchiveFilter,
        cwd: &Path,
    ) -> Option<(Vec<ExternalAgentIndexedSession>, ExternalAgentIndexCapabilities)> {
        let key = SnapshotKey {
            spec: spec_fp.clone(),
            filter: archive_filter_key(filter),
        };
        let mut snapshots = self
            .inner
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = snapshots.get(&key)?;
        if entry.recorded_at.elapsed() >= LIST_SNAPSHOT_TTL {
            snapshots.remove(&key);
            return None;
        }
        if !entry.cross_workspace && entry.source_cwd != cwd {
            return None;
        }
        let sessions = entry
            .rows
            .iter()
            .filter(|row| row.session.cwd == cwd)
            .cloned()
            .collect();
        Some((sessions, entry.capabilities))
    }

    fn lookup_cached_record(
        &self,
        spec_fp: &AgentSpecFingerprint,
        cwd: &Path,
        native_id: &str,
    ) -> Option<ExternalAgentSessionInfo> {
        let mut snapshots = self
            .inner
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for filter in [
            ExternalAgentArchiveFilter::All,
            ExternalAgentArchiveFilter::Unarchived,
        ] {
            let key = SnapshotKey {
                spec: spec_fp.clone(),
                filter: archive_filter_key(filter),
            };
            let Some(entry) = snapshots.get(&key) else {
                continue;
            };
            if entry.recorded_at.elapsed() >= LIST_SNAPSHOT_TTL {
                snapshots.remove(&key);
                continue;
            }
            if !entry.cross_workspace && entry.source_cwd != cwd {
                continue;
            }
            if let Some(row) = entry
                .rows
                .iter()
                .find(|row| row.session.cwd == cwd && row.session.native_id == native_id)
            {
                return Some(row.session.clone());
            }
        }
        None
    }

    fn clear_spec_snapshots(&self, spec_fp: &AgentSpecFingerprint) {
        self.inner
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|key, _| &key.spec != spec_fp);
    }

    fn store_snapshot(
        &self,
        spec_fp: AgentSpecFingerprint,
        filter: ExternalAgentArchiveFilter,
        source_cwd: PathBuf,
        cross_workspace: bool,
        rows: Vec<ExternalAgentIndexedSession>,
        capabilities: ExternalAgentIndexCapabilities,
    ) {
        let key = SnapshotKey {
            spec: spec_fp,
            filter: archive_filter_key(filter),
        };
        let mut snapshots = self
            .inner
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        snapshots.retain(|_, entry| entry.recorded_at.elapsed() < LIST_SNAPSHOT_TTL);
        if snapshots.len() >= MAX_SNAPSHOT_ENTRIES
            && !snapshots.contains_key(&key)
            && let Some(oldest) = snapshots
                .iter()
                .min_by_key(|(_, entry)| entry.recorded_at)
                .map(|(oldest_key, _)| oldest_key.clone())
        {
            snapshots.remove(&oldest);
        }
        snapshots.insert(
            key,
            SnapshotEntry {
                recorded_at: Instant::now(),
                capabilities,
                rows,
                cross_workspace,
                source_cwd,
            },
        );
    }

    pub(super) async fn retire_client(
        &self,
        client: acp::AcpClient,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
    ) {
        if !shutdown.is_cancelled()
            && let Ok(permit) = self.inner.closing_permits.clone().try_acquire_owned()
        {
            control_tasks.spawn(async move {
                let _permit = permit;
                client.close().await;
            });
            return;
        }
        client.close().await;
    }

    async fn acquire_slot(
        &self,
        key: SlotKey,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
    ) -> Arc<ControlSlot> {
        let evicted = {
            let mut slots = self.inner.slots.lock().await;
            if let Some(existing) = slots.get(&key) {
                existing.touch();
                return Arc::clone(existing);
            }
            let mut evicted = Vec::new();
            while slots.len() >= MAX_CONTROL_SLOTS {
                let Some(lru_key) = slots
                    .iter()
                    .min_by_key(|(_, slot)| slot.last_used_ms.load(Ordering::Relaxed))
                    .map(|(slot_key, _)| slot_key.clone())
                else {
                    break;
                };
                if let Some(removed) = slots.remove(&lru_key) {
                    removed.cancel_reaper();
                    evicted.push(removed);
                }
            }
            let slot = Arc::new(ControlSlot::new());
            slots.insert(key, Arc::clone(&slot));
            (slot, evicted)
        };
        let (slot, evicted_slots) = evicted;
        for old_slot in evicted_slots {
            if let Some(pooled) = old_slot.client.lock().await.take() {
                self.retire_client(pooled.client, control_tasks, shutdown)
                    .await;
            }
        }
        slot
    }

    fn arm_idle_reaper(
        &self,
        key: SlotKey,
        slot: &Arc<ControlSlot>,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
    ) {
        slot.cancel_reaper();
        if shutdown.is_cancelled() {
            return;
        }
        let plane = self.clone();
        let slot_ref = Arc::clone(slot);
        let tracker = control_tasks.clone();
        let shutdown_token = shutdown.clone();
        let handle = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = shutdown_token.cancelled() => return,
                _ = tokio::time::sleep(CONTROL_IDLE_TTL) => {}
            }
            let elapsed_ms = now_millis().saturating_sub(slot_ref.last_used_ms.load(Ordering::Relaxed));
            if Duration::from_millis(elapsed_ms) < CONTROL_IDLE_TTL {
                return;
            }
            let removed_client = {
                let mut guard = slot_ref.client.lock().await;
                let still_idle_ms =
                    now_millis().saturating_sub(slot_ref.last_used_ms.load(Ordering::Relaxed));
                if Duration::from_millis(still_idle_ms) < CONTROL_IDLE_TTL {
                    return;
                }
                guard.take()
            };
            {
                let mut slots = plane.inner.slots.lock().await;
                if slots
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, &slot_ref))
                {
                    slots.remove(&key);
                }
            }
            if let Some(pooled) = removed_client {
                tracing::debug!(
                    agent = key.spec.agent.id(),
                    "reaping idle ACP control-plane process"
                );
                plane
                    .retire_client(pooled.client, &tracker, &shutdown_token)
                    .await;
            }
        })
        .abort_handle();
        *slot
            .reaper
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(handle);
    }

    pub(super) async fn invalidate_agent(
        &self,
        agent: &ExternalAgent,
        evict_process: bool,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
    ) {
        self.inner
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|key, _| &key.spec.agent != agent);
        if !evict_process {
            return;
        }
        let evicted = {
            let mut slots = self.inner.slots.lock().await;
            let matching: Vec<_> = slots
                .keys()
                .filter(|key| &key.spec.agent == agent)
                .cloned()
                .collect();
            let mut removed = Vec::with_capacity(matching.len());
            for key in matching {
                if let Some(slot) = slots.remove(&key) {
                    slot.cancel_reaper();
                    removed.push(slot);
                }
            }
            removed
        };
        for slot in evicted {
            if let Some(pooled) = slot.client.lock().await.take() {
                self.retire_client(pooled.client, control_tasks, shutdown)
                    .await;
            }
        }
    }

    /// Synchronously evicts and closes all pooled control-plane clients for an
    /// agent before an exclusive peer spawns (for agents that cannot safely
    /// share a workspace across concurrent connections).
    pub(super) async fn evict_agent_sync(&self, agent: &ExternalAgent) {
        self.inner
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|key, _| &key.spec.agent != agent);
        let evicted = {
            let mut slots = self.inner.slots.lock().await;
            let matching: Vec<_> = slots
                .keys()
                .filter(|key| &key.spec.agent == agent)
                .cloned()
                .collect();
            let mut removed = Vec::with_capacity(matching.len());
            for key in matching {
                if let Some(slot) = slots.remove(&key) {
                    slot.cancel_reaper();
                    removed.push(slot);
                }
            }
            removed
        };
        for slot in evicted {
            if let Some(pooled) = slot.client.lock().await.take() {
                pooled.client.close().await;
            }
        }
    }

    pub(in crate::session::manager) fn invalidate_all(
        &self,
        control_tasks: TaskTracker,
        shutdown: CancellationToken,
    ) {
        self.inner
            .memo
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.inner
            .cwd_scoped
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.inner
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        let plane = self.clone();
        if shutdown.is_cancelled() {
            return;
        }
        let retire_tracker = control_tasks.clone();
        control_tasks.spawn(async move {
            let evicted = {
                let mut slots = plane.inner.slots.lock().await;
                let drained: Vec<_> = slots.drain().map(|(_, slot)| slot).collect();
                for slot in &drained {
                    slot.cancel_reaper();
                }
                drained
            };
            for slot in evicted {
                if let Some(pooled) = slot.client.lock().await.take() {
                    plane
                        .retire_client(pooled.client, &retire_tracker, &shutdown)
                        .await;
                }
            }
        });
    }

    pub(in crate::session::manager) async fn shutdown_all(&self) {
        let drained = {
            let mut slots = self.inner.slots.lock().await;
            let entries: Vec<_> = slots.drain().map(|(_, slot)| slot).collect();
            for slot in &entries {
                slot.cancel_reaper();
            }
            entries
        };
        let mut clients = Vec::new();
        for slot in drained {
            if let Some(pooled) = slot.client.lock().await.take() {
                clients.push(pooled.client);
            }
        }
        let mut set = tokio::task::JoinSet::new();
        for client in clients {
            set.spawn(async move {
                client.close().await;
            });
        }
        while set.join_next().await.is_some() {}
    }

    pub(super) async fn list_standard_history(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        cursor: Option<&str>,
        archived: ExternalAgentArchiveFilter,
        allow_standard_without_archive: bool,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<ControlListOutcome, AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self.acquire_slot(slot_key.clone(), control_tasks, shutdown).await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            // Singleflight + 5s cross-workspace snapshot: when concurrent expanded
            // workspaces arrive together, waiters hit this right after the leader finishes.
            if cursor.is_none()
                && let Some((sessions, capabilities)) =
                    self.lookup_snapshot(&control_fp, archived, cwd)
            {
                if capabilities.archive || allow_standard_without_archive {
                    return Ok(ControlListOutcome::Page {
                        sessions,
                        next_cursor: None,
                        capabilities,
                    });
                }
                return Ok(ControlListOutcome::NeedsAirIndexFallback);
            }

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = async {
                let caps = pooled.client.lifecycle_capabilities(cancel).await?;
                let archive = pooled.client.supports_native_archive(cancel).await?;
                if let Some(version) = pooled.client.negotiated_version() {
                    self.record_memo_entry(&control_spec, version, Some(caps));
                    if control_spec.args != spec.args {
                        self.record_memo_entry(spec, version, Some(caps));
                    }
                }
                if !archive && !allow_standard_without_archive {
                    return Ok(ControlListOutcome::NeedsAirIndexFallback);
                }
                let page = pooled
                    .client
                    .list_native_raw(cwd, cursor, archived, cancel)
                    .await?;
                let index_caps = ExternalAgentIndexCapabilities {
                    archive,
                    title: caps.title,
                    delete: caps.delete,
                    fork: caps.fork,
                    ..Default::default()
                };
                if cursor.is_none() && page.next_cursor.is_none() {
                    let cross_workspace =
                        page.cross_workspace || spec.id == ExternalAgent::Antigravity;
                    self.store_snapshot(
                        control_fp.clone(),
                        archived,
                        cwd.to_path_buf(),
                        cross_workspace,
                        page.all_sessions,
                        index_caps,
                    );
                }
                Ok(ControlListOutcome::Page {
                    sessions: page.sessions,
                    next_cursor: page.next_cursor,
                    capabilities: index_caps,
                })
            }
            .await;

            match rpc_result {
                Ok(outcome) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(outcome);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        tracing::debug!(
                            agent = spec.id.id(),
                            was_cross_cwd,
                            %error,
                            "retrying ACP control-plane list on a fresh connection"
                        );
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane list retries at most once")
    }

    pub(super) async fn query_status(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        native_id: Option<&str>,
        running: bool,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<ControlStatusOutcome, AppError> {
        if let Some(capabilities) = self.cached_capabilities(spec)
            && (!capabilities.status || running || native_id.is_none())
        {
            return Ok(ControlStatusOutcome {
                capabilities,
                native_status: running
                    .then(|| native_id.map(|_| "open".to_owned()))
                    .flatten(),
            });
        }

        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self.acquire_slot(slot_key.clone(), control_tasks, shutdown).await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = async {
                let capabilities = pooled.client.lifecycle_capabilities(cancel).await?;
                if let Some(version) = pooled.client.negotiated_version() {
                    self.record_memo_entry(&control_spec, version, Some(capabilities));
                    if control_spec.args != spec.args {
                        self.record_memo_entry(spec, version, Some(capabilities));
                    }
                }
                let native_status = match native_id {
                    Some(_) if running => Some("open".to_owned()),
                    Some(id) => pooled.client.native_status(id, cancel).await?,
                    None => None,
                };
                Ok(ControlStatusOutcome {
                    capabilities,
                    native_status,
                })
            }
            .await;

            match rpc_result {
                Ok(outcome) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(outcome);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane status retries at most once")
    }

    /// Replays or previews native history over the warm read-only control plane
    /// when no additional directories are required. Avoids cold process spawns
    /// and skips redundant `session/list` lookups when the 5s snapshot already
    /// carries the record. Never sends `session/close` so independent native
    /// background work stays untouched.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn replay_or_preview_native(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        native_id: &str,
        known_record: Option<&ExternalAgentSessionInfo>,
        replay_context: &acp::SessionContext,
        mode: ControlReplayMode,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<ControlReplayOutcome, AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            enum AttemptStep {
                Done(ControlReplayOutcome),
                NotListed,
                Retryable(AppError, bool),
                Fatal(AppError),
            }

            let step = async {
                let mut used_cached_record = false;
                let record = if let Some(record) = known_record {
                    record.clone()
                } else if let Some(cached) = self.lookup_cached_record(&control_fp, cwd, native_id)
                {
                    used_cached_record = true;
                    cached
                } else {
                    let archive = match pooled.client.supports_native_archive(cancel).await {
                        Ok(archive) => archive,
                        Err(AppError::Cancelled) => return AttemptStep::Fatal(AppError::Cancelled),
                        Err(error) => return AttemptStep::Retryable(error, false),
                    };
                    let filter = if archive {
                        ExternalAgentArchiveFilter::All
                    } else {
                        ExternalAgentArchiveFilter::Unarchived
                    };
                    match pooled
                        .client
                        .find_workspace_native(cwd, native_id, filter, cancel)
                        .await
                    {
                        Ok(Some(row)) => row.session,
                        Ok(None) if filter == ExternalAgentArchiveFilter::Unarchived => {
                            if was_warm && was_cross_cwd && attempt == 0 {
                                return AttemptStep::NotListed;
                            }
                            return AttemptStep::Done(ControlReplayOutcome::NeedsIndexFallback(
                                None,
                            ));
                        }
                        Ok(None) => return AttemptStep::NotListed,
                        Err(AppError::Cancelled) => return AttemptStep::Fatal(AppError::Cancelled),
                        Err(error) if filter == ExternalAgentArchiveFilter::Unarchived => {
                            if was_warm && attempt == 0 {
                                return AttemptStep::Retryable(error, false);
                            }
                            return AttemptStep::Done(ControlReplayOutcome::NeedsIndexFallback(
                                Some(error),
                            ));
                        }
                        Err(error) => return AttemptStep::Retryable(error, false),
                    }
                };

                let outcome = match mode {
                    ControlReplayMode::Preview(before_turn) => pooled
                        .client
                        .preview_native_history(cwd, native_id, replay_context, before_turn, cancel)
                        .await
                        .map(|preview| ControlReplayOutcome::Previewed { preview }),
                    ControlReplayMode::Replay => pooled
                        .client
                        .replay_native(cwd, native_id, replay_context, cancel)
                        .await
                        .map(|(turns, state)| ControlReplayOutcome::Replayed {
                            record,
                            turns,
                            state,
                        }),
                };
                if let Some(version) = pooled.client.negotiated_version() {
                    let caps = pooled.client.cached_lifecycle_capabilities();
                    self.record_memo_entry(&control_spec, version, caps);
                    if control_spec.args != spec.args {
                        self.record_memo_entry(spec, version, caps);
                    }
                }
                match outcome {
                    Ok(done) => AttemptStep::Done(done),
                    Err(AppError::Cancelled) => AttemptStep::Fatal(AppError::Cancelled),
                    Err(error) => AttemptStep::Retryable(error, used_cached_record),
                }
            }
            .await;

            match step {
                AttemptStep::Done(outcome) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(outcome);
                }
                AttemptStep::NotListed => {
                    if was_warm && was_cross_cwd && attempt == 0 {
                        if let Some(dead) = guard.take() {
                            self.retire_client(dead.client, control_tasks, shutdown)
                                .await;
                        }
                        drop(guard);
                        self.clear_spec_snapshots(&control_fp);
                        if !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Err(AppError::Config(
                        "That native conversation is not listed in this workspace.".into(),
                    ));
                }
                AttemptStep::Fatal(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(error);
                }
                AttemptStep::Retryable(error, used_cached_record) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    self.clear_spec_snapshots(&control_fp);
                    if (was_warm || used_cached_record) && attempt == 0 {
                        if was_warm && was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        tracing::debug!(
                            agent = spec.id.id(),
                            was_warm,
                            used_cached_record,
                            %error,
                            "retrying ACP control-plane replay on a fresh connection"
                        );
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane replay retries at most once")
    }

    /// Inspects or updates ACP session options on the warm read-only control
    /// plane when no additional directories are configured. Reuses the warm
    /// process so model/thinking/mode changes complete in ~12ms instead of
    /// paying a full cold process boot.
    pub(super) async fn inspect_options(
        &self,
        spec: &ResolvedExternalAgent,
        request: ControlOptionsRequest<'_>,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<ControlOptionsOutcome, AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| request.cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), request.cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: request.cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != request.cwd;

            let mut selection = request.selection.clone();
            let rpc_result = async {
                pooled.client.prepare_provider_routes(cancel).await?;
                if request.preview
                    || request.previous.is_none()
                    || (request.change.is_none() && selection.is_empty())
                {
                    match pooled
                        .client
                        .session_options(request.cwd, request.previous, &[], cancel)
                        .await
                    {
                        Ok(Some(response)) => {
                            let mut advertised = super::options::config_options(&response);
                            if request.preview {
                                let commands = if !request.native_linked && pooled.client.uses_v2()
                                {
                                    super::state::available_commands(&response["availableCommands"])
                                        .unwrap_or_default()
                                } else {
                                    Vec::new()
                                };
                                return Ok(ControlOptionsOutcome {
                                    selection,
                                    options: advertised,
                                    preview_commands: commands,
                                    new_link: None,
                                });
                            }
                            if request.change.is_none() && selection.is_empty() {
                                return Ok(ControlOptionsOutcome {
                                    selection,
                                    options: advertised,
                                    preview_commands: Vec::new(),
                                    new_link: None,
                                });
                            }
                            if request.previous.is_none() {
                                if request.model_changed
                                    && let Some(thought) =
                                        selection.get(&ExternalAgentOptionKind::ThoughtLevel)
                                    && !super::options::option_offers(
                                        &advertised,
                                        &ExternalAgentOptionKind::ThoughtLevel,
                                        thought,
                                    )
                                    && selection.get(&ExternalAgentOptionKind::Model).is_none_or(
                                        |model| {
                                            super::options::option_offers(
                                                &advertised,
                                                &ExternalAgentOptionKind::Model,
                                                model,
                                            )
                                        },
                                    )
                                {
                                    selection.remove(&ExternalAgentOptionKind::ThoughtLevel);
                                }
                                let missing = selection.iter().find(|(kind, value)| {
                                    !super::options::option_offers(&advertised, kind, value)
                                });
                                if let Some((missing_kind, _)) = missing {
                                    let parent_changed = selection.iter().any(|(kind, value)| {
                                        kind != missing_kind
                                            && advertised.iter().any(|option| {
                                                &option.kind == kind
                                                    && option.current_value != *value
                                            })
                                    });
                                    if !parent_changed {
                                        return Err(AppError::Config(format!(
                                            "{} no longer offers the selected {}. Open its options and choose an available value.",
                                            request.agent.label(),
                                            missing_kind.label(),
                                        )));
                                    }
                                } else {
                                    for (kind, value) in &selection {
                                        if let Some(option) = advertised
                                            .iter_mut()
                                            .find(|option| &option.kind == kind)
                                        {
                                            option.current_value = value.clone();
                                        }
                                    }
                                    return Ok(ControlOptionsOutcome {
                                        selection,
                                        options: advertised,
                                        preview_commands: Vec::new(),
                                        new_link: None,
                                    });
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                        Err(error) if request.preview => return Err(error),
                        Err(error) => tracing::debug!(
                            %error,
                            "could not inspect ACP options without opening a session"
                        ),
                    }
                }
                if request.preview {
                    return Ok(ControlOptionsOutcome {
                        selection,
                        options: Vec::new(),
                        preview_commands: Vec::new(),
                        new_link: None,
                    });
                }
                let session_context = request
                    .session_context
                    .as_ref()
                    .expect("non-preview option inspection provides session context");
                let mut opened = pooled
                    .client
                    .start_with_context(
                        request.cwd,
                        None,
                        request.previous.map(|id| (id, request.previous_state)),
                        false,
                        acp::OpenPurpose::Prompt,
                        session_context,
                        cancel,
                    )
                    .await?;
                let result = async {
                    for (kind, value) in selection.clone() {
                        if !opened.offers(&kind, &value) {
                            if request.change.is_none() {
                                continue;
                            }
                            if kind == ExternalAgentOptionKind::ThoughtLevel
                                && request.model_changed
                            {
                                selection.remove(&kind);
                                continue;
                            }
                        }
                        pooled
                            .client
                            .set_option(&mut opened, &kind, &value, cancel)
                            .await?;
                    }
                    Ok::<_, AppError>(())
                }
                .await;
                let mut new_link = None;
                if opened.resumed {
                    pooled
                        .client
                        .close_native_session_if_supported(&opened.id)
                        .await;
                } else if result.is_ok()
                    && pooled.client.can_reopen_session()
                    && request.previous.is_none()
                {
                    pooled
                        .client
                        .close_native_session_if_supported(&opened.id)
                        .await;
                    new_link = Some(ExternalAgentLink {
                        native_id: opened.id.clone(),
                        cwd: request.cwd.to_path_buf(),
                        incarnation: None,
                        resume_at: Some(ExternalAgentHistoryAnchor { tail: None }),
                    });
                } else {
                    pooled.client.close_ephemeral(&opened, cancel).await;
                }
                result?;
                Ok(ControlOptionsOutcome {
                    selection,
                    options: opened.options,
                    preview_commands: Vec::new(),
                    new_link,
                })
            }
            .await;

            if let Some(version) = pooled.client.negotiated_version() {
                let caps = pooled.client.cached_lifecycle_capabilities();
                self.record_memo_entry(&control_spec, version, caps);
                if control_spec.args != spec.args {
                    self.record_memo_entry(spec, version, caps);
                }
            }

            match rpc_result {
                Ok(outcome) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(outcome);
                }
                Err(error @ AppError::Config(_)) => {
                    // Option validation errors come from a healthy peer; keep the warm slot.
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Err(error);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        tracing::debug!(
                            agent = spec.id.id(),
                            was_cross_cwd,
                            %error,
                            "retrying ACP control-plane options inspection on a fresh connection"
                        );
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane options retries at most once")
    }

    /// Inspects non-terminal auth capabilities and live status on the warm
    /// read-only control plane (~2ms instead of spawning a cold process).
    pub(super) async fn inspect_auth(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<ExternalAgentAuth, AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = super::auth::inspect_auth_on_client(&mut pooled.client, spec, cwd, false, cancel).await;
            if let Some(version) = pooled.client.negotiated_version() {
                let caps = pooled.client.cached_lifecycle_capabilities();
                self.record_memo_entry(&control_spec, version, caps);
                if control_spec.args != spec.args {
                    self.record_memo_entry(spec, version, caps);
                }
            }
            match rpc_result {
                Ok(auth) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(auth);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane auth inspect retries at most once")
    }

    /// Inspects advertised ACP provider routes on the warm control plane.
    pub(super) async fn inspect_providers(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<ExternalAgentProviders, AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = pooled.client.provider_catalog(cancel).await;
            if let Some(version) = pooled.client.negotiated_version() {
                let caps = pooled.client.cached_lifecycle_capabilities();
                self.record_memo_entry(&control_spec, version, caps);
                if control_spec.args != spec.args {
                    self.record_memo_entry(spec, version, caps);
                }
            }
            match rpc_result {
                Ok(catalog) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(catalog);
                }
                Err(error @ AppError::Config(_)) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Err(error);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane providers inspect retries at most once")
    }

    /// Renames a native conversation over the warm control plane (~15ms vs ~7s).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn set_native_title(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        native_id: &str,
        title: &str,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<String, AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = pooled.client.set_native_title(native_id, title, cancel).await;
            if let Some(version) = pooled.client.negotiated_version() {
                let caps = pooled.client.cached_lifecycle_capabilities();
                self.record_memo_entry(&control_spec, version, caps);
                if control_spec.args != spec.args {
                    self.record_memo_entry(spec, version, caps);
                }
            }
            match rpc_result {
                Ok(saved) => {
                    self.clear_spec_snapshots(&control_fp);
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(saved);
                }
                Err(error @ AppError::Config(_)) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Err(error);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane set_native_title retries at most once")
    }

    /// Closes or deletes a native conversation over the warm control plane.
    /// Callers must only pass `delete = true` for agents that do not require
    /// Claude v1's `plain_delete` initialization flag.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn release_native(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        native_id: &str,
        delete: bool,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = pooled.client.release_native(native_id, delete, cancel).await;
            if let Some(version) = pooled.client.negotiated_version() {
                let caps = pooled.client.cached_lifecycle_capabilities();
                self.record_memo_entry(&control_spec, version, caps);
                if control_spec.args != spec.args {
                    self.record_memo_entry(spec, version, caps);
                }
            }
            match rpc_result {
                Ok(()) => {
                    self.clear_spec_snapshots(&control_fp);
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(());
                }
                Err(error @ AppError::Config(_)) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Err(error);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane release_native retries at most once")
    }

    /// Toggles standard ACP archive state on the warm control plane. Returns
    /// `Ok(true)` if archived/unarchived via standard ACP, or `Ok(false)` when
    /// the peer does not advertise standard archive and needs AIR index fallback.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn set_native_archived(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        native_id: &str,
        archived: bool,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<bool, AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = async {
                let supported = pooled.client.supports_native_archive(cancel).await?;
                if !supported {
                    return Ok(false);
                }
                pooled
                    .client
                    .set_native_archived(cwd, native_id, archived, cancel)
                    .await?;
                Ok(true)
            }
            .await;
            if let Some(version) = pooled.client.negotiated_version() {
                let caps = pooled.client.cached_lifecycle_capabilities();
                self.record_memo_entry(&control_spec, version, caps);
                if control_spec.args != spec.args {
                    self.record_memo_entry(spec, version, caps);
                }
            }
            match rpc_result {
                Ok(done) => {
                    if done {
                        self.clear_spec_snapshots(&control_fp);
                    }
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(done);
                }
                Err(error @ AppError::Config(_)) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Err(error);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane set_native_archived retries at most once")
    }

    /// Forks a native conversation on the warm control plane when no additional
    /// workspace directories are required. Returns `(new_native_id, capabilities)`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fork_native(
        &self,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        native_id: &str,
        cursor: Option<&str>,
        context: &acp::SessionContext,
        control_tasks: &TaskTracker,
        shutdown: &CancellationToken,
        cancel: &CancellationToken,
    ) -> Result<(String, ExternalAgentSessionCapabilities), AppError> {
        let control_spec = control_plane_spec(spec);
        let control_fp = spec_fingerprint(&control_spec);
        let isolated = acp::is_isolated(&control_spec.id);

        for attempt in 0..2 {
            let cwd_scoped = isolated || self.is_cwd_scoped(&control_fp);
            let slot_key = SlotKey {
                spec: control_fp.clone(),
                scoped_cwd: cwd_scoped.then(|| cwd.to_path_buf()),
            };
            let slot = self
                .acquire_slot(slot_key.clone(), control_tasks, shutdown)
                .await;
            let mut guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                guard = slot.client.lock() => guard,
            };
            slot.touch();

            if let Some(existing) = guard.as_ref()
                && existing.spawned_at.elapsed() >= CONTROL_MAX_AGE
                && let Some(expired) = guard.take()
            {
                self.retire_client(expired.client, control_tasks, shutdown)
                    .await;
            }

            let was_warm = guard.is_some();
            if guard.is_none() {
                let mut client = acp::AcpClient::spawn(control_spec.clone(), cwd)?;
                client.set_preferred_version(self.preferred_version(&control_spec));
                *guard = Some(PooledClient {
                    client,
                    spawned_at: Instant::now(),
                    spawn_cwd: cwd.to_path_buf(),
                });
            }
            let pooled = guard.as_mut().expect("control-plane client initialized");
            let was_cross_cwd = pooled.spawn_cwd != cwd;

            let rpc_result = async {
                let caps = pooled.client.lifecycle_capabilities(cancel).await?;
                let new_id = pooled
                    .client
                    .fork_native(cwd, native_id, cursor, context, cancel)
                    .await?;
                Ok((new_id, caps))
            }
            .await;
            if let Some(version) = pooled.client.negotiated_version() {
                let caps = pooled.client.cached_lifecycle_capabilities();
                self.record_memo_entry(&control_spec, version, caps);
                if control_spec.args != spec.args {
                    self.record_memo_entry(spec, version, caps);
                }
            }
            match rpc_result {
                Ok(result) => {
                    self.clear_spec_snapshots(&control_fp);
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Ok(result);
                }
                Err(error @ AppError::Config(_)) => {
                    slot.touch();
                    self.arm_idle_reaper(slot_key, &slot, control_tasks, shutdown);
                    return Err(error);
                }
                Err(AppError::Cancelled) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    return Err(AppError::Cancelled);
                }
                Err(error) => {
                    if let Some(dead) = guard.take() {
                        self.retire_client(dead.client, control_tasks, shutdown)
                            .await;
                    }
                    drop(guard);
                    self.clear_memo(&control_spec);
                    if control_spec.args != spec.args {
                        self.clear_memo(spec);
                    }
                    if was_warm && attempt == 0 {
                        if was_cross_cwd && !cwd_scoped {
                            self.mark_cwd_scoped(control_fp.clone());
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("control-plane fork_native retries at most once")
    }
}

impl SessionManager {
    pub(super) async fn prepare_exclusive_external_agent_spawn(&self, spec: &ResolvedExternalAgent) {
        if !supports_concurrent_control_plane(spec) {
            self.external_control_plane.evict_agent_sync(&spec.id).await;
        }
    }

    pub(super) fn spawn_external_agent(
        &self,
        spec: ResolvedExternalAgent,
        cwd: &Path,
    ) -> Result<acp::AcpClient, AppError> {
        self.spawn_external_agent_with_directories(spec, cwd, &[])
    }

    pub(super) fn spawn_external_agent_with_directories(
        &self,
        spec: ResolvedExternalAgent,
        cwd: &Path,
        directories: &[PathBuf],
    ) -> Result<acp::AcpClient, AppError> {
        let preferred = self.external_control_plane.preferred_version(&spec);
        let mut client = acp::AcpClient::spawn_with_directories(spec, cwd, directories)?;
        client.set_preferred_version(preferred);
        Ok(client)
    }

    pub(super) fn record_external_agent_memo(
        &self,
        spec: &ResolvedExternalAgent,
        client: &acp::AcpClient,
    ) {
        self.external_control_plane.record_client_memo(spec, client);
    }

    pub(super) async fn retire_external_agent_client(&self, client: acp::AcpClient) {
        self.external_control_plane
            .retire_client(client, &self.control_tasks, &self.shutdown)
            .await;
    }

    pub(super) async fn invalidate_external_agent_control_plane(
        &self,
        agent: &ExternalAgent,
        evict_process: bool,
    ) {
        self.external_control_plane
            .invalidate_agent(agent, evict_process, &self.control_tasks, &self.shutdown)
            .await;
    }

    pub(in crate::session::manager) fn invalidate_all_external_agent_control_planes(&self) {
        self.external_control_plane
            .invalidate_all(self.control_tasks.clone(), self.shutdown.clone());
        if let Some(acp) = self.acp_scope.get() {
            acp.invalidate_all_external_agent_control_planes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_spec(agent: ExternalAgent, args: &[&str]) -> ResolvedExternalAgent {
        ResolvedExternalAgent {
            id: agent,
            name: "Sample".into(),
            builtin: true,
            command: "agy".into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: BTreeMap::new(),
            client_tools: true,
            writable_paths: Vec::new(),
        }
    }

    #[test]
    fn control_plane_spec_strips_process_per_session_only_for_default_antigravity() {
        let default_agy = sample_spec(
            ExternalAgent::Antigravity,
            &["acp", "--process-per-session"],
        );
        assert_eq!(control_plane_spec(&default_agy).args, vec!["acp"]);

        let custom_agy = sample_spec(
            ExternalAgent::Antigravity,
            &["acp", "--process-per-session", "--sandbox"],
        );
        assert_eq!(control_plane_spec(&custom_agy).args, custom_agy.args);

        let claude = sample_spec(
            ExternalAgent::ClaudeCode,
            &["acp", "--process-per-session"],
        );
        assert_eq!(control_plane_spec(&claude).args, claude.args);
    }

    #[test]
    fn snapshot_filters_by_cwd_and_respects_cross_workspace_flag() {
        let plane = ControlPlane::default();
        let spec = sample_spec(ExternalAgent::Antigravity, &["acp"]);
        let fp = spec_fingerprint(&spec);
        let rows = vec![
            ExternalAgentIndexedSession::from(agena_domain::ExternalAgentSessionInfo {
                native_id: "s1".into(),
                cwd: PathBuf::from("/ws/a"),
                title: Some("A".into()),
                updated_at: None,
            }),
            ExternalAgentIndexedSession::from(agena_domain::ExternalAgentSessionInfo {
                native_id: "s2".into(),
                cwd: PathBuf::from("/ws/b"),
                title: Some("B".into()),
                updated_at: None,
            }),
        ];
        let caps = ExternalAgentIndexCapabilities::default();
        plane.store_snapshot(
            fp.clone(),
            ExternalAgentArchiveFilter::Unarchived,
            PathBuf::from("/ws/a"),
            true,
            rows.clone(),
            caps,
        );

        let (ws_b, _) = plane
            .lookup_snapshot(&fp, ExternalAgentArchiveFilter::Unarchived, Path::new("/ws/b"))
            .expect("cross-workspace snapshot hit");
        assert_eq!(ws_b.len(), 1);
        assert_eq!(ws_b[0].session.native_id, "s2");

        // Different archive filter must miss.
        assert!(
            plane
                .lookup_snapshot(&fp, ExternalAgentArchiveFilter::Archived, Path::new("/ws/b"))
                .is_none()
        );

        // Non-cross-workspace snapshot only serves its own cwd.
        plane.store_snapshot(
            fp.clone(),
            ExternalAgentArchiveFilter::All,
            PathBuf::from("/ws/a"),
            false,
            rows,
            caps,
        );
        assert!(
            plane
                .lookup_snapshot(&fp, ExternalAgentArchiveFilter::All, Path::new("/ws/a"))
                .is_some()
        );
        assert!(
            plane
                .lookup_snapshot(&fp, ExternalAgentArchiveFilter::All, Path::new("/ws/b"))
                .is_none()
        );
        assert_eq!(
            plane
                .lookup_cached_record(&fp, Path::new("/ws/b"), "s2")
                .map(|row| row.native_id),
            Some("s2".to_owned())
        );
        assert!(
            plane
                .lookup_cached_record(&fp, Path::new("/ws/b"), "s1")
                .is_none()
        );
    }

    #[test]
    fn concurrent_control_plane_is_enabled_only_for_antigravity() {
        let agy = sample_spec(
            ExternalAgent::Antigravity,
            &["acp", "--process-per-session"],
        );
        assert!(supports_concurrent_control_plane(&agy));

        let claude = sample_spec(ExternalAgent::ClaudeCode, &["acp"]);
        assert!(!supports_concurrent_control_plane(&claude));
    }
}
