//! In-process execution port for the shared command/query contract.
//!
//! The application layer implements this port. Runtime and plugin transports
//! depend only on the request contract and public failures, so they never need
//! to depend on the application implementation or on an HTTP listener.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock, Weak},
};

use agena_failure::Failure;
use serde_json::Value;

use crate::{Command, Query};

/// Serialized `QueryResult` or `CommandResult`, including its result/data tag.
pub type SessionApiFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'a>>;

/// One business implementation shared by HTTP, native and remote plugins.
///
/// Calls run in the originating task, retaining its admitted plugin context,
/// cancellation and call-completion authority.
pub trait SessionApiBackend: Send + Sync {
    fn query(&self, query: Query) -> SessionApiFuture<'_>;
    fn command(&self, command: Command) -> SessionApiFuture<'_>;
}

/// Process-wide attachment retained across runtime snapshot reloads.
///
/// The application owns the backend. Runtime only stores a weak reference,
/// breaking the application -> runtime -> plugin -> application ownership loop.
#[derive(Clone, Default)]
pub struct SessionApiBackendSlot {
    backend: Arc<RwLock<Option<Weak<dyn SessionApiBackend>>>>,
}

impl SessionApiBackendSlot {
    pub fn install(&self, backend: &Arc<dyn SessionApiBackend>) {
        *self
            .backend
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::downgrade(backend));
    }

    /// Lease the backend for one operation without holding the registry lock.
    pub fn backend(&self) -> Option<Arc<dyn SessionApiBackend>> {
        self.backend
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Backend;

    impl SessionApiBackend for Backend {
        fn query(&self, _: Query) -> SessionApiFuture<'_> {
            Box::pin(async { Ok(Value::Null) })
        }

        fn command(&self, _: Command) -> SessionApiFuture<'_> {
            Box::pin(async { Ok(Value::Null) })
        }
    }

    #[test]
    fn attachment_is_shared_without_owning_the_application() {
        let slot = SessionApiBackendSlot::default();
        let next_generation = slot.clone();
        let backend: Arc<dyn SessionApiBackend> = Arc::new(Backend);
        slot.install(&backend);
        assert!(next_generation.backend().is_some());
        assert_eq!(Arc::strong_count(&backend), 1);
        drop(backend);
        assert!(slot.backend().is_none());
        assert!(next_generation.backend().is_none());
    }
}
