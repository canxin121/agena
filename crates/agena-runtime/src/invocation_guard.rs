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

/// Maximum nesting depth of one session's plugin-to-plugin delegations.
///
/// Hooks can delegate to other plugins' tools, and those tools' hooks can
/// delegate again. A depth bound rejects a cycle (A→B→A→…) without rejecting
/// legitimate concurrent delegations to the same plugin, which a set-membership
/// guard would do.
pub const MAX_DELEGATION_DEPTH: usize = 8;

static DELEGATION_DEPTH: LazyLock<Mutex<HashMap<i64, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// RAII lease counting one live session-scoped delegation.
pub struct DelegationGuard {
    session_id: i64,
}

impl Drop for DelegationGuard {
    fn drop(&mut self) {
        let mut depths = delegation_depths("release delegation guard");
        if let Some(depth) = depths.get_mut(&self.session_id) {
            *depth = depth.saturating_sub(1);
            if *depth == 0 {
                depths.remove(&self.session_id);
            }
        }
    }
}

fn delegation_depths(context: &str) -> std::sync::MutexGuard<'static, HashMap<i64, usize>> {
    match DELEGATION_DEPTH.lock() {
        Ok(depths) => depths,
        Err(error) => {
            tracing::error!(
                operation = context,
                error = %error,
                "recovering poisoned delegation depth registry"
            );
            error.into_inner()
        }
    }
}

/// Enter one session-scoped delegation.
///
/// Returns `None` when this session already nests [`MAX_DELEGATION_DEPTH`]
/// delegations, which can only happen through a delegation cycle.
pub fn try_enter_delegation(session_id: i64) -> Option<DelegationGuard> {
    let mut depths = delegation_depths("enter delegation guard");
    let depth = depths.entry(session_id).or_insert(0);
    if *depth >= MAX_DELEGATION_DEPTH {
        return None;
    }
    *depth += 1;
    Some(DelegationGuard { session_id })
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
