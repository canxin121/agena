//! HTTP callback generations share a plugin instance, never a mutable request owner.
//!
//! The binding header selects only a destination previously registered by the
//! authenticated instance's lifecycle RPCs. It is not a URL or an authority token.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, RwLock};

use axum::http::HeaderMap;
use tokio::sync::OwnedRwLockReadGuard;
use tokio_util::sync::CancellationToken;

use super::{HttpCallbackBinding, PluginError, PluginErrorKind};

/// Opaque callback generation selected by a host-owned HTTP transport wrapper.
/// The same validated identifier must accompany initialization and its requests.
pub const BINDING_HEADER: &str = "x-agena-plugin-binding";

const ACTIVE_CAPACITY: usize = 256;
// Receipts make successful retire retries truthful without retaining secrets.
// An unknown or evicted ID fails closed; idempotence is limited to this window.
const RETIRED_CAPACITY: usize = 256;

/// Register a new callback generation without stopping the retained plugin.
/// Registration immediately selects the new process-only default. It does not
/// claim atomic runtime publication; retiring a failed candidate restores the
/// latest surviving default. Explicitly bound requests never follow that default.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpCallbackFork {
    pub expected_id: String,
    pub expected: HttpCallbackBinding,
    pub next_id: String,
    pub next: HttpCallbackBinding,
}

/// Retire one callback generation after its host stops issuing new requests.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpCallbackRetire {
    pub binding_id: String,
    /// Roll back a possibly uncommitted fork. Unknown IDs are fenced against a
    /// late commit instead of being represented as successfully retired objects.
    #[serde(default)]
    pub abort_uncommitted: bool,
}

/// Only the final retired binding owns physical plugin shutdown.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpCallbackRetired {
    pub last: bool,
    /// A later instance already completed this owner's physical shutdown.
    /// This is reported only with retained evidence for this exact binding ID.
    #[serde(default)]
    pub instance_stopped: bool,
    /// Only a rollback fence was recorded; no registered binding was retired.
    #[serde(default)]
    pub fenced_uncommitted: bool,
}

pub(super) fn identifier(value: &str) -> crate::Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(PluginError::invalid_params(
            "invalid HTTP callback binding identifier",
        ));
    }
    Ok(())
}

pub(super) fn header(headers: &HeaderMap) -> crate::Result<Option<String>> {
    let Some(value) = headers.get(BINDING_HEADER) else {
        return Ok(None);
    };
    if headers.get_all(BINDING_HEADER).iter().count() != 1 {
        return Err(PluginError::invalid_params(
            "duplicate HTTP callback binding header",
        ));
    }
    let value = value
        .to_str()
        .map_err(|error| PluginError::invalid_params_error(&error))?;
    identifier(value)?;
    Ok(Some(value.to_owned()))
}

fn unavailable() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::HostUnavailable,
        "HTTP callback binding is unknown, retired, or stopped",
    )
}

tokio::task_local! {
    static SHUTDOWN_CALLBACKS: ();
}

/// Only the driver's physical shutdown hook may use the final stopped binding.
/// A plugin-created task does not inherit this permission.
pub(super) async fn shutdown_scope<F: std::future::Future>(future: F) -> F::Output {
    SHUTDOWN_CALLBACKS.scope((), future).await
}

pub(super) fn in_shutdown_scope() -> bool {
    SHUTDOWN_CALLBACKS.try_with(|_| ()).is_ok()
}

struct Slot {
    id: Option<String>,
    destination: HttpCallbackBinding,
    stop: CancellationToken,
    operations: Arc<tokio::sync::RwLock<()>>,
}

struct State {
    // Registration order also defines the default for process-only callbacks.
    active: Vec<Arc<Slot>>,
    final_slot: Option<Arc<Slot>>,
    retired: VecDeque<(String, bool)>,
    aborted: HashSet<String>,
    forks_closed: bool,
}

pub(super) struct CallbackBindings {
    state: RwLock<State>,
    stop: CancellationToken,
}

#[derive(Clone)]
pub(super) struct BindingLease {
    pub(super) stop: CancellationToken,
    pub(super) destination: HttpCallbackBinding,
    _operation: Arc<OwnedRwLockReadGuard<()>>,
    shutdown: bool,
}

impl BindingLease {
    pub(super) async fn run<T>(
        &self,
        future: impl std::future::Future<Output = crate::Result<T>>,
    ) -> crate::Result<T> {
        if self.shutdown {
            return future.await;
        }
        tokio::select! {
            biased;
            _ = self.stop.cancelled() => Err(unavailable()),
            result = future => result,
        }
    }
}

impl CallbackBindings {
    pub(super) fn new(
        id: Option<String>,
        destination: HttpCallbackBinding,
        stop: CancellationToken,
    ) -> crate::Result<Self> {
        if let Some(id) = id.as_deref() {
            identifier(id)?;
        }
        destination.validate()?;
        let slot = Arc::new(Slot {
            id,
            destination,
            stop: stop.child_token(),
            operations: Arc::new(tokio::sync::RwLock::new(())),
        });
        Ok(Self {
            state: RwLock::new(State {
                active: vec![slot],
                final_slot: None,
                retired: VecDeque::new(),
                aborted: HashSet::new(),
                forks_closed: false,
            }),
            stop,
        })
    }

    fn read(&self) -> crate::Result<std::sync::RwLockReadGuard<'_, State>> {
        self.state
            .read()
            .map_err(|_| PluginError::internal("HTTP callback binding lock is poisoned"))
    }

    fn write(&self) -> crate::Result<std::sync::RwLockWriteGuard<'_, State>> {
        self.state
            .write()
            .map_err(|_| PluginError::internal("HTTP callback binding lock is poisoned"))
    }

    fn select(&self, id: Option<&str>, shutdown: bool) -> crate::Result<Arc<Slot>> {
        let state = self.read()?;
        let found = match id {
            Some(id) => state
                .active
                .iter()
                .find(|slot| slot.id.as_deref() == Some(id)),
            None => state.active.last(),
        };
        let found = found.or_else(|| {
            shutdown
                .then_some(state.final_slot.as_ref())
                .flatten()
                .filter(|slot| id.is_none() || slot.id.as_deref() == id)
        });
        found.cloned().ok_or_else(unavailable)
    }

    async fn lease(slot: Arc<Slot>, shutdown: bool) -> crate::Result<BindingLease> {
        let operation = if shutdown {
            slot.operations.clone().read_owned().await
        } else {
            tokio::select! {
                biased;
                _ = slot.stop.cancelled() => return Err(unavailable()),
                guard = slot.operations.clone().read_owned() => guard,
            }
        };
        if !shutdown && slot.stop.is_cancelled() {
            return Err(unavailable());
        }
        Ok(BindingLease {
            stop: slot.stop.clone(),
            destination: slot.destination.clone(),
            _operation: Arc::new(operation),
            shutdown,
        })
    }

    pub(super) async fn admit(&self, id: Option<&str>) -> crate::Result<BindingLease> {
        let slot = self.select(id, false)?;
        if slot.id.as_deref() != id {
            return Err(PluginError::invalid_params(
                "HTTP request requires its registered callback binding header",
            ));
        }
        Self::lease(slot, false).await
    }

    pub(super) async fn callback(&self, id: Option<&str>) -> crate::Result<BindingLease> {
        let shutdown = in_shutdown_scope();
        Self::lease(self.select(id, shutdown)?, shutdown).await
    }

    pub(super) fn fork(&self, fork: HttpCallbackFork) -> crate::Result<()> {
        identifier(&fork.expected_id)?;
        identifier(&fork.next_id)?;
        fork.expected.validate()?;
        fork.next.validate()?;
        let mut state = self.write()?;
        if state.forks_closed {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "HTTP callback forks are closed after the abort fence capacity was exhausted",
            ));
        }
        let previous = state.active.last().ok_or_else(unavailable)?;
        if previous.stop.is_cancelled()
            || previous.id.as_deref() != Some(fork.expected_id.as_str())
            || previous.destination != fork.expected
        {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "HTTP callback binding changed before fork",
            ));
        }
        if state
            .active
            .iter()
            .any(|slot| slot.id.as_deref() == Some(fork.next_id.as_str()))
            || state.aborted.contains(&fork.next_id)
            || state.retired.iter().any(|(id, _)| id == &fork.next_id)
            || state
                .final_slot
                .as_ref()
                .is_some_and(|slot| slot.id.as_deref() == Some(fork.next_id.as_str()))
        {
            return Err(PluginError::invalid_params(
                "HTTP callback binding identifier is already used",
            ));
        }
        if state.active.len() >= ACTIVE_CAPACITY {
            return Err(PluginError::internal(
                "HTTP callback binding capacity exhausted",
            ));
        }
        state.active.push(Arc::new(Slot {
            id: Some(fork.next_id),
            destination: fork.next,
            stop: self.stop.child_token(),
            operations: Arc::new(tokio::sync::RwLock::new(())),
        }));
        Ok(())
    }

    // Lifecycle control serializes this operation with fork/rebind/retire.
    // If cancelled while draining, the slot remains stopped and a retry drains it.
    pub(super) async fn retire(&self, id: &str) -> crate::Result<HttpCallbackRetired> {
        identifier(id)?;
        let slot = {
            let state = self.read()?;
            if let Some((_, last)) = state.retired.iter().find(|(retired, _)| retired == id) {
                return Ok(HttpCallbackRetired {
                    last: *last,
                    instance_stopped: false,
                    fenced_uncommitted: false,
                });
            }
            state
                .active
                .iter()
                .find(|slot| slot.id.as_deref() == Some(id))
                .cloned()
                .ok_or_else(unavailable)?
        };
        slot.stop.cancel();
        let _quiet = slot.operations.write().await;
        let mut state = self.write()?;
        state.active.retain(|current| !Arc::ptr_eq(current, &slot));
        let last = state.active.is_empty();
        if last {
            // Keep the real destination until physical shutdown finishes.
            // Ordinary admissions and callbacks cannot select this slot.
            state.final_slot = Some(slot.clone());
        }
        state.retired.push_back((id.to_owned(), last));
        while state.retired.len() > RETIRED_CAPACITY {
            state.retired.pop_front();
        }
        Ok(HttpCallbackRetired {
            last,
            instance_stopped: false,
            fenced_uncommitted: false,
        })
    }

    pub(super) async fn abort_fork(&self, id: &str) -> crate::Result<HttpCallbackRetired> {
        identifier(id)?;
        let committed = {
            let state = self.read()?;
            state
                .active
                .iter()
                .any(|slot| slot.id.as_deref() == Some(id))
                || state.retired.iter().any(|(retired, _)| retired == id)
        };
        if committed {
            return self.retire(id).await;
        }
        let mut state = self.write()?;
        // Unlike retirement receipts, an abort fence cannot be evicted: a
        // delayed HTTP fork could then resurrect a failed candidate. Once the
        // bounded set is full, close only future forks for this instance.
        if state.aborted.len() < RETIRED_CAPACITY {
            state.aborted.insert(id.to_owned());
        } else if !state.aborted.contains(id) {
            state.forks_closed = true;
        }
        Ok(HttpCallbackRetired {
            last: false,
            instance_stopped: false,
            fenced_uncommitted: true,
        })
    }

    pub(super) fn rebind(
        &self,
        expected: &HttpCallbackBinding,
        next: HttpCallbackBinding,
    ) -> crate::Result<()> {
        expected.validate()?;
        next.validate()?;
        let mut state = self.write()?;
        if state.active.len() != 1 {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "destructive HTTP callback rebind requires a single active binding",
            ));
        }
        let current = &state.active[0];
        if current.stop.is_cancelled() || current.destination != *expected {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "HTTP callback binding changed before handoff",
            ));
        }
        state.active[0] = Arc::new(Slot {
            id: current.id.clone(),
            destination: next,
            stop: current.stop.clone(),
            operations: current.operations.clone(),
        });
        Ok(())
    }

    pub(super) fn clear_after_shutdown(&self) -> crate::Result<Vec<String>> {
        let mut state = self.write()?;
        let mut ids: Vec<String> = state
            .active
            .iter()
            .chain(state.final_slot.iter())
            .filter_map(|slot| slot.id.clone())
            .collect();
        ids.extend(state.retired.iter().map(|(id, _)| id.clone()));
        ids.sort_unstable();
        ids.dedup();
        state.active.clear();
        state.final_slot = None;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests;
