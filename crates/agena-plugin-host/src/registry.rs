use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sdk::manifest::UiTextDisplayMode;
use crate::sdk::{
    HostCapability, PluginKey, ToolDefinition, ToolDescriptionMode, ToolKey, ToolTag,
};

pub fn validate_tool_definition(
    plugin_key: &PluginKey,
    definition: &ToolDefinition,
) -> Result<ToolKey, String> {
    if definition.name.trim() != definition.name {
        return Err(format!(
            "tool name `{}` must not contain leading or trailing whitespace",
            definition.name
        ));
    }
    let key = ToolKey::new(plugin_key.clone(), definition.name.clone())
        .map_err(|error| format!("invalid tool name `{}`: {error}", definition.name))?;
    validate_schema_shape(
        definition.name.as_str(),
        "input_schema",
        &definition.input_schema(),
        false,
    )?;
    validate_schema_shape(
        definition.name.as_str(),
        "output_schema",
        &definition.output_schema(),
        true,
    )?;
    Ok(key)
}

fn validate_schema_shape(
    tool_name: &str,
    field: &str,
    schema: &serde_json::Value,
    allow_null: bool,
) -> Result<(), String> {
    let schema_object = match schema {
        serde_json::Value::Null if allow_null => return Ok(()),
        serde_json::Value::Bool(_) => return Ok(()),
        serde_json::Value::Object(object) => object,
        _ => {
            return Err(format!(
                "tool `{tool_name}` {field} must be a JSON Schema object or boolean"
            ));
        }
    };
    if schema_object
        .get("properties")
        .is_some_and(|properties| !properties.is_object())
    {
        return Err(format!(
            "tool `{tool_name}` {field}.properties must be an object"
        ));
    }
    if schema_object.get("required").is_some_and(|required| {
        required
            .as_array()
            .is_none_or(|items| items.iter().any(|item| item.as_str().is_none()))
    }) {
        return Err(format!(
            "tool `{tool_name}` {field}.required must be an array of strings"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct PluginToolRegistry {
    by_key: BTreeMap<ToolKey, RegisteredTool>,
    by_canonical_name: BTreeMap<String, ToolKey>,
    plugin_tool_defaults: BTreeMap<PluginKey, ToolDescriptionMode>,
    plugin_ui_defaults: BTreeMap<PluginKey, UiTextDisplayMode>,
    generation: u64,
}

#[derive(Debug, Clone)]
pub struct ToolRegistrySnapshot {
    pub generation: u64,
    pub tools: Vec<RegisteredTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisteredTool {
    pub key: ToolKey,
    pub definition: ToolDefinition,
}

impl RegisteredTool {
    pub fn new(plugin: PluginKey, definition: ToolDefinition) -> Result<Self, String> {
        let key = validate_tool_definition(&plugin, &definition)?;
        Ok(Self { key, definition })
    }

    pub fn plugin_key(&self) -> &PluginKey {
        self.key.plugin()
    }

    pub fn tool_key(&self) -> &ToolKey {
        &self.key
    }

    pub fn namespace(&self) -> &str {
        self.key.plugin().namespace()
    }

    pub fn plugin_name(&self) -> &str {
        self.key.plugin().name()
    }

    pub fn tool_name(&self) -> &str {
        self.key.name()
    }

    pub fn plugin_full_name(&self) -> String {
        self.key.plugin().to_string()
    }

    /// Stable internal registry identity. This is not a provider function name.
    pub fn canonical_name(&self) -> String {
        self.key.to_string()
    }

    /// Legacy name retained for API compatibility. New provider-boundary code
    /// must use an explicit provider function specification instead.
    pub fn model_name(&self) -> String {
        self.canonical_name()
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

    pub fn input_schema(&self) -> serde_json::Value {
        self.definition.input_schema()
    }

    pub fn output_schema(&self) -> serde_json::Value {
        self.definition.output_schema()
    }

    pub fn definition_identity(&self) -> String {
        let value = serde_json::json!({
            "plugin": self.plugin_key(),
            "tool_name": self.tool_name(),
            // Keep the serialized identity field stable for persisted
            // advertised-tool fingerprints written before the terminology was
            // clarified.
            "model_name": self.canonical_name(),
            "summary": self.summary_text(),
            "input_schema": self.input_schema(),
            "output_schema": self.output_schema(),
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
            by_key: BTreeMap::new(),
            by_canonical_name: BTreeMap::new(),
            plugin_tool_defaults: BTreeMap::new(),
            plugin_ui_defaults: BTreeMap::new(),
            generation: 0,
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_key: &PluginKey,
        definitions: &[ToolDefinition],
        plugin_tool_default: Option<ToolDescriptionMode>,
        plugin_ui_default: Option<UiTextDisplayMode>,
    ) -> Result<(), String> {
        // Validate the whole batch before changing defaults or inserting any
        // tools. A bad manifest must never leave a partially updated registry.
        for definition in definitions {
            validate_tool_definition(plugin_key, definition)?;
        }
        if let Some(mode) = plugin_tool_default {
            self.plugin_tool_defaults.insert(plugin_key.clone(), mode);
        } else {
            self.plugin_tool_defaults.remove(plugin_key);
        }
        if let Some(mode) = plugin_ui_default {
            self.plugin_ui_defaults.insert(plugin_key.clone(), mode);
        } else {
            self.plugin_ui_defaults.remove(plugin_key);
        }
        for definition in definitions {
            self.upsert_from_plugin(plugin_key, definition.clone())?;
        }
        Ok(())
    }

    pub fn upsert_from_plugin(
        &mut self,
        plugin_key: &PluginKey,
        mut definition: ToolDefinition,
    ) -> Result<RegisteredTool, String> {
        if definition.display.description_mode.is_none() {
            definition.display.description_mode =
                self.plugin_tool_defaults.get(plugin_key).copied();
        }
        if definition.display.ui_display_mode.is_none() {
            definition.display.ui_display_mode = self.plugin_ui_defaults.get(plugin_key).copied();
        }
        let key = validate_tool_definition(plugin_key, &definition)?;
        self.by_canonical_name.remove(key.to_string().as_str());
        self.by_key.remove(&key);
        let tool = RegisteredTool { key, definition };
        self.by_canonical_name
            .insert(tool.canonical_name(), tool.key.clone());
        self.by_key.insert(tool.key.clone(), tool.clone());
        self.generation += 1;
        Ok(tool)
    }

    pub fn remove_from_plugin(
        &mut self,
        plugin_key: &PluginKey,
        tool_name: &str,
    ) -> Option<RegisteredTool> {
        let key = ToolKey::new(plugin_key.clone(), tool_name.to_string()).ok()?;
        let removed = self.by_key.remove(&key)?;
        self.by_canonical_name
            .remove(removed.canonical_name().as_str());
        self.generation += 1;
        Some(removed)
    }

    pub fn remove_plugin(&mut self, plugin_key: &PluginKey) -> Vec<RegisteredTool> {
        let keys = self
            .by_key
            .keys()
            .filter(|key| key.plugin() == plugin_key)
            .cloned()
            .collect::<Vec<_>>();
        let mut removed = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(tool) = self.by_key.remove(&key) {
                self.by_canonical_name
                    .remove(tool.canonical_name().as_str());
                removed.push(tool);
            }
        }
        self.plugin_tool_defaults.remove(plugin_key);
        self.plugin_ui_defaults.remove(plugin_key);
        if !removed.is_empty() {
            self.generation += 1;
        }
        removed
    }

    pub fn lookup_tool_by_canonical_name(&self, canonical_name: &str) -> Option<&RegisteredTool> {
        let key = self.by_canonical_name.get(canonical_name)?;
        self.by_key.get(key)
    }

    pub fn lookup_tool_by_key(&self, key: &ToolKey) -> Option<&RegisteredTool> {
        self.by_key.get(key)
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
        self.by_key.values().cloned().collect()
    }

    pub fn registered_tools(&self) -> impl Iterator<Item = &RegisteredTool> {
        self.by_key.values()
    }

    pub fn count(&self) -> usize {
        self.by_key.len()
    }

    pub fn lookup_for_plugin(
        &self,
        plugin_key: &PluginKey,
        tool_name: &str,
    ) -> Option<&RegisteredTool> {
        let key = ToolKey::new(plugin_key.clone(), tool_name.to_string()).ok()?;
        self.by_key.get(&key)
    }
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

#[cfg(test)]
mod tests {
    use super::PluginToolRegistry;
    use crate::sdk::{PluginKey, ToolDefinition};

    fn definition(name: &str) -> ToolDefinition {
        serde_json::from_value(serde_json::json!({ "name": name }))
            .expect("minimal tool definition")
    }

    #[test]
    fn removing_a_plugin_removes_only_its_tools() {
        let alpha = PluginKey::new("example", "alpha").expect("alpha key");
        let beta = PluginKey::new("example", "beta").expect("beta key");
        let mut registry = PluginToolRegistry::new();
        registry
            .extend_from_plugin(&alpha, &[definition("one"), definition("two")], None, None)
            .expect("alpha tools");
        registry
            .extend_from_plugin(&beta, &[definition("one")], None, None)
            .expect("beta tools");

        let removed = registry.remove_plugin(&alpha);

        assert_eq!(removed.len(), 2);
        assert_eq!(registry.count(), 1);
        assert!(
            registry
                .lookup_tool_by_canonical_name("example.beta.one")
                .is_some()
        );
    }
}
