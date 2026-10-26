//! Registry of tools published by loaded plugins.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::effect_scope::PluginEffectScope;
use crate::registration_owner::{RegistrationId, RegistrationOwner};
use crate::sdk::{PluginKey, ToolDefinition, ToolKey, ToolTag};

pub fn validate_tool_definition(
    plugin: &PluginKey,
    definition: &ToolDefinition,
) -> Result<ToolKey, String> {
    RegisteredTool::new(plugin.clone(), definition.clone()).map(|tool| tool.key)
}

fn validate_tool_key(
    plugin_key: &PluginKey,
    definition: &ToolDefinition,
) -> Result<ToolKey, String> {
    if plugin_key.namespace() == "agena" && plugin_key.name() == "tools" {
        return Err("agena.tools is reserved for the runtime Tool API protocol and cannot register plugin tools".into());
    }
    if definition.name.trim() != definition.name {
        return Err(format!(
            "tool name `{}` must not contain leading or trailing whitespace",
            definition.name
        ));
    }
    ToolKey::new(plugin_key.clone(), definition.name.clone())
        .map_err(|error| format!("invalid tool name `{}`: {error}", definition.name))
}

#[cfg(test)]
mod protocol_boundary_tests {
    use super::*;

    #[test]
    fn protocol_namespace_cannot_publish_plugin_tools() {
        let definition = ToolDefinition {
            name: "list".into(),
            contract: crate::sdk::ToolContract {
                input_schema: serde_json::json!({"type":"object"}),
                ..Default::default()
            },
            docs: Default::default(),
            runtime: Default::default(),
            tags: vec![],
        };
        let protocol = PluginKey::new("agena", "tools").unwrap();
        assert!(RegisteredTool::new(protocol, definition.clone()).is_err());
        let ordinary = PluginKey::new("agena", "fs").unwrap();
        assert!(RegisteredTool::new(ordinary, definition).is_ok());
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use serde_json::json;

    fn definition(input: serde_json::Value, output: serde_json::Value) -> ToolDefinition {
        ToolDefinition {
            name: "run".into(),
            contract: crate::sdk::ToolContract {
                input_schema: input,
                output_schema: output,
            },
            docs: Default::default(),
            runtime: Default::default(),
            tags: vec![],
        }
    }

    fn plugin() -> PluginKey {
        "test.contract".parse().unwrap()
    }

    #[test]
    fn compiled_contracts_validate_aliases_and_business_payloads() {
        let tool = RegisteredTool::new(plugin(), definition(
            json!({"type":"object","properties":{"count":{"type":"integer","minimum":1,"x-agena-aliases":["n"]}},"required":["count"],"additionalProperties":false}),
            json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),
        )).unwrap();
        assert!(tool.validate_input(&json!({"n":2})).is_ok());
        assert!(
            tool.validate_input(&json!({"count":0}))
                .unwrap_err()
                .contains("$/count")
        );
        assert!(tool.validate_input(&json!({"unknown":2})).is_err());
        assert!(tool.validate_output(Some(&json!({"ok":true}))).is_ok());
        assert!(
            tool.validate_output(Some(&json!({"payload":{"ok":true}})))
                .is_err()
        );
        assert!(tool.validate_output(None).is_err());
    }

    #[test]
    fn documentation_overrides_and_clones_share_compiled_contracts() {
        let tool = RegisteredTool::new(plugin(), definition(json!({"type":"object"}), json!(null)))
            .unwrap();
        let mut docs = tool.definition().clone();
        docs.docs.summary = Some("Updated help".into());
        let updated = tool.with_definition(docs).unwrap();
        assert!(Arc::ptr_eq(&tool.input_validator, &updated.input_validator));
        let cloned = updated.clone();
        assert!(Arc::ptr_eq(&updated.definition, &cloned.definition));
        let mut changed = updated.definition().clone();
        changed.contract.input_schema = json!({"type":"object","required":["new"]});
        let changed = updated.with_definition(changed).unwrap();
        assert!(!Arc::ptr_eq(
            &updated.input_validator,
            &changed.input_validator
        ));
        assert!(updated.validate_input(&json!({})).is_ok());
        assert!(changed.validate_input(&json!({})).is_err());
    }

    #[test]
    fn malformed_contract_rejects_the_whole_registration_batch() {
        let mut registry = PluginToolRegistry::new();
        let valid = definition(json!({"type":"object"}), json!(null));
        let mut invalid = definition(
            json!({"type":"object","properties":{"x":{"pattern":"["}}}),
            json!(null),
        );
        invalid.name = "bad".into();
        assert!(
            registry
                .extend_from_plugin(&plugin(), &[valid, invalid])
                .is_err()
        );
        assert!(registry.by_key.is_empty());
        assert_eq!(registry.generation, 0);
    }

    #[test]
    fn registration_validates_the_complete_input_and_output_schemas() {
        for schema in [
            json!("object"),
            json!([]),
            json!({"type":"not-a-json-type"}),
            json!({"properties":[]}),
            json!({"required":[1]}),
            json!({"properties":{"nested":{"minimum":"zero"}}}),
        ] {
            let input = RegisteredTool::new(plugin(), definition(schema.clone(), json!(null)));
            assert!(
                input.is_err(),
                "invalid input schema was accepted: {schema}"
            );
            let output = RegisteredTool::new(
                plugin(),
                definition(json!({"type":"object"}), schema.clone()),
            );
            assert!(
                output.is_err(),
                "invalid output schema was accepted: {schema}"
            );
        }
        let default_input =
            RegisteredTool::new(plugin(), definition(json!(null), json!(null))).unwrap();
        assert!(default_input.validate_input(&json!({})).is_ok());
        assert!(default_input.validate_input(&json!(null)).is_err());
        for schema in [json!(true), json!(false)] {
            assert!(RegisteredTool::new(plugin(), definition(schema.clone(), schema)).is_ok());
        }
    }

    #[test]
    fn deserialization_rebuilds_contract_and_rejects_identity_mismatch() {
        let tool = RegisteredTool::new(
            plugin(),
            definition(json!({"type":"object","required":["x"]}), json!(null)),
        )
        .unwrap();
        let value = serde_json::to_value(&tool).unwrap();
        let restored: RegisteredTool = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored, tool);
        assert!(restored.validate_input(&json!({})).is_err());
        let mut mismatched = value;
        mismatched["definition"]["name"] = json!("other");
        assert!(serde_json::from_value::<RegisteredTool>(mismatched).is_err());
    }
}

#[derive(Debug)]
/// Registry of tools provided by plugins.
pub struct PluginToolRegistry {
    by_key: BTreeMap<ToolKey, RegisteredTool>,
    by_canonical_name: BTreeMap<String, ToolKey>,
    generation: u64,
    owners: BTreeMap<ToolKey, RegistrationOwner>,
}

// A cloned registry is an independent snapshot. Its mutations must not
// release effects that still own values in the original live registry.
impl Clone for PluginToolRegistry {
    fn clone(&self) -> Self {
        Self {
            by_key: self.by_key.clone(),
            by_canonical_name: self.by_canonical_name.clone(),
            generation: self.generation,
            owners: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
/// Snapshot of the plugin tool registry.
pub struct ToolRegistrySnapshot {
    pub generation: u64,
    pub tools: Vec<RegisteredTool>,
}

#[derive(Debug, Clone)]
/// An immutable registered tool and its compiled execution contract.
pub struct RegisteredTool {
    key: ToolKey,
    definition: Arc<ToolDefinition>,
    input_validator: Arc<crate::schema::CompiledSchema>,
    output_validator: Option<Arc<crate::schema::CompiledSchema>>,
}

impl PartialEq for RegisteredTool {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.definition == other.definition
    }
}

impl Serialize for RegisteredTool {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("RegisteredTool", 2)?;
        state.serialize_field("key", &self.key)?;
        state.serialize_field("definition", self.definition.as_ref())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for RegisteredTool {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Data {
            key: ToolKey,
            definition: ToolDefinition,
        }
        let data = Data::deserialize(deserializer)?;
        let tool = Self::new(data.key.plugin().clone(), data.definition)
            .map_err(serde::de::Error::custom)?;
        if tool.key != data.key {
            return Err(serde::de::Error::custom(
                "registered tool key and definition name differ",
            ));
        }
        Ok(tool)
    }
}

impl RegisteredTool {
    pub fn new(plugin: PluginKey, definition: ToolDefinition) -> Result<Self, String> {
        let key = validate_tool_key(&plugin, &definition)?;
        let compile = |field, schema: &serde_json::Value| {
            crate::schema::CompiledSchema::new(schema)
                .map(Arc::new)
                .map_err(|error| format!("tool `{}` {field}: {error}", definition.name))
        };
        let input_validator = compile("input_schema", &definition.input_schema())?;
        let output_schema = definition.output_schema();
        let output_validator = (!output_schema.is_null())
            .then(|| compile("output_schema", &output_schema))
            .transpose()?;
        Ok(Self {
            key,
            definition: Arc::new(definition),
            input_validator,
            output_validator,
        })
    }

    /// Publish a replacement while retaining compiled validators for unchanged
    /// contracts. Definition hooks usually change documentation only.
    pub fn with_definition(&self, definition: ToolDefinition) -> Result<Self, String> {
        if definition.name != self.tool_name() {
            return Err("a definition override cannot rename its registered tool".into());
        }
        if definition.contract == self.definition.contract {
            return Ok(Self {
                key: self.key.clone(),
                definition: Arc::new(definition),
                input_validator: Arc::clone(&self.input_validator),
                output_validator: self.output_validator.clone(),
            });
        }
        Self::new(self.plugin_key().clone(), definition)
    }

    pub fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    pub fn into_definition(self) -> ToolDefinition {
        Arc::unwrap_or_clone(self.definition)
    }

    pub fn validate_input(&self, input: &serde_json::Value) -> Result<(), String> {
        self.input_validator.validate(input)
    }

    /// A declared output schema describes the business payload, never the
    /// presentation envelope. Missing payload is validated as JSON null.
    pub fn validate_output(&self, payload: Option<&serde_json::Value>) -> Result<(), String> {
        self.output_validator.as_ref().map_or(Ok(()), |validator| {
            validator.validate(payload.unwrap_or(&serde_json::Value::Null))
        })
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
            "streaming": self.definition.runtime.streaming,
            "tags": self.effective_tags(),
        });
        let bytes = match serde_json::to_vec(&value) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize plugin tool definition identity",
                        &error,
                    ),
                    "plugin tool definition identity is using a debug fallback"
                );
                format!("{value:?}").into_bytes()
            }
        };
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
            generation: 0,
            owners: BTreeMap::new(),
        }
    }

    pub fn extend_from_plugin(
        &mut self,
        plugin_key: &PluginKey,
        definitions: &[ToolDefinition],
    ) -> Result<(), String> {
        // Compile and validate the whole batch before publishing any tool.
        let tools = definitions
            .iter()
            .cloned()
            .map(|definition| RegisteredTool::new(plugin_key.clone(), definition))
            .collect::<Result<Vec<_>, _>>()?;
        for tool in tools {
            self.insert(tool, None);
        }
        Ok(())
    }

    pub fn upsert_from_plugin(
        &mut self,
        plugin_key: &PluginKey,
        definition: ToolDefinition,
    ) -> Result<RegisteredTool, String> {
        let tool = RegisteredTool::new(plugin_key.clone(), definition)?;
        Ok(self.insert(tool, None))
    }

    pub(crate) fn upsert_owned(
        &mut self,
        tool: RegisteredTool,
        owner: RegistrationOwner,
    ) -> RegisteredTool {
        self.insert(tool, Some(owner))
    }

    fn insert(&mut self, tool: RegisteredTool, owner: Option<RegistrationOwner>) -> RegisteredTool {
        if let Some(previous) = self.owners.remove(tool.tool_key()) {
            previous.release();
        }
        if let Some(owner) = owner {
            self.owners.insert(tool.tool_key().clone(), owner);
        }
        self.by_canonical_name
            .insert(tool.canonical_name(), tool.key.clone());
        self.by_key.insert(tool.key.clone(), tool.clone());
        self.generation += 1;
        tool
    }

    pub(crate) fn has_owner(&self, key: &ToolKey) -> bool {
        self.owners.contains_key(key)
    }

    pub(crate) fn assign_owner(&mut self, key: ToolKey, owner: RegistrationOwner) {
        debug_assert!(self.by_key.contains_key(&key));
        if let Some(previous) = self.owners.insert(key, owner) {
            previous.release();
        }
    }

    pub(crate) fn tools_owned_by(&self, owner: &PluginEffectScope) -> Vec<ToolDefinition> {
        self.owners
            .iter()
            .filter(|(_, entry)| entry.belongs_to(owner))
            .filter_map(|(key, _)| self.by_key.get(key))
            .map(|tool| tool.definition().clone())
            .collect()
    }

    pub(crate) fn remove_exact(&mut self, key: &ToolKey, id: &RegistrationId) {
        if self.owners.get(key).is_some_and(|owner| owner.matches(id)) {
            self.remove_from_plugin(key.plugin(), key.name());
        }
    }

    pub fn remove_from_plugin(
        &mut self,
        plugin_key: &PluginKey,
        tool_name: &str,
    ) -> Option<RegisteredTool> {
        let key = match ToolKey::new(plugin_key.clone(), tool_name.to_string()) {
            Ok(key) => key,
            Err(error) => {
                tracing::warn!(
                    plugin_id = %plugin_key,
                    tool_name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "construct tool key while removing a registered tool",
                        &error,
                    ),
                    "registered tool removal rejected an invalid tool name"
                );
                return None;
            }
        };
        let removed = self.by_key.remove(&key)?;
        if let Some(owner) = self.owners.remove(&key) {
            owner.release();
        }
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
                if let Some(owner) = self.owners.remove(&key) {
                    owner.release();
                }
                self.by_canonical_name
                    .remove(tool.canonical_name().as_str());
                removed.push(tool);
            }
        }
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

    /// Advance the shared catalog epoch for a mutation stored outside the
    /// global manifest registry (for example a session-scoped overlay).
    pub fn touch_generation(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
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
        let key = match ToolKey::new(plugin_key.clone(), tool_name.to_string()) {
            Ok(key) => key,
            Err(error) => {
                tracing::warn!(
                    plugin_id = %plugin_key,
                    tool_name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "construct tool key while looking up a registered tool",
                        &error,
                    ),
                    "registered tool lookup rejected an invalid tool name"
                );
                return None;
            }
        };
        self.by_key.get(&key)
    }
}

impl Default for PluginToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
