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
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_index: usize,
        plugin_name: &str,
        decls: &[PluginEntryDecl],
    ) {
        for decl in decls {
            let original = decl.name.clone();
            // Decide an exposed name.
            let exposed = if let Some(forced) = &decl.expose_as {
                forced.clone()
            } else if self.builtins.iter().any(|b| b == &original)
                || self.by_exposed.contains_key(&original)
            {
                // Collision: re-namespace this incoming AND the existing colliding entry.
                let new_name = format!("{plugin_name}__{original}");
                if let Some(existing) = self.by_exposed.remove(&original) {
                    let renamed_existing =
                        format!("{}__{}", existing.plugin_name, existing.original_name);
                    let mut e2 = existing;
                    e2.exposed_name = renamed_existing.clone();
                    self.by_exposed.insert(renamed_existing, e2);
                }
                new_name
            } else {
                original.clone()
            };
            self.by_exposed.insert(
                exposed.clone(),
                PluginEntry {
                    plugin_index,
                    plugin_name: plugin_name.to_string(),
                    original_name: original,
                    exposed_name: exposed,
                    decl: decl.clone(),
                },
            );
        }
    }

    pub fn lookup(&self, exposed_name: &str) -> Option<&PluginEntry> {
        self.by_exposed.get(exposed_name)
    }

    pub fn entries(&self) -> impl Iterator<Item = &PluginEntry> {
        self.by_exposed.values()
    }

    pub fn count(&self) -> usize {
        self.by_exposed.len()
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
}
