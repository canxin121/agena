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

fn active_invocations(
    context: &str,
) -> std::sync::MutexGuard<'static, HashMap<InvocationScope, HashSet<String>>> {
    match ACTIVE.lock() {
        Ok(active) => active,
        Err(error) => {
            tracing::error!(
                operation = context,
                error = %error,
                "recovering poisoned invocation guard registry"
            );
            error.into_inner()
        }
    }
}

/// RAII lease held while a target participates in one logical invocation.
pub struct InvocationGuard {
    scope: InvocationScope,
    target: String,
}

impl Drop for InvocationGuard {
    fn drop(&mut self) {
        let mut active = active_invocations("release invocation guard");
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
    let mut active = active_invocations("enter invocation guard");
    let targets = active.entry(scope).or_default();
    if !targets.insert(target.clone()) {
        return None;
    }
    Some(InvocationGuard { scope, target })
}
