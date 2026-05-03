//! Plugin entry name → plugin index. Auto-prefixes on collision.

use std::collections::BTreeMap;

use crate::sdk::{EntryBehavior, HostCapability, PluginEntryDecl};

#[derive(Debug, Clone)]
pub struct PluginEntryRegistry {
    /// `exposed_name → (plugin_index, decl)`. `exposed_name` is the name shown
    /// to the model: bare `entry` if unique, else `plugin__entry`.
    by_exposed: BTreeMap<String, PluginEntry>,
    /// All builtins occupy unique reserved keys.
    builtins: Vec<String>,
    generation: u64,
}

#[derive(Debug, Clone)]
pub struct PluginEntrySnapshot {
    pub generation: u64,
    pub entries: Vec<PluginEntry>,
}

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub plugin_index: usize,
    pub plugin_name: String,
    pub original_name: String,
    pub exposed_name: String,
    pub decl: PluginEntryDecl,
}

impl PluginEntryRegistry {
    pub fn new(builtins: impl IntoIterator<Item = String>) -> Self {
        Self {
            by_exposed: BTreeMap::new(),
            builtins: builtins.into_iter().collect(),
            generation: 0,
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_index: usize,
        plugin_name: &str,
        decls: &[PluginEntryDecl],
    ) {
        for decl in decls {
            self.upsert_from_plugin(plugin_index, plugin_name, decl.clone());
        }
    }

    pub fn upsert_from_plugin(
        &mut self,
        plugin_index: usize,
        plugin_name: &str,
        decl: PluginEntryDecl,
    ) -> PluginEntry {
        self.remove_existing_for_plugin(plugin_name, &decl.name);
        let original = decl.name.clone();
        let exposed = self.exposed_name_for(plugin_name, &original, decl.expose_as.as_ref());
        let entry = PluginEntry {
            plugin_index,
            plugin_name: plugin_name.to_string(),
            original_name: original,
            exposed_name: exposed.clone(),
            decl,
        };
        self.by_exposed.insert(exposed, entry.clone());
        self.generation += 1;
        entry
    }

    pub fn remove_from_plugin(
        &mut self,
        plugin_name: &str,
        original_name: &str,
    ) -> Option<PluginEntry> {
        let key = self
            .by_exposed
            .iter()
            .find(|(_, entry)| {
                entry.plugin_name == plugin_name && entry.original_name == original_name
            })
            .map(|(exposed, _)| exposed.clone())?;
        let removed = self.by_exposed.remove(&key);
        if removed.is_some() {
            self.generation += 1;
        }
        removed
    }

    pub fn remove_exposed_from_plugin(
        &mut self,
        plugin_name: &str,
        exposed_name: &str,
    ) -> Option<PluginEntry> {
        if !self
            .by_exposed
            .get(exposed_name)
            .is_some_and(|entry| entry.plugin_name == plugin_name)
        {
            return None;
        }
        let removed = self.by_exposed.remove(exposed_name);
        if removed.is_some() {
            self.generation += 1;
        }
        removed
    }

    pub fn lookup(&self, exposed_name: &str) -> Option<&PluginEntry> {
        self.by_exposed.get(exposed_name)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn snapshot(&self) -> PluginEntrySnapshot {
        PluginEntrySnapshot {
            generation: self.generation,
            entries: self.entries_owned(),
        }
    }

    pub fn entries_owned(&self) -> Vec<PluginEntry> {
        self.by_exposed.values().cloned().collect()
    }

    pub fn entries(&self) -> impl Iterator<Item = &PluginEntry> {
        self.by_exposed.values()
    }

    pub fn count(&self) -> usize {
        self.by_exposed.len()
    }

    fn remove_existing_for_plugin(&mut self, plugin_name: &str, original_name: &str) {
        if let Some(key) = self
            .by_exposed
            .iter()
            .find(|(_, entry)| {
                entry.plugin_name == plugin_name && entry.original_name == original_name
            })
            .map(|(exposed, _)| exposed.clone())
        {
            self.by_exposed.remove(&key);
        }
    }

    fn exposed_name_for(
        &mut self,
        plugin_name: &str,
        original: &str,
        expose_as: Option<&String>,
    ) -> String {
        if let Some(forced) = expose_as {
            return forced.clone();
        }
        if self.builtins.iter().any(|b| b == original) || self.by_exposed.contains_key(original) {
            let new_name = format!("{plugin_name}__{original}");
            if let Some(existing) = self.by_exposed.remove(original) {
                let renamed_existing =
                    format!("{}__{}", existing.plugin_name, existing.original_name);
                let mut renamed = existing;
                renamed.exposed_name = renamed_existing.clone();
                self.by_exposed.insert(renamed_existing, renamed);
            }
            new_name
        } else {
            original.to_string()
        }
    }
}

pub fn effective_host_capabilities(decls: &[PluginEntryDecl]) -> Vec<HostCapability> {
    let mut capabilities = Vec::new();
    for decl in decls {
        for capability in &decl.host_capabilities {
            if !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
    }
    capabilities
}

/// A behavior helper for plugin-shipped entries the host has to filter against
/// the catalog/agent.
pub fn behavior_label(b: EntryBehavior) -> &'static str {
    match b {
        EntryBehavior::ReadOnly => "read-only",
        EntryBehavior::WriteSandboxed => "write-sandboxed",
        EntryBehavior::WriteUnsandboxed => "write-unsandboxed",
        EntryBehavior::Task => "task",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_host_capabilities_unions_plugin_entries() {
        let decls = vec![
            PluginEntryDecl::new("one", serde_json::json!({}))
                .host_capability(HostCapability::ReadConfig)
                .host_capability(HostCapability::ListTools),
            PluginEntryDecl::new("two", serde_json::json!({}))
                .host_capability(HostCapability::ListTools)
                .host_capability(HostCapability::InvokeTool),
        ];

        assert_eq!(
            effective_host_capabilities(&decls),
            vec![
                HostCapability::ReadConfig,
                HostCapability::ListTools,
                HostCapability::InvokeTool,
            ]
        );
    }

    #[test]
    fn effective_host_capabilities_are_empty_without_declarations() {
        let decls = vec![
            PluginEntryDecl::new("one", serde_json::json!({})),
            PluginEntryDecl::new("two", serde_json::json!({})),
        ];

        assert!(effective_host_capabilities(&decls).is_empty());
    }

    #[test]
    fn upsert_increments_generation_and_lookup() {
        let mut registry = PluginEntryRegistry::new(Vec::<String>::new());
        assert_eq!(registry.generation(), 0);

        let entry = registry.upsert_from_plugin(
            0,
            "alpha",
            PluginEntryDecl::new("ping", serde_json::json!({})),
        );
        assert_eq!(entry.exposed_name, "ping");
        assert_eq!(registry.generation(), 1);
        assert!(registry.lookup("ping").is_some());

        let updated = registry.upsert_from_plugin(
            0,
            "alpha",
            PluginEntryDecl::new("ping", serde_json::json!({"v": 2})).description("v2"),
        );
        assert_eq!(updated.exposed_name, "ping");
        assert_eq!(registry.generation(), 2);
        assert_eq!(
            registry.lookup("ping").unwrap().decl.description.as_deref(),
            Some("v2")
        );
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn upsert_collision_renames_both_sides_like_extend() {
        let mut registry = PluginEntryRegistry::new(Vec::<String>::new());
        registry.upsert_from_plugin(
            0,
            "alpha",
            PluginEntryDecl::new("ping", serde_json::json!({})),
        );
        registry.upsert_from_plugin(
            1,
            "beta",
            PluginEntryDecl::new("ping", serde_json::json!({})),
        );

        assert!(registry.lookup("ping").is_none());
        assert_eq!(registry.lookup("alpha__ping").unwrap().plugin_name, "alpha");
        assert_eq!(registry.lookup("beta__ping").unwrap().plugin_name, "beta");
    }

    #[test]
    fn remove_increments_generation_and_keeps_namespace_stable() {
        let mut registry = PluginEntryRegistry::new(Vec::<String>::new());
        registry.upsert_from_plugin(
            0,
            "alpha",
            PluginEntryDecl::new("ping", serde_json::json!({})),
        );
        registry.upsert_from_plugin(
            1,
            "beta",
            PluginEntryDecl::new("ping", serde_json::json!({})),
        );
        let baseline = registry.generation();

        let removed = registry
            .remove_from_plugin("beta", "ping")
            .expect("remove existing entry");
        assert_eq!(removed.exposed_name, "beta__ping");
        assert_eq!(registry.generation(), baseline + 1);

        // Phase 2 keeps the surviving entry namespaced; do not de-prefix on remove.
        assert!(registry.lookup("alpha__ping").is_some());
        assert!(registry.lookup("ping").is_none());
    }

    #[test]
    fn snapshot_returns_owned_entries_with_generation() {
        let mut registry = PluginEntryRegistry::new(Vec::<String>::new());
        registry.upsert_from_plugin(
            0,
            "alpha",
            PluginEntryDecl::new("ping", serde_json::json!({})),
        );

        let snapshot = registry.snapshot();
        assert_eq!(snapshot.generation, registry.generation());
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].exposed_name, "ping");
    }
}
