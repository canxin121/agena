use super::*;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(5);

// Generic keys can do work while being cloned. Use that public extension point
// to observe or pause an unlocked registration, without production test hooks
// or probabilistic sleeps. Equality and ordering retain ordinary string keys.
struct ObservedKey {
    name: String,
    on_clone: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ObservedKey {
    fn plain() -> Self {
        Self {
            name: "tool".into(),
            on_clone: None,
        }
    }
}

impl Clone for ObservedKey {
    fn clone(&self) -> Self {
        if let Some(observe) = &self.on_clone {
            observe();
        }
        Self {
            name: self.name.clone(),
            on_clone: self.on_clone.clone(),
        }
    }
}

impl std::fmt::Debug for ObservedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ObservedKey").field(&self.name).finish()
    }
}

impl PartialEq for ObservedKey {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for ObservedKey {}
impl PartialOrd for ObservedKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ObservedKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

type Registry = ScopedRegistry<ObservedKey, String>;
type Registration = ScopedRegistryRegistration<ObservedKey, String>;

fn owner() -> Arc<PluginEffectScope> {
    PluginEffectScope::new("test.registry-atomicity".parse().unwrap())
}

fn session() -> PluginScopeKey {
    PluginScopeKey::session(41)
}

fn active_effects(owner: &PluginEffectScope) -> usize {
    owner
        .inspect()
        .effects
        .into_iter()
        .filter(|effect| effect.state == crate::effect_scope::PluginEffectState::Active)
        .count()
}

fn value(registry: &Registry, scope: Option<&PluginScopeKey>) -> Option<(u64, String)> {
    registry
        .resolve(scope, &ObservedKey::plain())
        .map(|entry| (entry.generation, entry.value))
}

fn write(
    registry: &Registry,
    owner: &Arc<PluginEffectScope>,
    scope: Option<PluginScopeKey>,
    key: ObservedKey,
    replace: bool,
) -> Result<Registration, ScopedRegistryError> {
    if replace {
        registry
            .replace_owned(owner, scope, key, "candidate".into(), "host.tool", "tool")
            .map(|(registration, _)| registration)
    } else {
        registry.register(owner, scope, key, "candidate".into(), "tool")
    }
}

async fn rejection_preserves_visibility_and_registry_state(replace: bool) {
    for scope in [None, Some(session())] {
        let registry = Registry::new();
        let owner = owner();
        if replace {
            registry
                .register(
                    &owner,
                    scope.clone(),
                    ObservedKey::plain(),
                    "original".into(),
                    "tool",
                )
                .unwrap();
        }
        owner.quiesce().await;
        let original = value(&registry, scope.as_ref());
        let before = {
            let data = registry.lock();
            (
                data.next_generation,
                data.known_scopes.clone(),
                data.parents.clone(),
            )
        };
        let observations = Arc::new(Mutex::new(Vec::new()));
        let weak_registry = Arc::downgrade(&registry.inner);
        let observer = observations.clone();
        let layer = scope
            .clone()
            .map(|scope| ScopedRegistryLayer::Scope { scope })
            .unwrap_or(ScopedRegistryLayer::Global);
        let key = ObservedKey {
            name: "tool".into(),
            on_clone: Some(Arc::new(move || {
                if let Some(registry) = weak_registry.upgrade()
                    && let Ok(data) = registry.try_lock()
                {
                    let current = entry_for_layer(&data, &layer, &ObservedKey::plain())
                        .map(|entry| (entry.generation, entry.value.clone()));
                    observer.lock().unwrap().push(current);
                }
            })),
        };
        let result = write(&registry, &owner, scope.clone(), key, replace);
        let actual = value(&registry, scope.as_ref());
        let after = {
            let data = registry.lock();
            (
                data.next_generation,
                data.known_scopes.clone(),
                data.parents.clone(),
            )
        };
        let observations = observations.lock().unwrap().clone();
        owner.dispose().await;
        assert!(matches!(
            result,
            Err(ScopedRegistryError::Owner(PluginEffectScopeError::Closed))
        ));
        assert_eq!(actual, original);
        assert!(
            observations.iter().all(|observed| observed == &original),
            "a rejected registration exposed an unadmitted value: {observations:?}"
        );
        assert_eq!(
            after, before,
            "rejection changed generation or scope metadata"
        );
        assert!(registry.inspect().is_empty());
    }
}

#[tokio::test]
async fn rejected_registration_never_exposes_an_unadmitted_value() {
    rejection_preserves_visibility_and_registry_state(false).await;
}

#[tokio::test]
async fn rejected_replacement_never_exposes_an_unadmitted_value() {
    rejection_preserves_visibility_and_registry_state(true).await;
}

struct PausedWrite {
    entered: tokio::sync::oneshot::Receiver<()>,
    resume: std::sync::mpsc::Sender<()>,
    task: tokio::task::JoinHandle<Result<Registration, ScopedRegistryError>>,
}

fn pause_write(registry: &Registry, owner: Arc<PluginEffectScope>, replace: bool) -> PausedWrite {
    let (entered, checkpoint) = tokio::sync::oneshot::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    let gate = Mutex::new(Some((entered, paused)));
    let weak_registry = Arc::downgrade(&registry.inner);
    let key = ObservedKey {
        name: "tool".into(),
        on_clone: Some(Arc::new(move || {
            let Some(registry) = weak_registry.upgrade() else {
                return;
            };
            // Never block while the product owns its registry lock. Pausing
            // before mutation is also valid after admission/publication merge.
            let Ok(data) = registry.try_lock() else {
                return;
            };
            drop(data);
            let Some((entered, paused)) = gate.lock().unwrap().take() else {
                return;
            };
            let _ = entered.send(());
            paused
                .recv_timeout(DEADLINE)
                .expect("test must resume registration");
        })),
    };
    let registry = registry.clone();
    let task = tokio::task::spawn_blocking(move || {
        write(&registry, &owner, Some(session()), key, replace)
    });
    PausedWrite {
        entered: checkpoint,
        resume,
        task,
    }
}

#[tokio::test]
async fn failed_replacement_cannot_restore_old_value_over_new_owner() {
    let registry = Registry::new();
    let previous = owner();
    let successor = owner();
    registry
        .register(
            &previous,
            Some(session()),
            ObservedKey::plain(),
            "original".into(),
            "tool",
        )
        .unwrap();
    let paused = pause_write(&registry, previous.clone(), true);
    tokio::time::timeout(DEADLINE, paused.entered)
        .await
        .unwrap()
        .unwrap();
    let quiesce = tokio::spawn({
        let previous = previous.clone();
        async move { previous.quiesce().await }
    });
    tokio::time::timeout(DEADLINE, async {
        while previous.is_accepting() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    registry.clear_scope_tree(&session());
    registry
        .register(
            &successor,
            Some(session()),
            ObservedKey::plain(),
            "successor".into(),
            "tool",
        )
        .unwrap();
    let expected = value(&registry, Some(&session()));
    paused.resume.send(()).unwrap();
    let result = tokio::time::timeout(DEADLINE, paused.task)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(DEADLINE, quiesce)
        .await
        .unwrap()
        .unwrap();
    let actual = value(&registry, Some(&session()));
    previous.dispose().await;
    let after_old_cleanup = value(&registry, Some(&session()));
    successor.dispose().await;
    assert!(result.is_err());
    assert_eq!(actual, expected, "failed rollback overwrote the successor");
    assert_eq!(after_old_cleanup, expected);
    assert!(registry.inspect().is_empty());
}

#[tokio::test]
async fn cleared_pending_registration_cannot_succeed_without_its_entry() {
    let registry = Registry::new();
    let previous = owner();
    let successor = owner();
    let paused = pause_write(&registry, previous.clone(), false);
    tokio::time::timeout(DEADLINE, paused.entered)
        .await
        .unwrap()
        .unwrap();
    registry.clear_scope_tree(&session());
    registry
        .register(
            &successor,
            Some(session()),
            ObservedKey::plain(),
            "successor".into(),
            "tool",
        )
        .unwrap();
    let expected = value(&registry, Some(&session()));
    paused.resume.send(()).unwrap();
    let result = tokio::time::timeout(DEADLINE, paused.task)
        .await
        .unwrap()
        .unwrap();
    let previous_effects = active_effects(&previous);
    let actual = value(&registry, Some(&session()));
    previous.dispose().await;
    successor.dispose().await;
    assert!(
        result.is_err(),
        "registration reported success for an absent entry"
    );
    assert_eq!(previous_effects, 0);
    assert_eq!(actual, expected);
    assert!(registry.inspect().is_empty());
}

#[tokio::test]
async fn remove_during_registration_cannot_leave_a_detached_effect() {
    let registry = Registry::new();
    let owner = owner();
    let paused = pause_write(&registry, owner.clone(), false);
    tokio::time::timeout(DEADLINE, paused.entered)
        .await
        .unwrap()
        .unwrap();
    let removed = registry.remove_owned(&owner, Some(&session()), &ObservedKey::plain());
    paused.resume.send(()).unwrap();
    let result = tokio::time::timeout(DEADLINE, paused.task)
        .await
        .unwrap()
        .unwrap();
    let actual = value(&registry, Some(&session()));
    let effects = active_effects(&owner);
    owner.dispose().await;
    assert!(result.is_ok());
    if removed.is_some() {
        assert!(actual.is_none());
        assert_eq!(effects, 0, "removal lost the registration's cleanup handle");
    } else {
        assert_eq!(actual.unwrap().1, "candidate");
        assert_eq!(effects, 1);
    }
    assert!(registry.inspect().is_empty());
}

#[tokio::test]
async fn exhausted_registry_generations_reject_without_changing_ownership() {
    for replace in [false, true] {
        let registry = Registry::new();
        let owner = owner();
        if replace {
            registry
                .register(
                    &owner,
                    Some(session()),
                    ObservedKey::plain(),
                    "original".into(),
                    "tool",
                )
                .unwrap();
        }
        registry.lock().next_generation = u64::MAX;
        let before = value(&registry, Some(&session()));
        let effects_before = active_effects(&owner);
        let result = write(
            &registry,
            &owner,
            Some(session()),
            ObservedKey::plain(),
            replace,
        );
        let after = value(&registry, Some(&session()));
        let effects_after = active_effects(&owner);
        owner.dispose().await;
        assert!(
            matches!(result, Err(ScopedRegistryError::GenerationExhausted)),
            "exhaustion must not allow generation identity reuse"
        );
        assert_eq!(after, before);
        assert_eq!(effects_after, effects_before);
        assert!(registry.inspect().is_empty());
    }
}

#[test]
fn rejected_parent_cycles_do_not_allocate_scope_metadata() {
    let registry = Registry::new();
    for index in 0..256 {
        let scope = PluginScopeKey::session(index);
        assert!(matches!(
            registry.set_parent(scope.clone(), scope),
            Err(ScopedRegistryError::ParentCycle { .. })
        ));
    }
    let data = registry.lock();
    assert!(data.parents.is_empty());
    assert!(
        data.known_scopes.is_empty(),
        "invalid parent requests retained {} scopes",
        data.known_scopes.len()
    );
}

#[tokio::test]
async fn duplicate_and_cross_owner_replacements_preserve_exact_ownership() {
    for scope in [None, Some(session())] {
        let registry = Registry::new();
        let original = owner();
        let same_plugin = owner();
        let other_plugin = PluginEffectScope::new("test.other-owner".parse().unwrap());
        let first = registry
            .register(
                &original,
                scope.clone(),
                ObservedKey::plain(),
                "original".into(),
                "tool",
            )
            .unwrap();
        let expected = value(&registry, scope.as_ref());
        let next = registry.lock().next_generation;
        for (candidate, replace) in [
            (&original, false),
            (&same_plugin, true),
            (&other_plugin, true),
        ] {
            let result = write(
                &registry,
                candidate,
                scope.clone(),
                ObservedKey::plain(),
                replace,
            );
            assert!(matches!(
                result,
                Err(ScopedRegistryError::DuplicateEntry { .. })
            ));
            if !Arc::ptr_eq(candidate, &original) {
                assert!(
                    registry
                        .remove_owned(candidate, scope.as_ref(), &ObservedKey::plain())
                        .is_none()
                );
                assert!(registry.owned_entries(candidate).is_empty());
            }
        }
        let actual = value(&registry, scope.as_ref());
        let next_after = registry.lock().next_generation;
        let original_effects = active_effects(&original);
        let other_effects = active_effects(&same_plugin) + active_effects(&other_plugin);
        same_plugin.dispose().await;
        other_plugin.dispose().await;
        first.dispose().await.unwrap();
        original.dispose().await;
        assert_eq!(actual, expected);
        assert_eq!(next_after, next);
        assert_eq!(original_effects, 1);
        assert_eq!(other_effects, 0);
        assert!(registry.inspect().is_empty());
    }
}

#[tokio::test]
async fn concurrent_replacements_have_one_visible_entry_and_one_live_effect() {
    for scope in [None, Some(session())] {
        let registry = ScopedRegistry::<String, String>::new();
        let owner = owner();
        let first = registry
            .register(
                &owner,
                scope.clone(),
                "tool".into(),
                "original".into(),
                "tool",
            )
            .unwrap();
        let start = Arc::new(std::sync::Barrier::new(16));
        let tasks: Vec<_> = (0..16)
            .map(|index| {
                let (registry, owner, scope, start) = (
                    registry.clone(),
                    owner.clone(),
                    scope.clone(),
                    start.clone(),
                );
                tokio::task::spawn_blocking(move || {
                    start.wait();
                    let value = format!("replacement-{index}");
                    let (handle, replaced) = registry
                        .replace_owned(
                            &owner,
                            scope,
                            "tool".into(),
                            value.clone(),
                            "host.tool",
                            "tool",
                        )
                        .unwrap();
                    (handle, replaced.unwrap(), value)
                })
            })
            .collect();
        let mut results = Vec::new();
        for task in tasks {
            results.push(tokio::time::timeout(DEADLINE, task).await.unwrap().unwrap());
        }
        results.sort_by_key(|(handle, _, _)| handle.generation());
        let (latest_generation, latest_value) = {
            let (handle, _, value) = results.last().unwrap();
            (handle.generation(), value.clone())
        };
        let current = registry.resolve(scope.as_ref(), &"tool".into()).unwrap();
        let effects = active_effects(&owner);
        first.dispose().await.unwrap();
        let mut replaced_value = "original".to_string();
        for (index, (handle, previous, value)) in results.into_iter().enumerate() {
            assert_eq!(handle.generation(), index as u64 + 2);
            assert_eq!(previous.generation, index as u64 + 1);
            assert_eq!(previous.value, replaced_value);
            replaced_value = value;
            if handle.generation() != latest_generation {
                handle.dispose().await.unwrap();
            }
        }
        let after_stale_handles = registry.resolve(scope.as_ref(), &"tool".into()).unwrap();
        let effects_after = active_effects(&owner);
        owner.dispose().await;
        assert_eq!(current.generation, latest_generation);
        assert_eq!(current.value, latest_value);
        assert_eq!(after_stale_handles.value, latest_value);
        assert_eq!(effects, 1);
        assert_eq!(effects_after, 1);
        assert!(registry.inspect().is_empty());
    }
}

#[tokio::test]
async fn final_advancing_generation_cannot_be_reused_by_later_handles() {
    let registry = Registry::new();
    let owner = owner();
    registry.lock().next_generation = u64::MAX - 1;
    let last = registry
        .register(&owner, None, ObservedKey::plain(), "last".into(), "tool")
        .unwrap();
    assert_eq!(last.generation(), u64::MAX - 1);
    assert!(matches!(
        write(&registry, &owner, None, ObservedKey::plain(), true),
        Err(ScopedRegistryError::GenerationExhausted)
    ));
    assert_eq!(value(&registry, None), Some((u64::MAX - 1, "last".into())));
    last.dispose().await.unwrap();
    assert!(matches!(
        write(
            &registry,
            &owner,
            Some(session()),
            ObservedKey::plain(),
            false
        ),
        Err(ScopedRegistryError::GenerationExhausted)
    ));
    let effects = active_effects(&owner);
    let known = registry.lock().known_scopes.clone();
    owner.dispose().await;
    assert_eq!(effects, 0);
    assert!(known.is_empty());
    assert!(registry.inspect().is_empty());
}
