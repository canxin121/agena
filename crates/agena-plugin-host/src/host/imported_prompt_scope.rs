use std::{future::Future, sync::Arc};

use agena_plugin_sdk::PluginKey;
use tokio::sync::{Mutex, OwnedMutexGuard};

const MAX_NESTED_SUBMISSIONS: usize = 16;

#[derive(Clone)]
struct Caller {
    plugin: PluginKey,
    session: i64,
    hook: usize,
    depth: usize,
}

/// Internal scope carried through the in-process hook transport. A nested
/// prompt can use its caller's admission lock while it waits for receipt.
#[derive(Clone, Default)]
pub struct ImportedPromptScope {
    admission: Option<Arc<OwnedMutexGuard<()>>>,
    caller: Option<Caller>,
}

tokio::task_local! {
    static SCOPE: ImportedPromptScope;
}

impl ImportedPromptScope {
    pub fn current() -> Self {
        SCOPE.try_with(Clone::clone).unwrap_or_default()
    }

    pub(crate) fn capture() -> Option<Self> {
        SCOPE.try_with(Clone::clone).ok()
    }

    pub async fn run<F: Future>(self, future: F) -> F::Output {
        SCOPE.scope(self, future).await
    }

    pub async fn acquire(mut self, lock: &Arc<Mutex<()>>) -> Self {
        if !self
            .admission
            .as_ref()
            .is_some_and(|guard| Arc::ptr_eq(OwnedMutexGuard::mutex(guard), lock))
        {
            self.admission = Some(Arc::new(lock.clone().lock_owned().await));
        }
        self
    }

    pub(crate) fn from_plugin(&self) -> bool {
        self.caller.is_some()
    }

    pub(crate) fn skip_hook(&self, plugin: &PluginKey, session: i64) -> Option<usize> {
        self.caller
            .as_ref()
            .filter(|caller| caller.plugin == *plugin && caller.session == session)
            .map(|caller| caller.hook)
    }

    pub(crate) fn with_caller(
        mut self,
        plugin: PluginKey,
        session: i64,
        hook: usize,
    ) -> Option<Self> {
        let depth = self.caller.as_ref().map_or(1, |caller| caller.depth + 1);
        if depth > MAX_NESTED_SUBMISSIONS {
            return None;
        }
        self.caller = Some(Caller {
            plugin,
            session,
            hook,
            depth,
        });
        Some(self)
    }
}
