//! Tool name → plugin index. Auto-prefixes on collision.

use std::collections::BTreeMap;

use crate::sdk::{ToolBehavior, ToolDecl};

#[derive(Debug, Clone)]
pub struct ToolRegistry {
    /// `exposed_name → (plugin_index, decl)`. `exposed_name` is the name shown
    /// to the model: bare `tool` if unique, else `plugin__tool`.
    by_exposed: BTreeMap<String, ToolEntry>,
    /// All builtins occupy unique reserved keys.
    builtins: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub plugin_index: usize,
    pub plugin_name: String,
    pub original_name: String,
    pub exposed_name: String,
    pub decl: ToolDecl,
}

impl ToolRegistry {
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
        decls: &[ToolDecl],
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
                    let renamed_existing = format!(
                        "{}__{}",
                        existing.plugin_name, existing.original_name
                    );
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
                ToolEntry {
                    plugin_index,
                    plugin_name: plugin_name.to_string(),
                    original_name: original,
                    exposed_name: exposed,
                    decl: decl.clone(),
                },
            );
        }
    }

    pub fn lookup(&self, exposed_name: &str) -> Option<&ToolEntry> {
        self.by_exposed.get(exposed_name)
    }

    pub fn entries(&self) -> impl Iterator<Item = &ToolEntry> {
        self.by_exposed.values()
    }

    pub fn count(&self) -> usize {
        self.by_exposed.len()
    }
}

/// A behavior helper for plugin-shipped tools the host has to filter against
/// the catalog/agent.
pub fn behavior_label(b: ToolBehavior) -> &'static str {
    match b {
        ToolBehavior::ReadOnly => "read-only",
        ToolBehavior::WriteSandboxed => "write-sandboxed",
        ToolBehavior::WriteUnsandboxed => "write-unsandboxed",
        ToolBehavior::Task => "task",
    }
}
