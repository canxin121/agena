//! Cordis module state belongs to a project session, including listener closures.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use agena_plugin_sdk::host_api::CordisJobEvent;
use agena_plugin_sdk::{HostClient, PluginError, Result};
use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};
use tokio_util::sync::CancellationToken;

use super::{
    Program, RuntimeSnapshot, SCRIPT_TIMEOUT, Vm, WORKERS, WorkerHostBridge, jobs::VmJobs, skills, subagents,
    events::Event,
};

const MAX_SESSIONS: usize = 128;

pub(super) type Sessions = Mutex<BTreeMap<i64, Arc<RetainedModule>>>;

pub(in crate::agent_plugin) struct RetainedModule {
    lifecycle: CancellationToken,
    serial: AsyncMutex<()>,
    vm: Mutex<Option<Vm>>,
    observing_mcp: AtomicBool,
    observing_events: AtomicBool,
    tool_visibility: Mutex<BTreeMap<String, bool>>,
    pub(super) subagents: Arc<subagents::Runs>,
}

impl Default for RetainedModule {
    fn default() -> Self {
        let lifecycle = CancellationToken::new();
        Self {
            subagents: Arc::new(subagents::Runs::new(lifecycle.child_token())),
            lifecycle,
            serial: AsyncMutex::new(()),
            vm: Mutex::new(None),
            observing_mcp: AtomicBool::new(false),
            observing_events: AtomicBool::new(false),
            tool_visibility: Mutex::new(BTreeMap::new()),
        }
    }
}

impl Drop for RetainedModule {
    fn drop(&mut self) {
        self.lifecycle.cancel();
    }
}

impl Program {
    pub(in crate::agent_plugin) async fn start_session(
        self: &Arc<Self>,
        session: i64,
        host: &dyn HostClient,
    ) -> Result<Arc<RetainedModule>> {
        let slot = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            sessions.retain(|_, slot| !slot.lifecycle.is_cancelled());
            if sessions.len() >= MAX_SESSIONS && !sessions.contains_key(&session) {
                return Err(PluginError::internal("Cordis active session limit reached"));
            }
            Arc::clone(sessions.entry(session).or_default())
        };
        let serial = slot.serial.lock().await;
        if slot.lifecycle.is_cancelled() {
            return Err(PluginError::internal("Cordis module session ended"));
        }
        let needs_vm = slot
            .vm
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_none();
        let mut initial_mcp = Value::Null;
        let snapshots = if self.uses_mcp_manager && !slot.observing_mcp.load(Ordering::Relaxed) {
            match host
                .observe_deepseek_mcp_manager(slot.lifecycle.child_token())
                .await
            {
                Ok(mut updates) => {
                    initial_mcp = updates.borrow_and_update().clone();
                    Some(updates)
                }
                Err(error) if self.uses_skills => {
                    // Catalog and command state can start before the session
                    // has permission to observe the MCP manager.
                    initial_mcp = serde_json::json!({"error": error.failure.user.fallback});
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        if needs_vm {
            let registry = if self.uses_jobs {
                let service = host
                    .open_cordis_job_registry(slot.lifecycle.child_token())
                    .await?;
                let events = service.subscribe();
                Some((service, events))
            } else {
                None
            };
            let vm = self
                .load_session_vm(host, initial_mcp, &slot.lifecycle)
                .await?;
            if slot.lifecycle.is_cancelled() {
                return Err(PluginError::internal("Cordis module session ended"));
            }
            let jobs = self.uses_jobs.then(|| vm.jobs.clone());
            if let Some((registry, events)) = registry {
                vm.jobs.bind_registry(session, registry, slot.lifecycle.clone());
                tokio::spawn(forward_job_events(
                    vm.jobs.clone(),
                    slot.lifecycle.clone(),
                    events,
                ));
            }
            *slot.vm.lock().unwrap_or_else(|error| error.into_inner()) = Some(vm);
            if let Some(jobs) = jobs {
                tokio::spawn(observe_jobs(
                    Arc::downgrade(&slot),
                    slot.lifecycle.clone(),
                    jobs,
                ));
            }
            if self.uses_subagents {
                tokio::spawn(observe_subagents(Arc::downgrade(&slot), slot.subagents.clone(), slot.lifecycle.clone()));
            }
        } else if snapshots.is_some() {
            slot.update_status(initial_mcp, slot.lifecycle.child_token())
                .await?;
        }
        if let Some(snapshots) = snapshots {
            slot.observing_mcp.store(true, Ordering::Relaxed);
            tokio::spawn(observe_session(
                Arc::downgrade(&slot),
                slot.lifecycle.clone(),
                snapshots,
            ));
        }
        if self.observes_session_events && !slot.observing_events.swap(true, Ordering::Relaxed) {
            let (channel, events) = self.session_events.subscribe(session);
            let entry_index = self.entry_index;
            let weak = Arc::downgrade(&slot);
            let life = slot.lifecycle.clone();
            tokio::spawn(async move {
                let _channel = channel;
                observe_events(weak, life, events, entry_index).await;
            });
        }
        drop(serial);
        Ok(slot)
    }

    async fn load_session_vm(
        self: &Arc<Self>,
        host: &dyn HostClient,
        mcp: Value,
        lifecycle: &CancellationToken,
    ) -> Result<Vm> {
        for _ in 0..3 {
            let initial = RuntimeSnapshot {
                mcp: mcp.clone(),
                storage: self.storage_snapshot(Some(host)).await?,
                ..Default::default()
            };
            let cancellation = lifecycle.child_token();
            let program = self.clone();
            let (vm, pending) = WORKERS
                .run(move || {
                    let vm = program.load(
                        &initial,
                        Instant::now() + SCRIPT_TIMEOUT,
                        Some(cancellation),
                        None,
                    )?;
                    let pending = if program.uses_storage {
                        vm.storage_startup()?
                    } else {
                        Vec::new()
                    };
                    Ok::<_, String>((vm, pending))
                })
                .await
                .map_err(|error| PluginError::internal_error(&error))?
                .map_err(PluginError::internal)?;
            let mut conflict = false;
            for transaction in pending {
                if lifecycle.is_cancelled() {
                    return Err(PluginError::internal("Cordis module session ended"));
                }
                if self
                    .storage
                    .commit_records(host, transaction)
                    .await?
                    .is_null()
                {
                    conflict = true;
                    break;
                }
            }
            if !conflict {
                return Ok(vm);
            }
        }
        Err(PluginError::internal(
            "Cordis records changed while opening the module",
        ))
    }

    pub(in crate::agent_plugin) fn end_session(&self, session: i64) {
        let slot = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&session);
        if let Some(slot) = slot {
            slot.lifecycle.cancel();
        }
    }

    pub(super) async fn evaluate_tool_when(
        self: &Arc<Self>,
        session: i64,
        host: &dyn HostClient,
        tool_name: &str,
    ) -> bool {
        let existing = {
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            sessions
                .get(&session)
                .filter(|slot| !slot.lifecycle.is_cancelled())
                .cloned()
        };
        let slot = match existing {
            Some(slot) => {
                // Re-entrant tool_definition checks while this module is already
                // running a foreground command/tool must not self-deadlock on
                // `slot.serial`; reuse the last observed visibility instead.
                if slot.serial.try_lock().is_err() {
                    return slot
                        .tool_visibility
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .get(tool_name)
                        .copied()
                        .unwrap_or(true);
                }
                slot
            }
            None => match self.start_session(session, host).await {
                Ok(slot) => slot,
                Err(_) => return false,
            },
        };
        let Ok(_serial) = slot.serial.try_lock() else {
            return slot
                .tool_visibility
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(tool_name)
                .copied()
                .unwrap_or(true);
        };
        let storage = match self.storage_snapshot(Some(host)).await {
            Ok(storage) => storage,
            Err(_) => return false,
        };
        let Ok(input) = serde_json::to_vec(&serde_json::json!({
            "name": tool_name,
            "sessionId": session,
        })) else {
            return false;
        };
        let snapshot = RuntimeSnapshot {
            storage,
            ..Default::default()
        };
        let cancel = slot.lifecycle.child_token();
        let worker_slot = Arc::clone(&slot);
        let active = WORKERS
            .run(move || {
                let mut vm = worker_slot
                    .vm
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let result = vm
                    .as_ref()
                    .ok_or_else(|| "Cordis module session ended".to_owned())?
                    .run::<bool>("evaluateToolWhen", &input, &snapshot, cancel, None);
                if result.is_err() {
                    vm.take();
                    worker_slot.lifecycle.cancel();
                }
                result.and_then(|value| value)
            })
            .await
            .ok()
            .and_then(|res| res.ok())
            .unwrap_or(false);
        slot.tool_visibility
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(tool_name.to_owned(), active);
        active
    }
}

impl RetainedModule {
    async fn update_status(
        self: &Arc<Self>,
        snapshot: Value,
        cancellation: CancellationToken,
    ) -> Result<()> {
        let slot = Arc::clone(self);
        WORKERS
            .run(move || {
                let vm = slot.vm.lock().unwrap_or_else(|error| error.into_inner());
                vm.as_ref()
                    .ok_or_else(|| "Cordis module session ended".to_owned())?
                    .update_status(&snapshot, cancellation)
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .map_err(PluginError::internal)
    }

    async fn session_event(
        self: &Arc<Self>,
        event: Event,
        cancellation: CancellationToken,
    ) -> Result<()> {
        let slot = self.clone();
        WORKERS
            .run(move || {
                let vm = slot.vm.lock().unwrap_or_else(|error| error.into_inner());
                vm.as_ref()
                    .ok_or_else(|| "Cordis module session ended".to_owned())?
                    .session_event(&event, cancellation)
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .map_err(PluginError::internal)
    }

    pub(super) async fn provided_skills(
        self: Arc<Self>,
        program: &Program,
        cwd: &Path,
        deadline: Instant,
        host: &dyn HostClient,
    ) -> Result<skills::Snapshot> {
        let _serial = self.serial.lock().await;
        let storage = program.storage_snapshot(Some(host)).await?;
        let slot = Arc::clone(&self);
        let root = program.root.clone();
        let cwd = cwd.to_owned();
        let cancel = self.lifecycle.child_token();
        let _stop = cancel.clone().drop_guard();
        WORKERS
            .run(move || {
                let vm = slot.vm.lock().unwrap_or_else(|error| error.into_inner());
                let vm = vm
                    .as_ref()
                    .ok_or_else(|| "Cordis module session ended".to_owned())?;
                vm.skill_snapshot(&root, &cwd, deadline, Some(cancel), Some(&storage))
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .map_err(PluginError::internal)
    }

    pub(super) async fn run<T: serde::de::DeserializeOwned + Send + 'static>(
        self: Arc<Self>,
        method: &'static str,
        input: Vec<u8>,
        snapshot: impl std::future::Future<Output = Result<RuntimeSnapshot>>,
        cancellation: CancellationToken,
        exchange: Option<WorkerHostBridge>,
    ) -> Result<T> {
        let serial = self.serial.lock().await;
        let snapshot = snapshot.await?;
        let slot = Arc::clone(&self);
        let result = WORKERS
            .run(move || {
                let mut vm = slot.vm.lock().unwrap_or_else(|error| error.into_inner());
                let result = vm
                    .as_ref()
                    .ok_or_else(|| "Cordis module session ended".to_owned())?
                    .run(method, &input, &snapshot, cancellation, exchange);
                if result.is_err() {
                    vm.take();
                    slot.lifecycle.cancel();
                }
                result.and_then(|result| result)
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .map_err(PluginError::internal);
        drop(serial);
        result
    }
}

async fn observe_subagents(
    slot: Weak<RetainedModule>,
    runs: Arc<subagents::Runs>,
    cancellation: CancellationToken,
) {
    loop {
        runs.ready().await;
        let Some(slot) = slot.upgrade() else { break };
        let _serial = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            guard = slot.serial.lock() => guard,
        };
        let worker = slot.clone();
        let runs = runs.clone();
        let lifetime = cancellation.clone();
        let result = WORKERS.run(move || {
            let mut vm = worker.vm.lock().unwrap_or_else(|error| error.into_inner());
            let Some(module) = vm.as_ref() else { return Ok(()) };
            let result = module.advance_subagents(&runs, lifetime);
            if result.is_err() {
                vm.take();
                worker.lifecycle.cancel();
            }
            result
        }).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(%error, "Cordis subagent event delivery ended");
                break;
            }
            Err(error) => {
                tracing::debug!(%error, "Cordis subagent event worker ended");
                break;
            }
        }
    }
}

async fn observe_jobs(
    slot: Weak<RetainedModule>,
    cancellation: CancellationToken,
    jobs: Arc<VmJobs>,
) {
    let mut delay: Option<Duration> = None;
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            _ = jobs.wake.notified() => {}
            _ = tokio::time::sleep(delay.unwrap_or_default()), if delay.is_some() => {}
        }
        let Some(slot) = slot.upgrade() else {
            break;
        };
        let _serial = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            guard = slot.serial.lock() => guard,
        };
        let worker = slot.clone();
        let life = cancellation.clone();
        let next = WORKERS
            .run(move || {
                let mut vm = worker.vm.lock().unwrap_or_else(|error| error.into_inner());
                let result = vm
                    .as_ref()
                    .ok_or_else(|| "Cordis module session ended".to_owned())?
                    .advance_jobs(&life);
                if result.is_err() {
                    vm.take();
                    worker.lifecycle.cancel();
                }
                result
            })
            .await;
        match next {
            Ok(Ok(next)) => delay = next,
            Ok(Err(error)) => {
                tracing::debug!(%error, "Cordis job pump ended");
                break;
            }
            Err(error) => {
                slot.lifecycle.cancel();
                tracing::warn!(%error, "Cordis job pump worker stopped");
                break;
            }
        }
    }
}

async fn forward_job_events(
    jobs: Arc<VmJobs>,
    cancellation: CancellationToken,
    mut events: broadcast::Receiver<Arc<CordisJobEvent>>,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            event = events.recv() => match event {
                Ok(event) => jobs.queue_event(event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "Cordis job event listener lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
}

async fn observe_session(
    slot: Weak<RetainedModule>,
    cancellation: CancellationToken,
    mut snapshots: watch::Receiver<Value>,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            changed = snapshots.changed() => if changed.is_err() { break },
        }
        let Some(session) = slot.upgrade() else {
            break;
        };
        let _serial = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            guard = session.serial.lock() => guard,
        };
        let update = snapshots.borrow_and_update().clone();
        if session
            .update_status(update, cancellation.clone())
            .await
            .is_err()
        {
            tracing::warn!("a Cordis MCP status listener could not process the update");
            break;
        }
    }
    cancellation.cancel();
}

async fn observe_events(
    slot: Weak<RetainedModule>,
    cancellation: CancellationToken,
    mut events: broadcast::Receiver<Event>,
    entry_index: usize,
) {
    loop {
        let event = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            event = events.recv() => match event {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "Cordis session event listener lagged");
                    continue;
                }
            },
        };
        if event.originates_from(entry_index) {
            continue;
        }
        let Some(session) = slot.upgrade() else {
            break;
        };
        let _serial = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            guard = session.serial.lock() => guard,
        };
        if let Err(error) = session.session_event(event, cancellation.clone()).await {
            tracing::warn!(%error, "a Cordis session event could not be delivered");
        }
    }
}
