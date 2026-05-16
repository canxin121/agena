//! Plugin tool name registry. Collisions are disambiguated as `plugin/tool`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sdk::{HostCapability, PluginToolDecl};

#[derive(Debug, Clone)]
pub struct PluginEntryRegistry {
    /// `exposed_name → entry`. `exposed_name` is the name shown
    /// to the model: bare `tool` if unique, else `plugin/tool`.
    by_exposed: BTreeMap<String, PluginEntry>,
    generation: u64,
}

#[derive(Debug, Clone)]
pub struct PluginEntrySnapshot {
    pub generation: u64,
    pub entries: Vec<PluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginEntry {
    pub plugin_name: String,
    pub original_name: String,
    pub exposed_name: String,
    pub decl: PluginToolDecl,
}

impl PluginEntry {
    pub fn new(plugin_name: impl Into<String>, decl: PluginToolDecl) -> Self {
        let plugin_name = plugin_name.into();
        let original_name = decl.name.clone();
        Self {
            plugin_name,
            original_name: original_name.clone(),
            exposed_name: original_name,
            decl,
        }
    }

    pub fn description_text(&self) -> &str {
        self.decl.description_text()
    }

    pub fn sanitized_input_schema(&self) -> serde_json::Value {
        self.decl.sanitized_input_schema()
    }

    pub fn effective_tags(&self) -> Vec<crate::sdk::ToolTag> {
        self.decl.effective_tags()
    }

    pub fn has_tag(&self, tag: crate::sdk::ToolTag) -> bool {
        self.decl.has_tag(tag)
    }

    pub fn should_load_by_default(&self) -> bool {
        self.decl.should_load_by_default()
    }

    pub fn is_deferred(&self) -> bool {
        self.decl.is_deferred()
    }
}

impl PluginEntryRegistry {
    pub fn new() -> Self {
        Self {
            by_exposed: BTreeMap::new(),
            generation: 0,
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_name: &str,
        decls: &[PluginToolDecl],
    ) {
        for decl in decls {
            self.upsert_from_plugin(plugin_name, decl.clone());
        }
    }

    pub fn upsert_from_plugin(
        &mut self,
        plugin_name: &str,
        decl: PluginToolDecl,
    ) -> PluginEntry {
        let original_name = decl.name.clone();
        let mut entries = self.entries_owned();
        entries.retain(|entry| {
            !(entry.plugin_name == plugin_name && entry.original_name == original_name)
        });
        entries.push(PluginEntry {
            plugin_name: plugin_name.to_string(),
            original_name: original_name.clone(),
            exposed_name: original_name.clone(),
            decl,
        });
        self.rebuild(entries);
        self.generation += 1;
        self.lookup_for_plugin(plugin_name, &original_name)
            .expect("upserted entry should exist after rebuild")
            .clone()
    }

    pub fn remove_from_plugin(
        &mut self,
        plugin_name: &str,
        original_name: &str,
    ) -> Option<PluginEntry> {
        let removed = self.lookup_for_plugin(plugin_name, original_name)?.clone();
        let mut entries = self.entries_owned();
        entries.retain(|entry| {
            !(entry.plugin_name == plugin_name && entry.original_name == original_name)
        });
        self.rebuild(entries);
        self.generation += 1;
        Some(removed)
    }

    pub fn remove_exposed_from_plugin(
        &mut self,
        plugin_name: &str,
        exposed_name: &str,
    ) -> Option<PluginEntry> {
        let original_name = self
            .by_exposed
            .get(exposed_name)
            .filter(|entry| entry.plugin_name == plugin_name)
            .map(|entry| entry.original_name.clone())?;
        self.remove_from_plugin(plugin_name, &original_name)
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

    fn rebuild(&mut self, mut entries: Vec<PluginEntry>) {
        let mut counts = BTreeMap::<String, usize>::new();
        for entry in &entries {
            *counts.entry(entry.original_name.clone()).or_default() += 1;
        }
        for entry in &mut entries {
            entry.exposed_name = if counts.get(&entry.original_name).copied().unwrap_or_default() > 1
            {
                format!("{}/{}", entry.plugin_name, entry.original_name)
            } else {
                entry.original_name.clone()
            };
        }
        self.by_exposed = entries
            .into_iter()
            .map(|entry| (entry.exposed_name.clone(), entry))
            .collect();
    }

    fn lookup_for_plugin(&self, plugin_name: &str, original_name: &str) -> Option<&PluginEntry> {
        self.by_exposed.values().find(|entry| {
            entry.plugin_name == plugin_name && entry.original_name == original_name
        })
    }
}

impl Default for PluginEntryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn effective_host_capabilities(decls: &[PluginToolDecl]) -> Vec<HostCapability> {
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

/// Same as [`effective_host_capabilities`] but additionally folds in
/// manifest-level `plugin_capabilities`. Used by the host to authorize
/// plugins that need host capabilities without exposing any tool entry
/// (e.g. background skill discovery plugins).
pub fn effective_host_capabilities_for_manifest(
    decls: &[PluginToolDecl],
    plugin_capabilities: &[HostCapability],
) -> Vec<HostCapability> {
    let mut capabilities = effective_host_capabilities(decls);
    for capability in plugin_capabilities {
        if !capabilities.contains(capability) {
            capabilities.push(*capability);
        }
    }
    capabilities
}

/// Per-entry capability map: each declared entry maps to its own
/// declared `host_capabilities` list. Used by [`HostHandle`] so that a
/// plugin shipping multiple entries can scope dangerous capabilities to
/// just the entry that needs them rather than leaking them via the union
/// to every entry the plugin owns.
pub fn per_entry_host_capabilities(
    decls: &[PluginToolDecl],
) -> std::collections::HashMap<String, Vec<HostCapability>> {
    let mut out = std::collections::HashMap::new();
    for decl in decls {
        out.insert(decl.name.clone(), decl.host_capabilities.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_host_capabilities_unions_plugin_entries() {
        let decls = vec![
            PluginToolDecl::new("one", serde_json::json!({}))
                .host_capability(HostCapability::ReadConfig)
                .host_capability(HostCapability::ListTools),
            PluginToolDecl::new("two", serde_json::json!({}))
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
            PluginToolDecl::new("one", serde_json::json!({})),
            PluginToolDecl::new("two", serde_json::json!({})),
        ];

        assert!(effective_host_capabilities(&decls).is_empty());
    }

    #[test]
    fn upsert_increments_generation_and_lookup() {
        let mut registry = PluginEntryRegistry::new();
        assert_eq!(registry.generation(), 0);

        let entry =
            registry.upsert_from_plugin("alpha", PluginToolDecl::new("ping", serde_json::json!({})));
        assert_eq!(entry.exposed_name, "ping");
        assert_eq!(registry.generation(), 1);
        assert!(registry.lookup("ping").is_some());

        let updated = registry.upsert_from_plugin(
            "alpha",
            PluginToolDecl::new("ping", serde_json::json!({"v": 2})).description("v2"),
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
    fn upsert_collision_namespaces_both_sides() {
        let mut registry = PluginEntryRegistry::new();
        registry.upsert_from_plugin("alpha", PluginToolDecl::new("ping", serde_json::json!({})));
        registry.upsert_from_plugin("beta", PluginToolDecl::new("ping", serde_json::json!({})));

        assert!(registry.lookup("ping").is_none());
        assert_eq!(registry.lookup("alpha/ping").unwrap().plugin_name, "alpha");
        assert_eq!(registry.lookup("beta/ping").unwrap().plugin_name, "beta");
    }

    #[test]
    fn remove_increments_generation_and_restores_unique_name() {
        let mut registry = PluginEntryRegistry::new();
        registry.upsert_from_plugin("alpha", PluginToolDecl::new("ping", serde_json::json!({})));
        registry.upsert_from_plugin("beta", PluginToolDecl::new("ping", serde_json::json!({})));
        let baseline = registry.generation();

        let removed = registry
            .remove_from_plugin("beta", "ping")
            .expect("remove existing entry");
        assert_eq!(removed.exposed_name, "beta/ping");
        assert_eq!(registry.generation(), baseline + 1);

        assert!(registry.lookup("alpha/ping").is_none());
        assert!(registry.lookup("ping").is_some());
    }

    #[test]
    fn snapshot_returns_owned_entries_with_generation() {
        let mut registry = PluginEntryRegistry::new();
        registry.upsert_from_plugin("alpha", PluginToolDecl::new("ping", serde_json::json!({})));

        let snapshot = registry.snapshot();
        assert_eq!(snapshot.generation, registry.generation());
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].exposed_name, "ping");
    }
}
