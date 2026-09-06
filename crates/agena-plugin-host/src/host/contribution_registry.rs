//! Replaceable host values use last-registration-wins semantics within each
//! plugin's namespace, with cleanup tied to that exact registration. Bindings
//! are cloned before invoking plugin code, so no registry lock spans an await.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use super::host_handle::{recover_read, recover_write};
use crate::effect_scope::{PluginEffectScope, PluginEffectScopeError};
use crate::registration_owner::RegistrationOwner;
use crate::sdk::PluginKey;

struct OwnedContribution<V> {
    value: V,
    owner: RegistrationOwner,
}

// Nest by plugin so transport lookups can borrow both keys. Streaming events
// should not allocate a new owned composite key just to find their binding.
type Contributions<V> = BTreeMap<PluginKey, BTreeMap<String, OwnedContribution<V>>>;

pub(super) struct ContributionRegistry<V> {
    entries: Arc<RwLock<Contributions<V>>>,
    label: &'static str,
}

impl<V: Clone + Send + Sync + 'static> ContributionRegistry<V> {
    pub(super) fn new(label: &'static str) -> Self {
        Self {
            entries: Arc::new(RwLock::new(BTreeMap::new())),
            label,
        }
    }

    pub(super) fn insert(
        &self,
        scope: &Arc<PluginEffectScope>,
        kind: &'static str,
        label: String,
        value: V,
    ) -> Result<(), PluginEffectScopeError> {
        let _lease = scope.lease()?;
        let key = (scope.plugin_id().clone(), label.clone());
        let registry = Arc::downgrade(&self.entries);
        let registry_label = self.label;
        let cleanup_key = key.clone();
        let mut entries = recover_write(&self.entries, self.label);
        let owner = RegistrationOwner::new(scope, kind, label, move |id| {
            if let Some(registry) = registry.upgrade() {
                let mut entries = recover_write(&registry, registry_label);
                if let Some(items) = entries.get_mut(&cleanup_key.0)
                    && items
                        .get(&cleanup_key.1)
                        .is_some_and(|entry| entry.owner.matches(&id))
                {
                    items.remove(&cleanup_key.1);
                    if items.is_empty() {
                        entries.remove(&cleanup_key.0);
                    }
                }
            }
            Ok(())
        })?;
        if let Some(previous) = entries
            .entry(key.0)
            .or_default()
            .insert(key.1, OwnedContribution { value, owner })
        {
            previous.owner.release();
        }
        Ok(())
    }

    pub(super) fn remove(&self, plugin: &PluginKey, id: &str) -> bool {
        let mut entries = recover_write(&self.entries, self.label);
        let Some(items) = entries.get_mut(plugin) else {
            return false;
        };
        let removed = items.remove(id);
        if items.is_empty() {
            entries.remove(plugin);
        }
        if let Some(entry) = removed {
            entry.owner.release();
            true
        } else {
            false
        }
    }

    pub(super) fn get(&self, plugin: &PluginKey, id: &str) -> Option<V> {
        recover_read(&self.entries, self.label)
            .get(plugin)?
            .get(id)
            .map(|entry| entry.value.clone())
    }

    pub(super) fn remove_plugin(&self, plugin: &PluginKey) {
        let mut entries = recover_write(&self.entries, self.label);
        if let Some(items) = entries.remove(plugin) {
            for entry in items.into_values() {
                entry.owner.release();
            }
        }
    }

    pub(super) fn values(&self) -> Vec<V> {
        recover_read(&self.entries, self.label)
            .values()
            .flat_map(|items| items.values())
            .map(|entry| entry.value.clone())
            .collect()
    }

    pub(super) fn owned_values(&self, owner: &PluginEffectScope) -> Vec<V> {
        recover_read(&self.entries, self.label)
            .values()
            .flat_map(|items| items.values())
            .filter(|entry| entry.owner.belongs_to(owner))
            .map(|entry| entry.value.clone())
            .collect()
    }
}
