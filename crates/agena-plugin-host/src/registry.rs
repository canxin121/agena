//! Plugin tool name registry. Model-visible tool names are always `plugin/tool`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sdk::manifest::UiTextDisplayMode;
use crate::sdk::{HostCapability, PluginToolDecl, ToolDescriptionMode};

#[derive(Debug, Clone)]
pub struct PluginToolRegistry {
    /// `exposed_name -> tool`. `exposed_name` is the name shown
    /// to the model and is always `plugin/tool`.
    by_exposed: BTreeMap<String, RegisteredTool>,
    plugin_tool_defaults: BTreeMap<String, ToolDescriptionMode>,
    plugin_ui_defaults: BTreeMap<String, UiTextDisplayMode>,
    generation: u64,
}

#[derive(Debug, Clone)]
pub struct ToolRegistrySnapshot {
    pub generation: u64,
    pub tools: Vec<RegisteredTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisteredTool {
    /// Host-local plugin id from `plugins.list.<id>`. This is used to route
    /// calls back to the loaded transport.
    pub plugin_id: String,
    /// Manifest plugin name. This is the namespace shown to the model.
    pub plugin_name: String,
    pub original_name: String,
    pub exposed_name: String,
    pub decl: PluginToolDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_exposed_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_input: Option<serde_json::Value>,
}

impl RegisteredTool {
    pub fn new(plugin_name: impl Into<String>, decl: PluginToolDecl) -> Self {
        let plugin_name = plugin_name.into();
        Self::new_with_plugin_id(plugin_name.clone(), plugin_name, decl)
    }

    pub fn new_with_plugin_id(
        plugin_id: impl Into<String>,
        plugin_name: impl Into<String>,
        decl: PluginToolDecl,
    ) -> Self {
        let plugin_id = plugin_id.into();
        let plugin_name = plugin_name.into();
        let original_name = decl.name.clone();
        let exposed_name = exposed_tool_name(&plugin_name, &original_name);
        Self {
            plugin_id,
            plugin_name,
            original_name: original_name.clone(),
            exposed_name,
            decl,
            base_exposed_name: None,
            fixed_input: None,
        }
    }

    pub fn behavior_exposed_name(&self) -> &str {
        self.base_exposed_name
            .as_deref()
            .unwrap_or(self.exposed_name.as_str())
    }

    pub fn with_model_alias(
        &self,
        alias_name: impl Into<String>,
        decl: PluginToolDecl,
        fixed_input: serde_json::Value,
    ) -> Self {
        let alias_name = alias_name.into();
        Self {
            plugin_id: self.plugin_id.clone(),
            plugin_name: self.plugin_name.clone(),
            original_name: self.original_name.clone(),
            exposed_name: exposed_tool_name(&self.plugin_name, &alias_name),
            decl,
            base_exposed_name: Some(self.behavior_exposed_name().to_string()),
            fixed_input: Some(fixed_input),
        }
    }

    pub fn description_text(&self) -> &str {
        self.decl.description_text()
    }

    pub fn summary_text(&self) -> Option<&str> {
        self.decl.summary_text()
    }

    pub fn help_text(&self) -> Option<&str> {
        self.decl.help_text()
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
}

impl PluginToolRegistry {
    pub fn new() -> Self {
        Self {
            by_exposed: BTreeMap::new(),
            plugin_tool_defaults: BTreeMap::new(),
            plugin_ui_defaults: BTreeMap::new(),
            generation: 0,
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_id: &str,
        plugin_name: &str,
        decls: &[PluginToolDecl],
        plugin_tool_default: Option<ToolDescriptionMode>,
        plugin_ui_default: Option<UiTextDisplayMode>,
    ) {
        assert_valid_tool_namespace(plugin_name, "plugin name");
        if let Some(mode) = plugin_tool_default {
            self.plugin_tool_defaults
                .insert(plugin_id.to_string(), mode);
        } else {
            self.plugin_tool_defaults.remove(plugin_id);
        }
        if let Some(mode) = plugin_ui_default {
            self.plugin_ui_defaults.insert(plugin_id.to_string(), mode);
        } else {
            self.plugin_ui_defaults.remove(plugin_id);
        }
        for decl in decls {
            self.upsert_from_plugin(plugin_id, plugin_name, decl.clone());
        }
    }

    pub fn upsert_from_plugin(
        &mut self,
        plugin_id: &str,
        plugin_name: &str,
        mut decl: PluginToolDecl,
    ) -> RegisteredTool {
        assert_valid_tool_namespace(plugin_name, "plugin name");
        if decl.description_mode.is_none() {
            decl.description_mode = self.plugin_tool_defaults.get(plugin_id).copied();
        }
        if decl.ui_display_mode.is_none() {
            decl.ui_display_mode = self.plugin_ui_defaults.get(plugin_id).copied();
        }
        let original_name = decl.name.clone();
        assert_valid_tool_namespace(&original_name, "tool name");
        let mut tools = self.registered_tools_owned();
        tools.retain(|tool| !(tool.plugin_id == plugin_id && tool.original_name == original_name));
        tools.push(RegisteredTool {
            plugin_id: plugin_id.to_string(),
            plugin_name: plugin_name.to_string(),
            original_name: original_name.clone(),
            exposed_name: exposed_tool_name(plugin_name, &original_name),
            decl,
            base_exposed_name: None,
            fixed_input: None,
        });
        self.rebuild(tools);
        self.generation += 1;
        self.lookup_for_plugin(plugin_id, &original_name)
            .expect("upserted tool should exist after rebuild")
            .clone()
    }

    pub fn remove_from_plugin(
        &mut self,
        plugin_id: &str,
        original_name: &str,
    ) -> Option<RegisteredTool> {
        assert_valid_tool_namespace(original_name, "tool name");
        let removed = self.lookup_for_plugin(plugin_id, original_name)?.clone();
        let mut tools = self.registered_tools_owned();
        tools.retain(|tool| !(tool.plugin_id == plugin_id && tool.original_name == original_name));
        self.rebuild(tools);
        self.generation += 1;
        Some(removed)
    }

    pub fn remove_exposed_from_plugin(
        &mut self,
        plugin_id: &str,
        exposed_name: &str,
    ) -> Option<RegisteredTool> {
        let original_name = self
            .by_exposed
            .get(exposed_name)
            .filter(|tool| tool.plugin_id == plugin_id)
            .map(|tool| tool.original_name.clone())?;
        self.remove_from_plugin(plugin_id, &original_name)
    }

    pub fn lookup_tool(&self, exposed_name: &str) -> Option<&RegisteredTool> {
        self.by_exposed.get(exposed_name)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn snapshot(&self) -> ToolRegistrySnapshot {
        ToolRegistrySnapshot {
            generation: self.generation,
            tools: self.registered_tools_owned(),
        }
    }

    pub fn registered_tools_owned(&self) -> Vec<RegisteredTool> {
        self.by_exposed.values().cloned().collect()
    }

    pub fn registered_tools(&self) -> impl Iterator<Item = &RegisteredTool> {
        self.by_exposed.values()
    }

    pub fn count(&self) -> usize {
        self.by_exposed.len()
    }

    fn rebuild(&mut self, mut tools: Vec<RegisteredTool>) {
        for tool in &mut tools {
            assert_valid_tool_namespace(&tool.plugin_name, "plugin name");
            assert_valid_tool_namespace(&tool.original_name, "tool name");
            tool.exposed_name = exposed_tool_name(&tool.plugin_name, &tool.original_name);
        }
        self.by_exposed = tools
            .into_iter()
            .map(|tool| (tool.exposed_name.clone(), tool))
            .collect();
    }

    fn lookup_for_plugin(&self, plugin_id: &str, original_name: &str) -> Option<&RegisteredTool> {
        self.by_exposed
            .values()
            .find(|tool| tool.plugin_id == plugin_id && tool.original_name == original_name)
    }
}

pub fn exposed_tool_name(plugin_name: &str, tool_name: &str) -> String {
    assert_valid_tool_namespace(plugin_name, "plugin name");
    assert_valid_tool_namespace(tool_name, "tool name");
    format!("{plugin_name}/{tool_name}")
}

fn assert_valid_tool_namespace(value: &str, label: &str) {
    assert!(
        !value.trim().is_empty(),
        "{label} must not be empty for plugin tool exposure"
    );
    assert!(
        !value.contains('/'),
        "{label} `{value}` must not contain `/`; model-visible tool names use `plugin/tool`"
    );
}

impl Default for PluginToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_tool_default_applies_when_tool_has_no_explicit_mode() {
        let mut registry = PluginToolRegistry::new();
        registry.extend_from_plugin(
            "plugin-id",
            "fixture",
            &[PluginToolDecl::new(
                "inspect",
                serde_json::json!({ "type": "object" }),
            )],
            Some(ToolDescriptionMode::Brief),
            Some(UiTextDisplayMode::Summary),
        );

        let tool = registry
            .lookup_tool("fixture/inspect")
            .expect("tool should be registered");
        assert_eq!(tool.decl.description_mode, Some(ToolDescriptionMode::Brief));
        assert_eq!(tool.decl.ui_display_mode, Some(UiTextDisplayMode::Summary));
    }

    #[test]
    fn explicit_tool_mode_overrides_plugin_default() {
        let mut registry = PluginToolRegistry::new();
        registry.extend_from_plugin(
            "plugin-id",
            "fixture",
            &[
                PluginToolDecl::new("inspect", serde_json::json!({ "type": "object" }))
                    .description_mode(ToolDescriptionMode::Detailed)
                    .ui_display_mode(UiTextDisplayMode::Detailed),
            ],
            Some(ToolDescriptionMode::Brief),
            Some(UiTextDisplayMode::Summary),
        );

        let tool = registry
            .lookup_tool("fixture/inspect")
            .expect("tool should be registered");
        assert_eq!(
            tool.decl.description_mode,
            Some(ToolDescriptionMode::Detailed)
        );
        assert_eq!(tool.decl.ui_display_mode, Some(UiTextDisplayMode::Detailed));
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
/// plugins that need host capabilities without exposing any model-visible tool
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

/// Per-tool capability map: each declared tool maps to its own
/// declared `host_capabilities` list. Used by [`HostHandle`] so that a
/// plugin shipping multiple tools can scope dangerous capabilities to
/// just the tool that needs them rather than leaking them via the union
/// to every tool the plugin owns.
pub fn per_tool_host_capabilities(
    decls: &[PluginToolDecl],
) -> std::collections::HashMap<String, Vec<HostCapability>> {
    let mut out = std::collections::HashMap::new();
    for decl in decls {
        out.insert(decl.name.clone(), decl.host_capabilities.clone());
    }
    out
}
