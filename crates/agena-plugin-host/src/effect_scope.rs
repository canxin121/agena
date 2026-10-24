//! Plugin-owned effect scopes inspired by Cordis `Fiber` ownership.
//!
//! Every host registration belongs to exactly one plugin generation. Effects
//! register synchronously so a resource can never become visible without an
//! owner; disposal is asynchronous so shutdown can wait for accepted work and
//! async cleanup. Explicit removal uses `release()` to mark the effect disposed
//! without re-running its disposer, preventing an old generation from deleting
//! a replacement resource.

use portable_atomic::AtomicU64;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::FutureExt as _;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::sdk::PluginKey;

static NEXT_SCOPE_GENERATION: AtomicU64 = AtomicU64::new(1);

pub type PluginEffectFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;
type SyncDisposer = Box<dyn FnOnce() -> Result<(), String> + Send + 'static>;
type AsyncDisposer = Box<dyn FnOnce() -> PluginEffectFuture + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginEffectScopeState {
    Active,
    Disposing,
    Disposed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginEffectState {
    Active,
    Disposing,
    Disposed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEffectDescriptor {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub registered_at_ms: u64,
    pub state: PluginEffectState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEffectScopeInspect {
    pub plugin_id: PluginKey,
    pub generation: u64,
    pub lifecycle: PluginEffectScopeState,
    pub accepting: bool,
    pub active_leases: usize,
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<PluginEffectDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEffectDisposeReport {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Stable identity returned by an effect registration. Resource-specific APIs
/// generally retain only `(kind,label)`, but the handle is useful to generic
/// registries and inspectors without exposing the disposer itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEffectHandle {
    pub plugin_id: PluginKey,
    pub generation: u64,
    pub id: u64,
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEffectScopeError {
    Closed,
}

impl std::fmt::Display for PluginEffectScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => f.write_str("plugin effect scope is no longer accepting effects"),
        }
    }
}

impl std::error::Error for PluginEffectScopeError {}

enum EffectDisposer {
    Sync(SyncDisposer),
    Async(AsyncDisposer),
}

struct OwnedEffect {
    descriptor: PluginEffectDescriptor,
    disposer: Option<EffectDisposer>,
}

struct ScopeData {
    lifecycle: PluginEffectScopeState,
    live: Vec<OwnedEffect>,
    completed: Vec<PluginEffectDescriptor>,
    errors: Vec<String>,
    dispose_started: bool,
    dispose_finished: bool,
}

pub struct PluginEffectScope {
    plugin_id: PluginKey,
    generation: u64,
    accepting: AtomicBool,
    active_leases: AtomicUsize,
    next_effect_id: AtomicU64,
    cancellation: CancellationToken,
    idle: Notify,
    disposed: Notify,
    data: Mutex<ScopeData>,
}

impl std::fmt::Debug for PluginEffectScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginEffectScope")
            .field("plugin_id", &self.plugin_id)
            .field("generation", &self.generation)
            .field("lifecycle", &self.state())
            .field("active_leases", &self.active_leases())
            .finish_non_exhaustive()
    }
}

impl PluginEffectScope {
    /// Create an already-active generation. Preparation attaches a scoped host
    /// client immediately, so callbacks need a valid generation before
    /// `meta/init`; activation blockers dispose that exact generation later.
    pub fn new(plugin_id: PluginKey) -> Arc<Self> {
        Arc::new(Self {
            plugin_id,
            generation: NEXT_SCOPE_GENERATION.fetch_add(1, Ordering::AcqRel),
            accepting: AtomicBool::new(true),
            active_leases: AtomicUsize::new(0),
            next_effect_id: AtomicU64::new(1),
            cancellation: CancellationToken::new(),
            idle: Notify::new(),
            disposed: Notify::new(),
            data: Mutex::new(ScopeData {
                lifecycle: PluginEffectScopeState::Active,
                live: Vec::new(),
                completed: Vec::new(),
                errors: Vec::new(),
                dispose_started: false,
                dispose_finished: false,
            }),
        })
    }

    pub fn plugin_id(&self) -> &PluginKey {
        &self.plugin_id
    }

    /// Alias used by generic owner-aware registries.
    pub fn owner(&self) -> &PluginKey {
        self.plugin_id()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn state(&self) -> PluginEffectScopeState {
        self.data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .lifecycle
    }

    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }

    pub fn active_leases(&self) -> usize {
        self.active_leases.load(Ordering::Acquire)
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn inspect(&self) -> PluginEffectScopeInspect {
        let data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut effects = data.completed.clone();
        effects.extend(data.live.iter().map(|effect| effect.descriptor.clone()));
        effects.sort_by_key(|effect| effect.id);
        PluginEffectScopeInspect {
            plugin_id: self.plugin_id.clone(),
            generation: self.generation,
            lifecycle: data.lifecycle,
            accepting: self.is_accepting(),
            active_leases: self.active_leases(),
            cancelled: self.cancellation.is_cancelled(),
            effects,
            errors: data.errors.clone(),
        }
    }

    pub fn own_sync<F>(
        self: &Arc<Self>,
        kind: impl Into<String>,
        label: impl Into<String>,
        disposer: F,
    ) -> Result<PluginEffectHandle, PluginEffectScopeError>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        self.register(kind, label, EffectDisposer::Sync(Box::new(disposer)), false)
    }

    pub fn own_async<F, Fut>(
        self: &Arc<Self>,
        kind: impl Into<String>,
        label: impl Into<String>,
        disposer: F,
    ) -> Result<PluginEffectHandle, PluginEffectScopeError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.register(
            kind,
            label,
            EffectDisposer::Async(Box::new(move || Box::pin(disposer()))),
            false,
        )
    }

    pub(crate) fn replace_sync<F>(
        self: &Arc<Self>,
        kind: impl Into<String>,
        label: impl Into<String>,
        disposer: F,
    ) -> Result<PluginEffectHandle, PluginEffectScopeError>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        self.register(kind, label, EffectDisposer::Sync(Box::new(disposer)), true)
    }

    pub fn own_child(
        self: &Arc<Self>,
        child: Arc<PluginEffectScope>,
    ) -> Result<PluginEffectHandle, PluginEffectScopeError> {
        let label = format!("{}#{}", child.plugin_id(), child.generation());
        self.own_async("child_scope", label, move || async move {
            let report = child.dispose().await;
            if report.errors.is_empty() {
                Ok(())
            } else {
                Err(report.errors.join("; "))
            }
        })
    }

    fn register(
        self: &Arc<Self>,
        kind: impl Into<String>,
        label: impl Into<String>,
        disposer: EffectDisposer,
        replace: bool,
    ) -> Result<PluginEffectHandle, PluginEffectScopeError> {
        if !self.is_accepting() {
            return Err(PluginEffectScopeError::Closed);
        }
        let kind = kind.into();
        let label = label.into();
        let id = self.next_effect_id.fetch_add(1, Ordering::AcqRel);
        let descriptor = PluginEffectDescriptor {
            id,
            kind: kind.clone(),
            label: label.clone(),
            registered_at_ms: unix_ms(),
            state: PluginEffectState::Active,
            error: None,
        };
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.is_accepting() || data.dispose_started {
            return Err(PluginEffectScopeError::Closed);
        }
        // Retire the previous disposer only after the replacement has been
        // admitted under the same lock. Rejection retains all old ownership.
        if replace
            && let Some(index) = data.live.iter().rposition(|effect| {
                effect.descriptor.kind == kind && effect.descriptor.label == label
            })
        {
            let mut previous = data.live.remove(index);
            previous.descriptor.state = PluginEffectState::Disposed;
            previous.disposer = None;
            data.completed.push(previous.descriptor);
        }
        data.live.push(OwnedEffect {
            descriptor,
            disposer: Some(disposer),
        });
        Ok(PluginEffectHandle {
            plugin_id: self.plugin_id.clone(),
            generation: self.generation,
            id,
            kind,
            label,
        })
    }

    /// Mark an explicitly removed/replaced resource as disposed without running
    /// its stored disposer. This prevents the old disposer from deleting the
    /// replacement resource during plugin shutdown.
    pub fn release(&self, kind: &str, label: &str) -> bool {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(index) = data
            .live
            .iter()
            .rposition(|effect| effect.descriptor.kind == kind && effect.descriptor.label == label)
        else {
            return false;
        };
        let mut effect = data.live.remove(index);
        effect.descriptor.state = PluginEffectState::Disposed;
        effect.disposer = None;
        data.completed.push(effect.descriptor);
        true
    }

    /// Release one exact registration token. Generation and effect id prevent
    /// an old manual handle from releasing a newer replacement with the same
    /// `(kind,label)`.
    pub fn release_handle(&self, handle: &PluginEffectHandle) -> bool {
        if handle.plugin_id != self.plugin_id || handle.generation != self.generation {
            return false;
        }
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(index) = data
            .live
            .iter()
            .position(|effect| effect.descriptor.id == handle.id)
        else {
            return false;
        };
        let mut effect = data.live.remove(index);
        effect.descriptor.state = PluginEffectState::Disposed;
        effect.disposer = None;
        data.completed.push(effect.descriptor);
        true
    }

    pub fn lease(self: &Arc<Self>) -> Result<PluginEffectLease, PluginEffectScopeError> {
        if !self.is_accepting() {
            return Err(PluginEffectScopeError::Closed);
        }
        self.active_leases.fetch_add(1, Ordering::AcqRel);
        if !self.is_accepting() {
            if self.active_leases.fetch_sub(1, Ordering::AcqRel) == 1 {
                self.idle.notify_waiters();
            }
            return Err(PluginEffectScopeError::Closed);
        }
        Ok(PluginEffectLease {
            scope: Arc::clone(self),
        })
    }

    /// Stop callbacks before snapshotting registrations for a host handoff.
    /// Disposers remain intact until `dispose` is called.
    pub async fn quiesce(&self) {
        self.stop_admission();
        self.wait_idle().await;
    }

    pub(crate) fn stop_admission(&self) {
        self.accepting.store(false, Ordering::Release);
        self.cancellation.cancel();
    }

    /// Close admission, wait for accepted leases, then run live disposers in
    /// reverse registration order. Concurrent callers share one terminal
    /// report and effects never re-enter the live stack after disposal.
    pub async fn dispose(self: &Arc<Self>) -> PluginEffectDisposeReport {
        let leader = {
            let mut data = self
                .data
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if data.dispose_finished {
                return PluginEffectDisposeReport {
                    generation: self.generation,
                    errors: data.errors.clone(),
                };
            }
            if data.dispose_started {
                false
            } else {
                data.dispose_started = true;
                data.lifecycle = PluginEffectScopeState::Disposing;
                self.accepting.store(false, Ordering::Release);
                self.cancellation.cancel();
                true
            }
        };

        if leader {
            // Cleanup owns its progress. Cancelling any caller only abandons
            // that caller's wait, never a half-run disposer or its followers.
            let scope = Arc::clone(self);
            tokio::spawn(async move {
                scope.finish_disposal().await;
            });
        }
        loop {
            let notified = self.disposed.notified();
            if let Some(report) = self.finished_report() {
                return report;
            }
            notified.await;
        }
    }

    async fn finish_disposal(&self) {
        self.wait_idle().await;
        loop {
            let next = {
                let mut data = self
                    .data
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                data.live.pop().map(|mut effect| {
                    effect.descriptor.state = PluginEffectState::Disposing;
                    effect
                })
            };
            let Some(mut effect) = next else {
                break;
            };
            let invoke = async {
                match effect.disposer.take() {
                    Some(EffectDisposer::Sync(disposer)) => disposer(),
                    Some(EffectDisposer::Async(disposer)) => disposer().await,
                    None => Ok(()),
                }
            };
            let result = std::panic::AssertUnwindSafe(invoke)
                .catch_unwind()
                .await
                .unwrap_or_else(|payload| {
                    let message = payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| payload.downcast_ref::<&str>().copied())
                        .unwrap_or("non-string panic payload");
                    Err(format!("effect disposer panicked: {message}"))
                });
            let mut data = self
                .data
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match result {
                Ok(()) => effect.descriptor.state = PluginEffectState::Disposed,
                Err(error) => {
                    effect.descriptor.state = PluginEffectState::Failed;
                    effect.descriptor.error = Some(error.clone());
                    data.errors.push(format!(
                        "{} `{}`: {error}",
                        effect.descriptor.kind, effect.descriptor.label
                    ));
                }
            }
            data.completed.push(effect.descriptor);
        }

        {
            let mut data = self
                .data
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            data.dispose_finished = true;
            data.lifecycle = if data.errors.is_empty() {
                PluginEffectScopeState::Disposed
            } else {
                PluginEffectScopeState::Failed
            };
        }
        self.disposed.notify_waiters();
    }

    fn finished_report(&self) -> Option<PluginEffectDisposeReport> {
        let data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.dispose_finished.then(|| PluginEffectDisposeReport {
            generation: self.generation,
            errors: data.errors.clone(),
        })
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self.active_leases() == 0 {
                return;
            }
            notified.await;
        }
    }
}

pub struct PluginEffectLease {
    scope: Arc<PluginEffectScope>,
}

impl Drop for PluginEffectLease {
    fn drop(&mut self) {
        if self.scope.active_leases.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.scope.idle.notify_waiters();
        }
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
