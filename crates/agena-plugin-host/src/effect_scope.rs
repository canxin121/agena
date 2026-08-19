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
        self.register(kind, label, EffectDisposer::Sync(Box::new(disposer)))
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
        )
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

    /// Close admission, wait for accepted leases, then run live disposers in
    /// reverse registration order. Concurrent callers share one terminal
    /// report and effects never re-enter the live stack after disposal.
    pub async fn dispose(&self) -> PluginEffectDisposeReport {
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

        if !leader {
            loop {
                let notified = self.disposed.notified();
                if let Some(report) = self.finished_report() {
                    return report;
                }
                notified.await;
            }
        }

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
            let result = match effect.disposer.take() {
                Some(EffectDisposer::Sync(disposer)) => disposer(),
                Some(EffectDisposer::Async(disposer)) => disposer().await,
                None => Ok(()),
            };
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

        let report = {
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
            PluginEffectDisposeReport {
                generation: self.generation,
                errors: data.errors.clone(),
            }
        };
        self.disposed.notify_waiters();
        report
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;

    fn scope() -> Arc<PluginEffectScope> {
        PluginEffectScope::new("example.plugin".parse().unwrap())
    }

    #[tokio::test]
    async fn effects_dispose_once_in_reverse_order_and_remain_inspectable() {
        let scope = scope();
        let events = Arc::new(StdMutex::new(Vec::new()));
        for label in ["first", "second", "third"] {
            let events = Arc::clone(&events);
            scope
                .own_async("test", label, move || async move {
                    events.lock().unwrap().push(label);
                    Ok(())
                })
                .unwrap();
        }
        let report = scope.dispose().await;
        assert!(report.errors.is_empty());
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["third", "second", "first"]
        );
        assert_eq!(scope.state(), PluginEffectScopeState::Disposed);
        assert!(
            scope
                .inspect()
                .effects
                .iter()
                .all(|effect| effect.state == PluginEffectState::Disposed)
        );
        let again = scope.dispose().await;
        assert!(again.errors.is_empty());
        assert_eq!(
            events.lock().unwrap().len(),
            3,
            "disposers must never run twice"
        );
    }

    #[tokio::test]
    async fn release_marks_explicit_removal_without_running_disposer() {
        let scope = scope();
        let runs = Arc::new(AtomicUsize::new(0));
        let runs_effect = Arc::clone(&runs);
        scope
            .own_sync("theme", "paper", move || {
                runs_effect.fetch_add(1, Ordering::AcqRel);
                Ok(())
            })
            .unwrap();
        assert!(scope.release("theme", "paper"));
        assert!(!scope.release("theme", "paper"));
        assert_eq!(runs.load(Ordering::Acquire), 0);
        assert_eq!(
            scope.inspect().effects[0].state,
            PluginEffectState::Disposed
        );
        scope.dispose().await;
        assert_eq!(runs.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn disposer_failure_does_not_abandon_later_cleanup() {
        let scope = scope();
        let events = Arc::new(StdMutex::new(Vec::new()));
        let ok = Arc::clone(&events);
        scope
            .own_sync("test", "ok", move || {
                ok.lock().unwrap().push("ok");
                Ok(())
            })
            .unwrap();
        let failed = Arc::clone(&events);
        scope
            .own_sync("test", "failed", move || {
                failed.lock().unwrap().push("failed");
                Err("boom".into())
            })
            .unwrap();
        let report = scope.dispose().await;
        assert_eq!(events.lock().unwrap().as_slice(), ["failed", "ok"]);
        assert!(report.errors.iter().any(|error| error.contains("boom")));
        assert_eq!(scope.state(), PluginEffectScopeState::Failed);
    }

    #[tokio::test]
    async fn disposal_closes_admission_and_waits_for_accepted_leases() {
        let scope = scope();
        let lease = scope.lease().unwrap();
        let stopping = {
            let scope = Arc::clone(&scope);
            tokio::spawn(async move { scope.dispose().await })
        };
        while scope.is_accepting() {
            tokio::task::yield_now().await;
        }
        assert!(matches!(scope.lease(), Err(PluginEffectScopeError::Closed)));
        assert!(!stopping.is_finished());
        drop(lease);
        let report = stopping.await.unwrap();
        assert!(report.errors.is_empty());
        assert_eq!(scope.active_leases(), 0);
    }

    #[tokio::test]
    async fn concurrent_dispose_shares_completion() {
        let scope = scope();
        let runs = Arc::new(AtomicUsize::new(0));
        let effect_runs = Arc::clone(&runs);
        scope
            .own_async("test", "once", move || async move {
                effect_runs.fetch_add(1, Ordering::AcqRel);
                tokio::task::yield_now().await;
                Ok(())
            })
            .unwrap();
        let left = {
            let scope = Arc::clone(&scope);
            tokio::spawn(async move { scope.dispose().await })
        };
        let right = {
            let scope = Arc::clone(&scope);
            tokio::spawn(async move { scope.dispose().await })
        };
        assert!(left.await.unwrap().errors.is_empty());
        assert!(right.await.unwrap().errors.is_empty());
        assert_eq!(runs.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn child_scope_is_owned_as_an_async_effect() {
        let parent = scope();
        let child = PluginEffectScope::new("example.child".parse().unwrap());
        let runs = Arc::new(AtomicUsize::new(0));
        let child_runs = Arc::clone(&runs);
        child
            .own_sync("child", "resource", move || {
                child_runs.fetch_add(1, Ordering::AcqRel);
                Ok(())
            })
            .unwrap();
        parent.own_child(Arc::clone(&child)).unwrap();
        parent.dispose().await;
        assert_eq!(runs.load(Ordering::Acquire), 1);
        assert_eq!(child.state(), PluginEffectScopeState::Disposed);
    }
}
