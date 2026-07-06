//! Plugin tool registry. A tool has exactly one model-visible name:
//! `plugin_name.tool_name`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sdk::manifest::UiTextDisplayMode;
use crate::sdk::{HostCapability, ToolDefinition, ToolDescriptionMode, ToolTag};

#[derive(Debug, Clone)]
pub struct PluginToolRegistry {
    by_model: BTreeMap<String, RegisteredTool>,
    plugin_tool_defaults: BTreeMap<String, ToolDescriptionMode>,
    plugin_ui_defaults: BTreeMap<String, UiTextDisplayMode>,
    generation: u64,
}

#[derive(Debug, Clone)]
pub struct ToolRegistrySnapshot {
    pub generation: u64,
    pub tools: Vec<RegisteredTool>,
}

pub type RegisteredTool = ToolBinding;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolBinding {
    pub plugin_name: String,
    pub tool_name: String,
    pub model_name: String,
    pub definition: ToolDefinition,
}

impl ToolBinding {
    pub fn new(plugin_name: impl Into<String>, definition: ToolDefinition) -> Self {
        let plugin_name = plugin_name.into();
        let tool_name = definition.name.clone();
        let model_name = model_tool_name(&plugin_name, &tool_name);
        Self {
            plugin_name,
            tool_name,
            model_name,
            definition,
        }
    }

    pub fn description_text(&self) -> &str {
        self.definition.description_text()
    }

    pub fn summary_text(&self) -> Option<&str> {
        self.definition.summary_text()
    }

    pub fn help_text(&self) -> Option<&str> {
        self.definition.help_text()
    }

    pub fn before_help_text(&self) -> Option<&str> {
        self.definition.before_help_text()
    }

    pub fn after_help_text(&self) -> Option<&str> {
        self.definition.after_help_text()
    }

    pub fn sanitized_input_schema(&self) -> serde_json::Value {
        self.definition.sanitized_input_schema()
    }

    pub fn sanitized_output_schema(&self) -> serde_json::Value {
        self.definition.sanitized_output_schema()
    }

    pub fn definition_identity(&self) -> String {
        let value = serde_json::json!({
            "plugin_name": self.plugin_name,
            "tool_name": self.tool_name,
            "model_name": self.model_name,
            "description": self.description_text(),
            "summary": self.summary_text(),
            "input_schema": self.sanitized_input_schema(),
            "output_schema": self.sanitized_output_schema(),
            "strict": self.definition.contract.strict,
            "streaming": self.definition.runtime.streaming,
            "tags": self.effective_tags(),
        });
        let bytes = serde_json::to_vec(&value).unwrap_or_default();
        blake3::hash(bytes.as_slice()).to_hex().to_string()
    }

    pub fn effective_tags(&self) -> Vec<ToolTag> {
        self.definition.effective_tags()
    }

    pub fn has_tag(&self, tag: ToolTag) -> bool {
        self.definition.has_tag(tag)
    }
}

impl PluginToolRegistry {
    pub fn new() -> Self {
        Self {
            by_model: BTreeMap::new(),
            plugin_tool_defaults: BTreeMap::new(),
            plugin_ui_defaults: BTreeMap::new(),
            generation: 0,
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_name: &str,
        definitions: &[ToolDefinition],
        plugin_tool_default: Option<ToolDescriptionMode>,
        plugin_ui_default: Option<UiTextDisplayMode>,
    ) {
        assert_valid_tool_namespace(plugin_name, "plugin name");
        if let Some(mode) = plugin_tool_default {
            self.plugin_tool_defaults
                .insert(plugin_name.to_string(), mode);
        } else {
            self.plugin_tool_defaults.remove(plugin_name);
        }
        if let Some(mode) = plugin_ui_default {
            self.plugin_ui_defaults
                .insert(plugin_name.to_string(), mode);
        } else {
            self.plugin_ui_defaults.remove(plugin_name);
        }
        for definition in definitions {
            self.upsert_from_plugin(plugin_name, definition.clone());
        }
    }

    pub fn upsert_from_plugin(
        &mut self,
        plugin_name: &str,
        mut definition: ToolDefinition,
    ) -> RegisteredTool {
        assert_valid_tool_namespace(plugin_name, "plugin name");
        if definition.display.description_mode.is_none() {
            definition.display.description_mode =
                self.plugin_tool_defaults.get(plugin_name).copied();
        }
        if definition.display.ui_display_mode.is_none() {
            definition.display.ui_display_mode = self.plugin_ui_defaults.get(plugin_name).copied();
        }
        let tool_name = definition.name.clone();
        assert_valid_tool_namespace(&tool_name, "tool name");
        let model_name = model_tool_name(plugin_name, &tool_name);
        let mut tools = self.registered_tools_owned();
        tools.retain(|tool| !(tool.plugin_name == plugin_name && tool.tool_name == tool_name));
        tools.push(ToolBinding::new(plugin_name, definition));
        self.rebuild(tools);
        self.generation += 1;
        self.lookup_for_plugin(plugin_name, &tool_name)
            .or_else(|| self.lookup_tool(&model_name))
            .expect("upserted tool should exist after rebuild")
            .clone()
    }

    pub fn remove_from_plugin(
        &mut self,
        plugin_name: &str,
        tool_name: &str,
    ) -> Option<RegisteredTool> {
        assert_valid_tool_namespace(plugin_name, "plugin name");
        assert_valid_tool_namespace(tool_name, "tool name");
        let removed = self.lookup_for_plugin(plugin_name, tool_name)?.clone();
        let mut tools = self.registered_tools_owned();
        tools.retain(|tool| !(tool.plugin_name == plugin_name && tool.tool_name == tool_name));
        self.rebuild(tools);
        self.generation += 1;
        Some(removed)
    }

    pub fn lookup_tool(&self, model_name: &str) -> Option<&RegisteredTool> {
        self.by_model.get(model_name)
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
        self.by_model.values().cloned().collect()
    }

    pub fn registered_tools(&self) -> impl Iterator<Item = &RegisteredTool> {
        self.by_model.values()
    }

    pub fn count(&self) -> usize {
        self.by_model.len()
    }

    fn rebuild(&mut self, mut tools: Vec<RegisteredTool>) {
        for tool in &mut tools {
            assert_valid_tool_namespace(&tool.plugin_name, "plugin name");
            assert_valid_tool_namespace(&tool.tool_name, "tool name");
            tool.definition.name = tool.tool_name.clone();
            tool.model_name = model_tool_name(&tool.plugin_name, &tool.tool_name);
        }
        let mut by_model = BTreeMap::new();
        for tool in tools {
            if let Some(existing) = by_model.insert(tool.model_name.clone(), tool) {
                panic!(
                    "tool `{}` collides with another registered tool after model-visible name normalization",
                    existing.model_name
                );
            }
        }
        self.by_model = by_model;
    }

    pub fn lookup_for_plugin(&self, plugin_name: &str, tool_name: &str) -> Option<&RegisteredTool> {
        self.by_model
            .values()
            .find(|tool| tool.plugin_name == plugin_name && tool.tool_name == tool_name)
    }
}

pub fn model_tool_name(plugin_name: &str, tool_name: &str) -> String {
    assert_valid_tool_namespace(plugin_name, "plugin name");
    assert_valid_tool_namespace(tool_name, "tool name");
    let plugin = dotted_name(plugin_name);
    let tool = dotted_name(tool_name);
    if tool.starts_with(format!("{plugin}.").as_str()) {
        return tool;
    }
    format!("{plugin}.{tool}")
}

pub fn model_tool_name_segment(value: &str) -> String {
    let trimmed = value.trim();
    let mut out = String::with_capacity(trimmed.len());
    let mut previous_was_separator = false;

    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
            previous_was_separator = false;
        } else if !previous_was_separator {
            out.push('_');
            previous_was_separator = true;
        }
    }

    while out.ends_with('_') {
        out.pop();
    }
    while out.starts_with('_') {
        out.remove(0);
    }
    if out.is_empty() {
        out.push_str("tool");
    }
    if out
        .bytes()
        .next()
        .is_some_and(|byte| !byte.is_ascii_alphabetic() && byte != b'_')
    {
        out.insert(0, '_');
    }
    out
}

fn dotted_name(value: &str) -> String {
    value
        .trim()
        .split('.')
        .map(model_tool_name_segment)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn assert_valid_tool_namespace(value: &str, label: &str) {
    let trimmed = value.trim();
    assert!(!trimmed.is_empty(), "{label} cannot be empty");
    assert!(!trimmed.contains('/'), "{label} cannot contain `/`");
}

pub fn effective_capabilities(
    manifest_capabilities: &[HostCapability],
    tool_capabilities: &[HostCapability],
) -> Vec<HostCapability> {
    let mut capabilities = Vec::new();
    for capability in manifest_capabilities
        .iter()
        .chain(tool_capabilities.iter())
        .copied()
    {
        if !capabilities.contains(&capability) {
            capabilities.push(capability);
        }
    }
    capabilities
}

pub fn effective_capabilities_for_manifest(
    tools: &[ToolDefinition],
    manifest_capabilities: &[HostCapability],
) -> Vec<HostCapability> {
    let mut capabilities = manifest_capabilities.to_vec();
    for tool in tools {
        for capability in &tool.capabilities {
            if !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
    }
    capabilities
}

pub fn per_tool_capabilities(tools: &[ToolDefinition]) -> BTreeMap<String, Vec<HostCapability>> {
    tools
        .iter()
        .filter(|tool| !tool.capabilities.is_empty())
        .map(|tool| (tool.name.clone(), tool.capabilities.clone()))
        .collect()
}

impl Default for PluginToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
