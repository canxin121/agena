use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use agena_plugin_sdk::ImportedInstructionFile;

use super::{Event, HookScope};

#[derive(Default)]
pub(super) struct ScopedHooks {
    sessions: Mutex<BTreeMap<i64, Arc<Mutex<SessionScopes>>>>,
    forks: Mutex<BTreeMap<(i64, String), ForkScope>>,
}

struct ForkScope {
    hook: HookScope,
    child: Option<i64>,
}

#[derive(Default)]
struct SessionScopes {
    active: BTreeSet<HookScope>,
    claimed: BTreeSet<(Event, usize)>,
    instructions: BTreeMap<PathBuf, String>,
}

pub(super) struct OnceHook {
    scopes: Arc<Mutex<SessionScopes>>,
    key: (Event, usize),
    success: bool,
}

pub(super) struct PendingFork<'a> {
    scopes: &'a ScopedHooks,
    key: (i64, String),
}

impl ScopedHooks {
    pub(super) fn new_instructions(
        &self,
        session: i64,
        compacted: bool,
        mut files: Vec<ImportedInstructionFile>,
    ) -> Vec<ImportedInstructionFile> {
        let scopes = self.session(session);
        let mut scopes = scopes.lock().unwrap_or_else(|error| error.into_inner());
        if compacted {
            scopes.instructions.clear();
        }
        files.retain(|file| {
            let identity = file.canonical_path.as_ref().unwrap_or(&file.path);
            if scopes.instructions.get(identity) == Some(&file.content_hash) {
                return false;
            }
            scopes
                .instructions
                .insert(identity.clone(), file.content_hash.clone());
            true
        });
        files
    }

    pub(super) fn prepare_fork(
        &self,
        parent: i64,
        name: &str,
        scope: HookScope,
    ) -> PendingFork<'_> {
        let key = (parent, name.to_owned());
        self.forks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                key.clone(),
                ForkScope {
                    hook: scope,
                    child: None,
                },
            );
        PendingFork { scopes: self, key }
    }

    pub(super) fn activate_fork(&self, parent: i64, child: i64, name: &str) {
        let mut forks = self.forks.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(scope) = forks.get_mut(&(parent, name.to_owned()))
            && scope.child.is_none()
        {
            self.activate(child, scope.hook.clone());
            scope.child = Some(child);
        }
    }

    pub(super) fn activate(&self, session: i64, scope: HookScope) {
        self.session(session)
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(scope);
    }

    pub(super) fn is_active(&self, session: i64, scope: &HookScope) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&session)
            .is_some_and(|scopes| {
                scopes
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .active
                    .contains(scope)
            })
    }

    pub(super) fn claim_once(&self, session: i64, event: Event, index: usize) -> Option<OnceHook> {
        let scopes = self.session(session);
        let key = (event, index);
        let claimed = scopes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .claimed
            .insert(key);
        claimed.then(|| OnceHook {
            scopes,
            key,
            success: false,
        })
    }

    pub(super) fn end_session(&self, session: i64) {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&session);
    }

    fn session(&self, session: i64) -> Arc<Mutex<SessionScopes>> {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(session)
            .or_default()
            .clone()
    }
}

impl Drop for PendingFork<'_> {
    fn drop(&mut self) {
        let scope = self
            .scopes
            .forks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.key);
        if let Some(ForkScope {
            hook,
            child: Some(child),
        }) = scope
            && let Some(session) = self
                .scopes
                .sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&child)
        {
            session
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .remove(&hook);
        }
    }
}

impl OnceHook {
    pub(super) fn complete(mut self) {
        self.success = true;
    }
}

impl Drop for OnceHook {
    fn drop(&mut self) {
        if !self.success {
            self.scopes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .claimed
                .remove(&self.key);
        }
    }
}

#[cfg(test)]
#[test]
fn fork_hooks_unbind_from_the_child_when_invocation_finishes() {
    let scopes = ScopedHooks::default();
    let local = HookScope::Command("inspect".into());
    let background = HookScope::Command("background".into());
    scopes.activate(3, background.clone());
    {
        let _invocation = scopes.prepare_fork(1, "fork", local.clone());
        scopes.activate_fork(2, 4, "fork");
        scopes.activate_fork(1, 3, "fork");
        assert!(scopes.is_active(3, &local));
        assert!(!scopes.is_active(1, &local));
        assert!(!scopes.is_active(4, &local));
    }
    assert!(!scopes.is_active(3, &local));
    assert!(scopes.is_active(3, &background));
}
