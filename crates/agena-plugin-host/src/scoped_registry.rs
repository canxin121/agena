//! Owner-aware scoped overlays for plugin capabilities.
//!
//! This is the host's common visibility primitive: global entries are visible
//! everywhere, ancestor overlays apply farthest-to-nearest, and the exact
//! scope wins last. Reads never create state. Every registration is owned by
//! the registering plugin generation's `PluginEffectScope`, so catalog and
//! lookup entries disappear with the exact owner generation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};

use crate::effect_scope::{PluginEffectHandle, PluginEffectScope, PluginEffectScopeError};
use crate::sdk::PluginKey;

const MAX_SCOPE_ID_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginScopeKey(String);

impl PluginScopeKey {
    pub fn session(session_id: i64) -> Self {
        Self(format!("session:{session_id}"))
    }

    pub fn new(value: impl Into<String>) -> Result<Self, ScopedRegistryError> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            return Err(ScopedRegistryError::InvalidScope(
                "scope id cannot be empty".into(),
            ));
        }

        if value.len() > MAX_SCOPE_ID_BYTES {
            return Err(ScopedRegistryError::InvalidScope(format!(
                "scope id exceeds {MAX_SCOPE_ID_BYTES} bytes"
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(ScopedRegistryError::InvalidScope(
                "scope id cannot contain control characters".into(),
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for PluginScopeKey {
    type Err = ScopedRegistryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for PluginScopeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopedRegistryError {
    InvalidScope(String),
    DuplicateEntry {
        scope: Option<PluginScopeKey>,
    },
    ParentCycle {
        scope: PluginScopeKey,
        parent: PluginScopeKey,
    },
    Owner(PluginEffectScopeError),
}

impl fmt::Display for ScopedRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScope(message) => f.write_str(message),
            Self::DuplicateEntry { scope: None } => {
                f.write_str("an entry with the same key already exists globally")
            }
            Self::DuplicateEntry { scope: Some(scope) } => write!(
                f,
                "an entry with the same key already exists in scope `{scope}`"
            ),
            Self::ParentCycle { scope, parent } => write!(
                f,
                "scope parent `{parent}` would create a cycle for `{scope}`"
            ),
            Self::Owner(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ScopedRegistryError {}

impl From<PluginEffectScopeError> for ScopedRegistryError {
    fn from(value: PluginEffectScopeError) -> Self {
        Self::Owner(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopedRegistryLayer {
    Global,
    Scope { scope: PluginScopeKey },
}

#[derive(Debug, Clone)]
pub struct ScopedRegistryValue<V> {
    pub owner: PluginKey,
    pub generation: u64,
    pub layer: ScopedRegistryLayer,
    pub value: V,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopedRegistryEntryDescriptor<K> {
    pub key: K,
    pub owner: PluginKey,
    pub generation: u64,
    pub layer: ScopedRegistryLayer,
}

#[derive(Clone)]
struct Entry<V> {
    owner: PluginKey,
    owner_generation: u64,
    owner_scope: std::sync::Weak<PluginEffectScope>,
    generation: u64,
    value: V,
    effect: Option<PluginEffectHandle>,
}

struct RegistryData<K, V> {
    global: BTreeMap<K, Entry<V>>,
    overlays: BTreeMap<PluginScopeKey, BTreeMap<K, Entry<V>>>,
    parents: BTreeMap<PluginScopeKey, PluginScopeKey>,
    known_scopes: BTreeSet<PluginScopeKey>,
    next_generation: u64,
}

impl<K, V> Default for RegistryData<K, V> {
    fn default() -> Self {
        Self {
            global: BTreeMap::new(),
            overlays: BTreeMap::new(),
            parents: BTreeMap::new(),
            known_scopes: BTreeSet::new(),
            next_generation: 1,
        }
    }
}

pub struct ScopedRegistry<K, V> {
    inner: Arc<Mutex<RegistryData<K, V>>>,
}

impl<K, V> Clone for ScopedRegistry<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K, V> Default for ScopedRegistry<K, V> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryData::default())),
        }
    }
}

impl<K, V> std::fmt::Debug for ScopedRegistry<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedRegistry").finish_non_exhaustive()
    }
}

impl<K, V> ScopedRegistry<K, V>
where
    K: Clone + Ord + Send + 'static,
    V: Clone + Send + 'static,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        owner: &Arc<PluginEffectScope>,
        scope: Option<PluginScopeKey>,
        key: K,
        value: V,
        label: impl Into<String>,
    ) -> Result<ScopedRegistryRegistration<K, V>, ScopedRegistryError> {
        self.register_with_effect_kind(owner, scope, key, value, "scoped_registry", label)
    }

    pub fn register_with_effect_kind(
        &self,
        owner: &Arc<PluginEffectScope>,
        scope: Option<PluginScopeKey>,
        key: K,
        value: V,
        effect_kind: &'static str,
        label: impl Into<String>,
    ) -> Result<ScopedRegistryRegistration<K, V>, ScopedRegistryError> {
        let label = label.into();
        let (generation, layer) = {
            let mut data = self.lock();
            if let Some(scope) = &scope {
                data.known_scopes.insert(scope.clone());
                if data
                    .overlays
                    .get(scope)
                    .is_some_and(|items| items.contains_key(&key))
                {
                    return Err(ScopedRegistryError::DuplicateEntry {
                        scope: Some(scope.clone()),
                    });
                }
            } else if data.global.contains_key(&key) {
                return Err(ScopedRegistryError::DuplicateEntry { scope: None });
            }
            let generation = data.next_generation;
            data.next_generation = data.next_generation.saturating_add(1);
            let entry = Entry {
                owner: owner.plugin_id().clone(),
                owner_generation: owner.generation(),
                owner_scope: Arc::downgrade(owner),
                generation,
                value,
                effect: None,
            };
            let layer = match &scope {
                Some(scope) => {
                    data.overlays
                        .entry(scope.clone())
                        .or_default()
                        .insert(key.clone(), entry);
                    ScopedRegistryLayer::Scope {
                        scope: scope.clone(),
                    }
                }
                None => {
                    data.global.insert(key.clone(), entry);
                    ScopedRegistryLayer::Global
                }
            };
            (generation, layer)
        };

        let registry = self.clone();
        let disposer_key = key.clone();
        let disposer_layer = layer.clone();
        let effect = match owner.own_sync(effect_kind, label, move || {
            registry.remove_exact(&disposer_layer, &disposer_key, generation);
            Ok(())
        }) {
            Ok(effect) => effect,
            Err(error) => {
                self.remove_exact(&layer, &key, generation);
                return Err(error.into());
            }
        };
        self.set_effect_handle(&layer, &key, generation, effect.clone());

        Ok(ScopedRegistryRegistration {
            registry: Arc::downgrade(&self.inner),
            key,
            layer,
            generation,
            owner: Arc::downgrade(owner),
            effect,
        })
    }

    /// Replace an entry in one exact layer when it is owned by the same plugin
    /// generation, otherwise preserve duplicate isolation. This is the scoped
    /// equivalent of updating a Cordis fiber-owned registration: visibility
    /// swaps atomically, the old disposer is released without running, and an
    /// old generation can never delete the replacement later.
    pub fn replace_owned(
        &self,
        owner: &Arc<PluginEffectScope>,
        scope: Option<PluginScopeKey>,
        key: K,
        value: V,
        effect_kind: &'static str,
        label: impl Into<String>,
    ) -> Result<
        (
            ScopedRegistryRegistration<K, V>,
            Option<ScopedRegistryValue<V>>,
        ),
        ScopedRegistryError,
    > {
        let label = label.into();
        let layer = scope
            .as_ref()
            .map(|scope| ScopedRegistryLayer::Scope {
                scope: scope.clone(),
            })
            .unwrap_or(ScopedRegistryLayer::Global);
        let (generation, replaced) = {
            let mut data = self.lock();
            if let Some(scope) = &scope {
                data.known_scopes.insert(scope.clone());
            }
            let existing = entry_for_layer(&data, &layer, &key).cloned();
            if let Some(existing) = existing.as_ref()
                && (existing.owner != *owner.plugin_id()
                    || existing.owner_generation != owner.generation())
            {
                return Err(ScopedRegistryError::DuplicateEntry {
                    scope: scope.clone(),
                });
            }
            let generation = data.next_generation;
            data.next_generation = data.next_generation.saturating_add(1);
            insert_entry(
                &mut data,
                &layer,
                key.clone(),
                Entry {
                    owner: owner.plugin_id().clone(),
                    owner_generation: owner.generation(),
                    owner_scope: Arc::downgrade(owner),
                    generation,
                    value,
                    effect: None,
                },
            );
            let replaced = existing
                .as_ref()
                .map(|entry| scoped_value(entry, layer.clone()));
            (generation, (existing, replaced))
        };

        let registry = self.clone();
        let disposer_key = key.clone();
        let disposer_layer = layer.clone();
        let effect = match owner.own_sync(effect_kind, label, move || {
            registry.remove_exact(&disposer_layer, &disposer_key, generation);
            Ok(())
        }) {
            Ok(effect) => effect,
            Err(error) => {
                let (old_entry, _) = replaced;
                let mut data = self.lock();
                remove_exact_inner(&mut data, &layer, &key, generation);
                if let Some(old_entry) = old_entry {
                    insert_entry(&mut data, &layer, key.clone(), old_entry);
                }
                return Err(error.into());
            }
        };
        self.set_effect_handle(&layer, &key, generation, effect.clone());
        if let Some(old_effect) = replaced.0.as_ref().and_then(|entry| entry.effect.as_ref()) {
            owner.release_handle(old_effect);
        }
        Ok((
            ScopedRegistryRegistration {
                registry: Arc::downgrade(&self.inner),
                key,
                layer,
                generation,
                owner: Arc::downgrade(owner),
                effect,
            },
            replaced.1,
        ))
    }

    /// Remove one exact-layer entry only when the caller owns that registration
    /// generation. The associated effect is released synchronously without
    /// running its disposer because the registry entry has already been removed.
    pub fn remove_owned(
        &self,
        owner: &Arc<PluginEffectScope>,
        scope: Option<&PluginScopeKey>,
        key: &K,
    ) -> Option<ScopedRegistryValue<V>> {
        let layer = scope
            .cloned()
            .map(|scope| ScopedRegistryLayer::Scope { scope })
            .unwrap_or(ScopedRegistryLayer::Global);
        let entry = {
            let mut data = self.lock();
            let existing = entry_for_layer(&data, &layer, key)?.clone();
            if existing.owner != *owner.plugin_id()
                || existing.owner_generation != owner.generation()
            {
                return None;
            }
            remove_exact_inner(&mut data, &layer, key, existing.generation);
            existing
        };
        if let Some(effect) = entry.effect.as_ref() {
            owner.release_handle(effect);
        }
        Some(scoped_value(&entry, layer))
    }

    /// Close one visibility scope and every descendant scope atomically.
    /// Registrations disappear before their effect handles are released, so
    /// no disposer can observe a half-closed layer or remove a later replacement.
    pub fn clear_scope_tree(&self, scope: &PluginScopeKey) -> Vec<ScopedRegistryValue<V>> {
        let removed = {
            let mut data = self.lock();
            let mut scopes = data
                .known_scopes
                .iter()
                .filter(|candidate| scope_is_descendant_or_same(&data.parents, candidate, scope))
                .cloned()
                .collect::<Vec<_>>();
            if !scopes.iter().any(|candidate| candidate == scope) {
                scopes.push(scope.clone());
            }
            scopes.sort();
            let mut removed = Vec::new();
            for current in &scopes {
                if let Some(entries) = data.overlays.remove(current) {
                    removed.extend(entries.into_values().map(|entry| {
                        (
                            entry,
                            ScopedRegistryLayer::Scope {
                                scope: current.clone(),
                            },
                        )
                    }));
                }
                data.known_scopes.remove(current);
                data.parents.remove(current);
            }
            data.parents
                .retain(|child, parent| !scopes.contains(child) && !scopes.contains(parent));
            removed
        };

        removed
            .into_iter()
            .map(|(entry, layer)| {
                if let (Some(owner), Some(effect)) =
                    (entry.owner_scope.upgrade(), entry.effect.as_ref())
                {
                    owner.release_handle(effect);
                }
                scoped_value(&entry, layer)
            })
            .collect()
    }

    pub fn resolve(
        &self,
        scope: Option<&PluginScopeKey>,
        key: &K,
    ) -> Option<ScopedRegistryValue<V>> {
        let data = self.lock();
        let mut resolved = data
            .global
            .get(key)
            .map(|entry| scoped_value(entry, ScopedRegistryLayer::Global));
        for scope in scope_chain(&data, scope) {
            if let Some(entry) = data.overlays.get(scope).and_then(|items| items.get(key)) {
                resolved = Some(scoped_value(
                    entry,
                    ScopedRegistryLayer::Scope {
                        scope: scope.clone(),
                    },
                ));
            }
        }
        resolved
    }

    pub fn visible(&self, scope: Option<&PluginScopeKey>) -> BTreeMap<K, ScopedRegistryValue<V>> {
        let data = self.lock();
        let mut visible = data
            .global
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    scoped_value(entry, ScopedRegistryLayer::Global),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for scope in scope_chain(&data, scope) {
            if let Some(entries) = data.overlays.get(scope) {
                for (key, entry) in entries {
                    visible.insert(
                        key.clone(),
                        scoped_value(
                            entry,
                            ScopedRegistryLayer::Scope {
                                scope: scope.clone(),
                            },
                        ),
                    );
                }
            }
        }
        visible
    }

    pub fn inspect(&self) -> Vec<ScopedRegistryEntryDescriptor<K>> {
        let data = self.lock();
        let mut entries = data
            .global
            .iter()
            .map(|(key, entry)| ScopedRegistryEntryDescriptor {
                key: key.clone(),
                owner: entry.owner.clone(),
                generation: entry.generation,
                layer: ScopedRegistryLayer::Global,
            })
            .collect::<Vec<_>>();
        for (scope, layer) in &data.overlays {
            entries.extend(
                layer
                    .iter()
                    .map(|(key, entry)| ScopedRegistryEntryDescriptor {
                        key: key.clone(),
                        owner: entry.owner.clone(),
                        generation: entry.generation,
                        layer: ScopedRegistryLayer::Scope {
                            scope: scope.clone(),
                        },
                    }),
            );
        }
        entries.sort_by_key(|entry| entry.generation);
        entries
    }

    pub fn parent(&self, scope: &PluginScopeKey) -> Option<PluginScopeKey> {
        self.lock().parents.get(scope).cloned()
    }

    pub fn set_parent(
        &self,
        scope: PluginScopeKey,
        parent: PluginScopeKey,
    ) -> Result<(), ScopedRegistryError> {
        let mut data = self.lock();
        data.known_scopes.insert(scope.clone());
        data.known_scopes.insert(parent.clone());
        if would_cycle(&data, &scope, &parent) {
            return Err(ScopedRegistryError::ParentCycle { scope, parent });
        }
        data.parents.insert(scope, parent);
        Ok(())
    }

    pub fn clear_parent(&self, scope: &PluginScopeKey) -> Option<PluginScopeKey> {
        let mut data = self.lock();
        let parent = data.parents.remove(scope);
        if !data.overlays.contains_key(scope) && !data.parents.values().any(|value| value == scope)
        {
            data.known_scopes.remove(scope);
        }
        parent
    }

    pub fn overlay_count(&self) -> usize {
        self.lock().overlays.len()
    }

    fn remove_exact(&self, layer: &ScopedRegistryLayer, key: &K, generation: u64) -> bool {
        remove_exact_inner(&mut self.lock(), layer, key, generation)
    }

    fn set_effect_handle(
        &self,
        layer: &ScopedRegistryLayer,
        key: &K,
        generation: u64,
        effect: PluginEffectHandle,
    ) {
        let mut data = self.lock();
        if let Some(entry) = entry_for_layer_mut(&mut data, layer, key)
            && entry.generation == generation
        {
            entry.effect = Some(effect);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryData<K, V>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct ScopedRegistryRegistration<K, V> {
    registry: Weak<Mutex<RegistryData<K, V>>>,
    key: K,
    layer: ScopedRegistryLayer,
    generation: u64,
    owner: Weak<PluginEffectScope>,
    effect: PluginEffectHandle,
}

impl<K, V> std::fmt::Debug for ScopedRegistryRegistration<K, V>
where
    K: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedRegistryRegistration")
            .field("key", &self.key)
            .field("layer", &self.layer)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl<K, V> ScopedRegistryRegistration<K, V>
where
    K: Clone + Ord,
{
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub async fn dispose(self) -> Result<(), ScopedRegistryError> {
        if let Some(registry) = self.registry.upgrade() {
            let mut data = registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            remove_exact_inner(&mut data, &self.layer, &self.key, self.generation);
        }
        if let Some(owner) = self.owner.upgrade() {
            owner.release_handle(&self.effect);
        }
        Ok(())
    }
}

fn scoped_value<V: Clone>(entry: &Entry<V>, layer: ScopedRegistryLayer) -> ScopedRegistryValue<V> {
    ScopedRegistryValue {
        owner: entry.owner.clone(),
        generation: entry.generation,
        layer,
        value: entry.value.clone(),
    }
}

fn entry_for_layer<'a, K, V>(
    data: &'a RegistryData<K, V>,
    layer: &ScopedRegistryLayer,
    key: &K,
) -> Option<&'a Entry<V>>
where
    K: Ord,
{
    match layer {
        ScopedRegistryLayer::Global => data.global.get(key),
        ScopedRegistryLayer::Scope { scope } => data.overlays.get(scope)?.get(key),
    }
}

fn entry_for_layer_mut<'a, K, V>(
    data: &'a mut RegistryData<K, V>,
    layer: &ScopedRegistryLayer,
    key: &K,
) -> Option<&'a mut Entry<V>>
where
    K: Ord,
{
    match layer {
        ScopedRegistryLayer::Global => data.global.get_mut(key),
        ScopedRegistryLayer::Scope { scope } => data.overlays.get_mut(scope)?.get_mut(key),
    }
}

fn insert_entry<K, V>(
    data: &mut RegistryData<K, V>,
    layer: &ScopedRegistryLayer,
    key: K,
    entry: Entry<V>,
) where
    K: Ord,
{
    match layer {
        ScopedRegistryLayer::Global => {
            data.global.insert(key, entry);
        }
        ScopedRegistryLayer::Scope { scope } => {
            data.known_scopes.insert(scope.clone());
            data.overlays
                .entry(scope.clone())
                .or_default()
                .insert(key, entry);
        }
    }
}

fn scope_is_descendant_or_same(
    parents: &BTreeMap<PluginScopeKey, PluginScopeKey>,
    candidate: &PluginScopeKey,
    ancestor: &PluginScopeKey,
) -> bool {
    if candidate == ancestor {
        return true;
    }
    let mut cursor = candidate;
    let mut seen = BTreeSet::new();
    while let Some(parent) = parents.get(cursor) {
        if !seen.insert(cursor.clone()) {
            return false;
        }
        if parent == ancestor {
            return true;
        }
        cursor = parent;
    }
    false
}

fn remove_exact_inner<K, V>(
    data: &mut RegistryData<K, V>,
    layer: &ScopedRegistryLayer,
    key: &K,
    generation: u64,
) -> bool
where
    K: Clone + Ord,
{
    match layer {
        ScopedRegistryLayer::Global => {
            if data
                .global
                .get(key)
                .is_some_and(|entry| entry.generation == generation)
            {
                data.global.remove(key);
                true
            } else {
                false
            }
        }
        ScopedRegistryLayer::Scope { scope } => {
            let matches = data
                .overlays
                .get(scope)
                .and_then(|items| items.get(key))
                .is_some_and(|entry| entry.generation == generation);
            if !matches {
                return false;
            }
            let empty = if let Some(items) = data.overlays.get_mut(scope) {
                items.remove(key);
                items.is_empty()
            } else {
                false
            };
            if empty {
                data.overlays.remove(scope);
            }
            true
        }
    }
}

fn scope_chain<'a, K, V>(
    data: &'a RegistryData<K, V>,
    scope: Option<&'a PluginScopeKey>,
) -> Vec<&'a PluginScopeKey> {
    let Some(scope) = scope.filter(|scope| data.known_scopes.contains(*scope)) else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    let mut current = Some(scope);
    let mut seen = BTreeSet::new();
    while let Some(scope) = current {
        if !seen.insert(scope) {
            break;
        }
        chain.push(scope);
        current = data.parents.get(scope);
    }
    chain.reverse();
    chain
}

fn would_cycle<K, V>(
    data: &RegistryData<K, V>,
    scope: &PluginScopeKey,
    parent: &PluginScopeKey,
) -> bool {
    if scope == parent {
        return true;
    }
    let mut current = Some(parent);
    let mut seen = BTreeSet::new();
    while let Some(candidate) = current {
        if candidate == scope || !seen.insert(candidate) {
            return true;
        }
        current = data.parents.get(candidate);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(value: &str) -> PluginKey {
        value.parse().unwrap()
    }

    fn scope(value: &str) -> PluginScopeKey {
        value.parse().unwrap()
    }

    #[tokio::test]
    async fn replace_owned_releases_old_effect_without_old_disposer_deleting_replacement() {
        let registry = ScopedRegistry::<String, String>::new();
        let session = scope("session:42");
        let owner = PluginEffectScope::new(plugin("example.owner"));
        let first = registry
            .register(
                &owner,
                Some(session.clone()),
                "tool".into(),
                "first".into(),
                "host.tool",
            )
            .unwrap();
        let (_second, replaced) = registry
            .replace_owned(
                &owner,
                Some(session.clone()),
                "tool".into(),
                "second".into(),
                "host.tool",
                "session:42:tool",
            )
            .unwrap();
        assert_eq!(replaced.expect("replaced entry").value, "first");
        assert_eq!(
            registry
                .resolve(Some(&session), &"tool".into())
                .unwrap()
                .value,
            "second"
        );

        first.dispose().await.unwrap();
        assert_eq!(
            registry
                .resolve(Some(&session), &"tool".into())
                .unwrap()
                .value,
            "second",
            "old registration disposer must be generation-exact"
        );
        let inspect = owner.inspect();
        assert_eq!(
            inspect
                .effects
                .iter()
                .filter(|effect| effect.kind == "host.tool"
                    && effect.state == crate::effect_scope::PluginEffectState::Active)
                .count(),
            1
        );

        let removed = registry
            .remove_owned(&owner, Some(&session), &"tool".into())
            .expect("owned replacement removal");
        assert_eq!(removed.value, "second");
        assert!(registry.resolve(Some(&session), &"tool".into()).is_none());
        assert_eq!(
            owner
                .inspect()
                .effects
                .iter()
                .filter(|effect| effect.kind == "host.tool"
                    && effect.state == crate::effect_scope::PluginEffectState::Active)
                .count(),
            0
        );
    }

    #[test]
    fn clear_scope_tree_removes_descendants_and_releases_owned_effects() {
        let registry = ScopedRegistry::<String, String>::new();
        let workspace = scope("workspace:one");
        let session = scope("session:42");
        let turn = scope("turn:7");
        registry
            .set_parent(session.clone(), workspace.clone())
            .unwrap();
        registry.set_parent(turn.clone(), session.clone()).unwrap();
        let owner = PluginEffectScope::new(plugin("example.owner"));
        registry
            .register(
                &owner,
                Some(session.clone()),
                "session-tool".into(),
                "session".into(),
                "session tool",
            )
            .unwrap();
        registry
            .register(
                &owner,
                Some(turn.clone()),
                "turn-tool".into(),
                "turn".into(),
                "turn tool",
            )
            .unwrap();
        assert_eq!(
            owner
                .inspect()
                .effects
                .iter()
                .filter(|effect| effect.state == crate::effect_scope::PluginEffectState::Active)
                .count(),
            2
        );

        let removed = registry.clear_scope_tree(&session);
        assert_eq!(removed.len(), 2);
        assert!(
            registry
                .resolve(Some(&session), &"session-tool".into())
                .is_none()
        );
        assert!(registry.resolve(Some(&turn), &"turn-tool".into()).is_none());
        assert!(!registry.inspect().iter().any(|entry| matches!(
            &entry.layer,
            ScopedRegistryLayer::Scope { scope }
                if scope.as_str() == "session:42" || scope.as_str() == "turn:7"
        )));
        assert_eq!(
            owner
                .inspect()
                .effects
                .iter()
                .filter(|effect| effect.state == crate::effect_scope::PluginEffectState::Active)
                .count(),
            0,
            "scope teardown must release the effects that owned the removed registrations"
        );
    }

    #[tokio::test]
    async fn nearest_overlay_matches_lookup_and_manual_dispose_falls_back() {
        let registry = ScopedRegistry::<String, String>::new();
        let workspace = scope("workspace:one");
        let session = scope("session:42");
        registry
            .set_parent(session.clone(), workspace.clone())
            .unwrap();
        let global_owner = PluginEffectScope::new(plugin("example.global"));
        let session_owner = PluginEffectScope::new(plugin("example.session"));
        registry
            .register(
                &global_owner,
                None,
                "search".into(),
                "global".into(),
                "global",
            )
            .unwrap();
        let session_registration = registry
            .register(
                &session_owner,
                Some(session.clone()),
                "search".into(),
                "session".into(),
                "session",
            )
            .unwrap();
        assert_eq!(
            registry
                .resolve(Some(&session), &"search".into())
                .unwrap()
                .value,
            "session"
        );
        assert_eq!(registry.visible(Some(&session))["search"].value, "session");
        session_registration.dispose().await.unwrap();
        assert_eq!(
            registry
                .resolve(Some(&session), &"search".into())
                .unwrap()
                .value,
            "global"
        );
    }

    #[test]
    fn reads_do_not_create_layers_and_parent_cycles_fail() {
        let registry = ScopedRegistry::<String, String>::new();
        let unknown = scope("session:unknown");
        assert!(
            registry
                .resolve(Some(&unknown), &"missing".into())
                .is_none()
        );
        assert_eq!(registry.overlay_count(), 0);
        let a = scope("scope:a");
        let b = scope("scope:b");
        registry.set_parent(b.clone(), a.clone()).unwrap();
        assert!(matches!(
            registry.set_parent(a, b),
            Err(ScopedRegistryError::ParentCycle { .. })
        ));
    }

    #[tokio::test]
    async fn owner_disposal_removes_exact_generation_and_keeps_replacement_safe() {
        let registry = ScopedRegistry::<String, String>::new();
        let owner = PluginEffectScope::new(plugin("example.owner"));
        registry
            .register(&owner, None, "tool".into(), "first".into(), "first")
            .unwrap();
        let report = owner.dispose().await;
        assert!(report.errors.is_empty());
        assert!(registry.resolve(None, &"tool".into()).is_none());
        assert!(
            owner
                .inspect()
                .effects
                .iter()
                .all(|effect| effect.state == crate::effect_scope::PluginEffectState::Disposed)
        );
    }
}
