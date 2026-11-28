//! Authority attached by the host to an exact session lifecycle dispatch.

use std::future::Future;

use crate::sdk::rpc::method;

tokio::task_local! {
    static SESSION_LIFECYCLE: SessionLifecycleScope;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionLifecycleScope {
    Disposal(i64),
    Shutdown(i64),
    WorktreeRemoval(i64),
}

impl SessionLifecycleScope {
    pub(crate) fn for_dispatch(method: &str) -> Option<Self> {
        let scope = SESSION_LIFECYCLE.try_with(|scope| *scope).ok()?;
        match (method, scope) {
            (method::HOOK_SESSION_END, Self::Disposal(_) | Self::Shutdown(_))
            | (method::HOOK_WORKTREE_REMOVE, Self::WorktreeRemoval(_)) => Some(scope),
            _ => None,
        }
    }

    pub(crate) async fn run<T>(self, task: impl Future<Output = T>) -> T {
        SESSION_LIFECYCLE.scope(self, task).await
    }

    fn is_active(self) -> bool {
        SESSION_LIFECYCLE
            .try_with(|scope| *scope == self)
            .unwrap_or(false)
    }
}

/// The native callback belongs to the session being explicitly closed.
pub fn is_session_disposal(session: i64) -> bool {
    session > 0 && SessionLifecycleScope::Disposal(session).is_active()
}

/// Allow authorized cleanup during shutdown without reopening admission.
pub fn is_shutdown_session_end(session: i64) -> bool {
    session > 0 && SessionLifecycleScope::Shutdown(session).is_active()
}

/// Use the session's recorded source workspace for worktree removal.
pub fn is_worktree_removal(session: i64) -> bool {
    session > 0 && SessionLifecycleScope::WorktreeRemoval(session).is_active()
}
