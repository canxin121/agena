//! Process-wide reentrancy protection for one logical invocation chain.
//!
//! The guard is deliberately scoped by stable invocation identifiers instead
//! of a Tokio worker thread: an async callback can migrate between workers,
//! and nested runtimes may execute on a different thread.

use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct InvocationScope {
    session_id: i64,
    call_id: i64,
}

static ACTIVE: LazyLock<Mutex<HashMap<InvocationScope, HashSet<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// RAII lease held while a target participates in one logical invocation.
pub struct InvocationGuard {
    scope: InvocationScope,
    target: String,
}

impl Drop for InvocationGuard {
    fn drop(&mut self) {
        let Ok(mut active) = ACTIVE.lock() else {
            return;
        };
        let Some(targets) = active.get_mut(&self.scope) else {
            return;
        };
        targets.remove(self.target.as_str());
        if targets.is_empty() {
            active.remove(&self.scope);
        }
    }
}

/// Atomically enter a target for one logical session invocation.
///
/// Returns `None` only when that exact invocation already has the target on
/// its call chain. The caller must retain the returned guard for the duration
/// of the callback.
pub fn try_enter_invocation(
    session_id: i64,
    call_id: i64,
    target: impl Into<String>,
) -> Option<InvocationGuard> {
    let scope = InvocationScope {
        session_id,
        call_id,
    };
    let target = target.into();
    let mut active = ACTIVE.lock().ok()?;
    let targets = active.entry(scope).or_default();
    if !targets.insert(target.clone()) {
        return None;
    }
    Some(InvocationGuard { scope, target })
}

#[cfg(test)]
fn is_active(session_id: i64, call_id: i64, target: &str) -> bool {
    let scope = InvocationScope {
        session_id,
        call_id,
    };
    ACTIVE
        .lock()
        .ok()
        .and_then(|active| active.get(&scope).cloned())
        .is_some_and(|targets| targets.contains(target))
}

#[cfg(test)]
mod tests {
    use super::{is_active, try_enter_invocation};

    #[test]
    fn reentrancy_is_scoped_to_one_session_call_chain() {
        let first =
            try_enter_invocation(9_001, 101, "example.target").expect("enter first invocation");
        assert!(is_active(9_001, 101, "example.target"));

        assert!(try_enter_invocation(9_001, 101, "example.target").is_none());
        let other_call = try_enter_invocation(9_001, 102, "example.target")
            .expect("same target is valid for a distinct call");
        let other_session = try_enter_invocation(9_002, 101, "example.target")
            .expect("same target is valid for a distinct session");

        drop(other_call);
        drop(other_session);
        drop(first);
        assert!(!is_active(9_001, 101, "example.target"));
        assert!(try_enter_invocation(9_001, 101, "example.target").is_some());
    }

    #[test]
    fn guard_cleanup_is_not_bound_to_the_entry_thread() {
        let guard = try_enter_invocation(9_003, 103, "example.target").expect("enter invocation");
        assert!(is_active(9_003, 103, "example.target"));

        std::thread::spawn(move || drop(guard))
            .join()
            .expect("drop guard from another thread");

        assert!(!is_active(9_003, 103, "example.target"));
        assert!(try_enter_invocation(9_003, 103, "example.target").is_some());
    }
}
