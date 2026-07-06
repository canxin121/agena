//! Plugin tool name registry. Model-visible tool names use dotted names such as
//! `fs.read` and `plan.update`, with each dotted segment normalized to a
//! provider-friendly ASCII identifier.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sdk::manifest::UiTextDisplayMode;
use crate::sdk::{HostCapability, ToolDefinition, ToolDescriptionMode, ToolTag};

#[derive(Debug, Clone)]
pub struct PluginToolRegistry {
    /// `model_name -> tool`. `model_name` is the name shown to the model.
    by_model: BTreeMap<String, RegisteredTool>,
    /// `alias_model_name -> canonical_model_name`.
    aliases_by_model: BTreeMap<String, String>,
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
    pub tool_id: String,
    pub model_name: String,
    pub definition: ToolDefinition,
    pub target: ToolInvocationTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInvocationTarget {
    pub plugin_id: String,
    pub plugin_name: String,
    pub plugin_tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_input: Option<serde_json::Value>,
}

impl ToolBinding {
    pub fn new(plugin_name: impl Into<String>, definition: ToolDefinition) -> Self {
        let plugin_name = plugin_name.into();
        Self::new_with_plugin_id(plugin_name.clone(), plugin_name, definition)
    }

    pub fn new_with_plugin_id(
        plugin_id: impl Into<String>,
        plugin_name: impl Into<String>,
        definition: ToolDefinition,
    ) -> Self {
        let plugin_id = plugin_id.into();
        let plugin_name = plugin_name.into();
        let plugin_tool_name = definition.name.clone();
        let model_name = model_tool_name(&plugin_name, &plugin_tool_name);
        Self {
            tool_id: tool_id(&plugin_id, &plugin_tool_name, None),
            model_name,
            definition,
            target: ToolInvocationTarget {
                plugin_id,
                plugin_name,
                plugin_tool_name,
                fixed_input: None,
            },
        }
    }

    pub fn behavior_model_name(&self) -> String {
        if self.is_model_alias() {
            self.base_model_name()
        } else {
            self.model_name.clone()
        }
    }

    pub fn base_model_name(&self) -> String {
        model_tool_name(&self.target.plugin_name, &self.target.plugin_tool_name)
    }

    pub fn is_model_alias(&self) -> bool {
        self.model_name.as_str() != self.base_model_name()
    }

    pub fn with_model_alias(
        &self,
        alias_name: impl Into<String>,
        mut definition: ToolDefinition,
        fixed_input: serde_json::Value,
    ) -> Self {
        let alias_name = alias_name.into();
        definition.aliases.clear();
        let model_name = model_tool_name(&self.target.plugin_name, &alias_name);
        Self {
            tool_id: tool_id(
                &self.target.plugin_id,
                &self.target.plugin_tool_name,
                Some(&alias_name),
            ),
            model_name,
            definition,
            target: ToolInvocationTarget {
                plugin_id: self.target.plugin_id.clone(),
                plugin_name: self.target.plugin_name.clone(),
                plugin_tool_name: self.target.plugin_tool_name.clone(),
                fixed_input: Some(fixed_input),
            },
        }
    }

    pub fn with_tool_alias(
        &self,
        alias_name: impl Into<String>,
        mut definition: ToolDefinition,
    ) -> Self {
        let alias_name = alias_name.into();
        definition.aliases.clear();
        let model_name = model_tool_name(&self.target.plugin_name, &alias_name);
        Self {
            tool_id: tool_id(
                &self.target.plugin_id,
                &self.target.plugin_tool_name,
                Some(&alias_name),
            ),
            model_name,
            definition,
            target: ToolInvocationTarget {
                plugin_id: self.target.plugin_id.clone(),
                plugin_name: self.target.plugin_name.clone(),
                plugin_tool_name: self.target.plugin_tool_name.clone(),
                fixed_input: None,
            },
        }
    }

    pub fn alias_names(&self) -> impl Iterator<Item = &str> {
        self.definition.alias_texts().iter().map(String::as_str)
    }

    pub fn alias_model_names(&self) -> Vec<String> {
        self.alias_names()
            .map(|alias| model_tool_name(&self.target.plugin_name, alias))
            .collect()
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
            "tool_id": self.tool_id,
            "model_name": self.model_name,
            "target": self.target,
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

fn tool_id(plugin_id: &str, plugin_tool_name: &str, model_alias: Option<&str>) -> String {
    match model_alias {
        Some(alias) => format!("{plugin_id}/{plugin_tool_name}:{alias}"),
        None => format!("{plugin_id}/{plugin_tool_name}"),
    }
}

impl PluginToolRegistry {
    pub fn new() -> Self {
        Self {
            by_model: BTreeMap::new(),
            aliases_by_model: BTreeMap::new(),
            plugin_tool_defaults: BTreeMap::new(),
            plugin_ui_defaults: BTreeMap::new(),
            generation: 0,
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_id: &str,
        plugin_name: &str,
        definitions: &[ToolDefinition],
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
        for definition in definitions {
            self.upsert_from_plugin(plugin_id, plugin_name, definition.clone());
        }
    }

    pub fn upsert_from_plugin(
        &mut self,
        plugin_id: &str,
        plugin_name: &str,
        mut definition: ToolDefinition,
    ) -> RegisteredTool {
        assert_valid_tool_namespace(plugin_name, "plugin name");
        if definition.display.description_mode.is_none() {
            definition.display.description_mode = self.plugin_tool_defaults.get(plugin_id).copied();
        }
        if definition.display.ui_display_mode.is_none() {
            definition.display.ui_display_mode = self.plugin_ui_defaults.get(plugin_id).copied();
        }
        let plugin_tool_name = definition.name.clone();
        assert_valid_tool_namespace(&plugin_tool_name, "tool name");
        let mut tools = self.registered_tools_owned();
        tools.retain(|tool| {
            !(tool.target.plugin_id == plugin_id
                && tool.target.plugin_tool_name == plugin_tool_name)
        });
        tools.push(ToolBinding::new_with_plugin_id(
            plugin_id,
            plugin_name,
            definition,
        ));
        self.rebuild(tools);
        self.generation += 1;
        self.lookup_for_plugin(plugin_id, &plugin_tool_name)
            .expect("upserted tool should exist after rebuild")
            .clone()
    }

    pub fn remove_from_plugin(
        &mut self,
        plugin_id: &str,
        plugin_tool_name: &str,
    ) -> Option<RegisteredTool> {
        assert_valid_tool_namespace(plugin_tool_name, "tool name");
        let removed = self.lookup_for_plugin(plugin_id, plugin_tool_name)?.clone();
        let mut tools = self.registered_tools_owned();
        tools.retain(|tool| {
            !(tool.target.plugin_id == plugin_id
                && tool.target.plugin_tool_name == plugin_tool_name)
        });
        self.rebuild(tools);
        self.generation += 1;
        Some(removed)
    }

    pub fn remove_by_model_name_from_plugin(
        &mut self,
        plugin_id: &str,
        model_name: &str,
    ) -> Option<RegisteredTool> {
        let canonical_model_name = self
            .aliases_by_model
            .get(model_name)
            .map(String::as_str)
            .unwrap_or(model_name);
        let plugin_tool_name = self
            .by_model
            .get(canonical_model_name)
            .filter(|tool| tool.target.plugin_id == plugin_id)
            .map(|tool| tool.target.plugin_tool_name.clone())?;
        self.remove_from_plugin(plugin_id, &plugin_tool_name)
    }

    pub fn lookup_tool(&self, model_name: &str) -> Option<&RegisteredTool> {
        self.by_model.get(model_name).or_else(|| {
            let canonical = self.aliases_by_model.get(model_name)?;
            self.by_model.get(canonical)
        })
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
            assert_valid_tool_namespace(&tool.target.plugin_name, "plugin name");
            assert_valid_tool_namespace(&tool.target.plugin_tool_name, "tool name");
            tool.definition.aliases =
                normalize_tool_aliases(&tool.definition.name, &tool.definition.aliases);
            tool.model_name = model_tool_name(&tool.target.plugin_name, &tool.definition.name);
            tool.tool_id = tool_id(
                &tool.target.plugin_id,
                &tool.target.plugin_tool_name,
                (tool.definition.name != tool.target.plugin_tool_name)
                    .then_some(tool.definition.name.as_str()),
            );
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
        let mut aliases_by_model = BTreeMap::new();
        for tool in by_model.values() {
            for alias_model_name in tool.alias_model_names() {
                assert!(
                    !by_model.contains_key(&alias_model_name),
                    "tool alias `{alias_model_name}` for `{}` collides with a registered tool",
                    tool.model_name
                );
                if let Some(existing) =
                    aliases_by_model.insert(alias_model_name.clone(), tool.model_name.clone())
                {
                    panic!(
                        "tool alias `{alias_model_name}` for `{}` collides with alias for `{existing}`",
                        tool.model_name
                    );
                }
            }
        }
        self.by_model = by_model;
        self.aliases_by_model = aliases_by_model;
    }

    pub fn lookup_for_plugin(
        &self,
        plugin_id: &str,
        plugin_tool_name: &str,
    ) -> Option<&RegisteredTool> {
        self.by_model.values().find(|tool| {
            tool.target.plugin_id == plugin_id && tool.target.plugin_tool_name == plugin_tool_name
        })
    }
}

pub fn model_tool_name(plugin_name: &str, tool_name: &str) -> String {
    assert_valid_tool_namespace(plugin_name, "plugin name");
    assert_valid_tool_namespace(tool_name, "tool name");
    let dotted_tool_name = model_dotted_tool_name(tool_name);
    let plugin_prefix = model_plugin_prefix(plugin_name);
    if dotted_tool_name == plugin_prefix {
        return dotted_tool_name;
    }
    if builtin_tool_name_can_stand_alone(plugin_prefix.as_str(), dotted_tool_name.as_str()) {
        return dotted_tool_name;
    }
    if let Some(tail) = dotted_tool_name
        .strip_prefix(plugin_prefix.as_str())
        .and_then(|tail| tail.strip_prefix('.'))
    {
        return format!("{plugin_prefix}.{tail}");
    }
    format!("{plugin_prefix}.{dotted_tool_name}")
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

fn model_plugin_prefix(plugin_name: &str) -> String {
    let trimmed = plugin_name.trim();
    let trimmed = trimmed.strip_prefix("agena.").unwrap_or(trimmed);
    model_flat_tool_name(trimmed)
}

fn model_dotted_tool_name(value: &str) -> String {
    let parts = value
        .trim()
        .split('.')
        .map(model_tool_name_segment)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "tool".to_owned()
    } else {
        parts.join(".")
    }
}

fn model_flat_tool_name(value: &str) -> String {
    let parts = value
        .trim()
        .split('.')
        .map(model_tool_name_segment)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "tool".to_owned()
    } else {
        parts.join("_")
    }
}

fn builtin_tool_name_can_stand_alone(plugin_prefix: &str, tool_name: &str) -> bool {
    match plugin_prefix {
        "catalog" => {
            tool_name == "tools"
                || tool_name == "tool.help"
                || tool_name == "tool_catalog"
                || tool_name.starts_with("tools.")
        }
        "runtime" => {
            matches!(tool_name, "agent" | "session" | "user")
                || tool_name.starts_with("agent.")
                || tool_name.starts_with("session.")
                || tool_name.starts_with("user.")
        }
        "tasks" => tool_name == "task" || tool_name.starts_with("task."),
        "cron" => tool_name == "schedule" || tool_name.starts_with("schedule."),
        "mcp" => {
            tool_name.starts_with("resources.")
                || tool_name.starts_with("prompts.")
                || tool_name.starts_with("tools.")
        }
        _ => false,
    }
}

fn assert_valid_tool_namespace(value: &str, label: &str) {
    assert!(
        !value.trim().is_empty(),
        "{label} must not be empty for plugin tool exposure"
    );
    assert!(
        !value.contains('/'),
        "{label} `{value}` must not contain `/`; model-visible tool names use dotted segments"
    );
}

fn normalize_tool_aliases(tool_name: &str, aliases: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for alias in aliases {
        let alias = alias.trim();
        if alias.is_empty() || alias == tool_name {
            continue;
        }
        assert_valid_tool_namespace(alias, "tool alias");
        if !normalized.iter().any(|existing| existing == alias) {
            normalized.push(alias.to_string());
        }
    }
    normalized
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
            &[ToolDefinition::new(
                "inspect",
                serde_json::json!({ "type": "object" }),
            )],
            Some(ToolDescriptionMode::Brief),
            Some(UiTextDisplayMode::Summary),
        );

        let tool = registry
            .lookup_tool("fixture.inspect")
            .expect("tool should be registered");
        assert_eq!(
            tool.definition.display.description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(
            tool.definition.display.ui_display_mode,
            Some(UiTextDisplayMode::Summary)
        );
    }

    #[test]
    fn explicit_tool_mode_overrides_plugin_default() {
        let mut registry = PluginToolRegistry::new();
        registry.extend_from_plugin(
            "plugin-id",
            "fixture",
            &[
                ToolDefinition::new("inspect", serde_json::json!({ "type": "object" }))
                    .description_mode(ToolDescriptionMode::Detailed)
                    .ui_display_mode(UiTextDisplayMode::Detailed),
            ],
            Some(ToolDescriptionMode::Brief),
            Some(UiTextDisplayMode::Summary),
        );

        let tool = registry
            .lookup_tool("fixture.inspect")
            .expect("tool should be registered");
        assert_eq!(
            tool.definition.display.description_mode,
            Some(ToolDescriptionMode::Detailed)
        );
        assert_eq!(
            tool.definition.display.ui_display_mode,
            Some(UiTextDisplayMode::Detailed)
        );
    }

    #[test]
    fn tool_definition_identity_changes_when_contract_changes() {
        let mut registry = PluginToolRegistry::new();
        registry.extend_from_plugin(
            "plugin-id",
            "fixture",
            &[
                ToolDefinition::new("inspect", serde_json::json!({ "type": "object" }))
                    .output_schema(serde_json::json!({
                        "type": "object",
                        "properties": { "count": { "type": "integer" } }
                    })),
            ],
            None,
            None,
        );
        let first = registry
            .lookup_tool("fixture.inspect")
            .expect("tool should be registered")
            .definition_identity();

        registry.extend_from_plugin(
            "plugin-id",
            "fixture",
            &[
                ToolDefinition::new("inspect", serde_json::json!({ "type": "object" }))
                    .output_schema(serde_json::json!({
                        "type": "object",
                        "properties": { "count": { "type": "string" } }
                    })),
            ],
            None,
            None,
        );
        let second = registry
            .lookup_tool("fixture.inspect")
            .expect("tool should be registered")
            .definition_identity();

        assert_ne!(first, second);
    }

    #[test]
    fn tool_aliases_lookup_and_remove_canonical_tools() {
        let mut registry = PluginToolRegistry::new();
        registry.extend_from_plugin(
            "plugin-id",
            "fixture",
            &[
                ToolDefinition::new("inspect", serde_json::json!({ "type": "object" }))
                    .alias("i")
                    .aliases(["show", "i", " inspect "]),
            ],
            None,
            None,
        );

        let canonical = registry
            .lookup_tool("fixture.inspect")
            .expect("canonical tool should be registered");
        assert_eq!(canonical.model_name, "fixture.inspect");
        assert_eq!(
            canonical.alias_model_names(),
            vec!["fixture.i".to_string(), "fixture.show".to_string()]
        );

        let alias = registry
            .lookup_tool("fixture.i")
            .expect("alias should resolve to canonical tool");
        assert_eq!(alias.model_name, "fixture.inspect");
        assert_eq!(alias.target.plugin_tool_name, "inspect");

        let removed = registry
            .remove_by_model_name_from_plugin("plugin-id", "fixture.show")
            .expect("alias removal should remove canonical tool");
        assert_eq!(removed.model_name, "fixture.inspect");
        assert!(registry.lookup_tool("fixture.inspect").is_none());
        assert!(registry.lookup_tool("fixture.i").is_none());
    }

    #[test]
    fn model_tool_names_use_dotted_normalized_segments() {
        assert_eq!(model_tool_name("agena.plan", "plan.set"), "plan.set");
        assert_eq!(
            model_tool_name("streaming-fixture", "stream_fixture.count"),
            "streaming-fixture.stream_fixture.count"
        );
    }
}

pub fn effective_capabilities(definitions: &[ToolDefinition]) -> Vec<HostCapability> {
    let mut capabilities = Vec::new();
    for definition in definitions {
        for capability in &definition.capabilities {
            if !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
    }
    capabilities
}

/// Same as [`effective_capabilities`] but additionally folds in
/// manifest-level `plugin_capabilities`. Used by the host to authorize
/// plugins that need host capabilities without exposing any model-visible tool
/// (e.g. background skill discovery plugins).
pub fn effective_capabilities_for_manifest(
    definitions: &[ToolDefinition],
    plugin_capabilities: &[HostCapability],
) -> Vec<HostCapability> {
    let mut capabilities = effective_capabilities(definitions);
    for capability in plugin_capabilities {
        if !capabilities.contains(capability) {
            capabilities.push(*capability);
        }
    }
    capabilities
}

/// Per-tool capability map: each declared tool maps to its own
/// declared `capabilities` list. Used by [`HostHandle`] so that a
/// plugin shipping multiple tools can scope dangerous capabilities to
/// just the tool that needs them rather than leaking them via the union
/// to every tool the plugin owns.
pub fn per_tool_capabilities(
    definitions: &[ToolDefinition],
) -> std::collections::HashMap<String, Vec<HostCapability>> {
    let mut out = std::collections::HashMap::new();
    for definition in definitions {
        out.insert(definition.name.clone(), definition.capabilities.clone());
    }
    out
}
