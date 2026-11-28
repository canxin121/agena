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
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agena_domain::{
    ExternalAgent, ExternalAgentArchiveFilter, ExternalAgentIndexCapabilities,
    ExternalAgentIndexedSession, ExternalAgentSessionCapabilities, ResolvedExternalAgent,
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
struct AgentSpecFingerprint {
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

fn spec_fingerprint(spec: &ResolvedExternalAgent) -> AgentSpecFingerprint {
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
        memo.insert(
            key,
            MemoEntry {
                version,
                capabilities: capabilities.or(existing_caps),
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
}

impl SessionManager {
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
    }
}
