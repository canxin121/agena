use abi_stable::StableAbi;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, StableAbi)]
#[serde(rename_all = "snake_case")]
pub enum ToolBehavior {
    ReadOnly,
    Mutating,
    Task,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolSource {
    Builtin,
    Plugin { plugin_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolLoadPriority {
    Always,
    #[default]
    Standard,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub behavior: ToolBehavior,
    pub source: ToolSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_terms: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub concurrency_safe: bool,
    #[serde(default)]
    pub requires_user_interaction: bool,
    #[serde(default)]
    pub load_priority: ToolLoadPriority,
}

impl ToolDefinition {
    pub fn builtin<T>(name: &str, description: &str, behavior: ToolBehavior) -> Self
    where
        T: JsonSchema,
    {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema: json_schema_for::<T>(),
            behavior,
            source: ToolSource::Builtin,
            search_terms: Vec::new(),
            read_only: behavior == ToolBehavior::ReadOnly,
            concurrency_safe: behavior == ToolBehavior::ReadOnly,
            requires_user_interaction: false,
            load_priority: ToolLoadPriority::Standard,
        }
    }

    pub fn plugin(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
        behavior: ToolBehavior,
        plugin_name: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: sanitize_schema_json(input_schema),
            behavior,
            source: ToolSource::Plugin {
                plugin_name: plugin_name.into(),
            },
            search_terms: Vec::new(),
            read_only: behavior == ToolBehavior::ReadOnly,
            concurrency_safe: behavior == ToolBehavior::ReadOnly,
            requires_user_interaction: false,
            load_priority: ToolLoadPriority::Standard,
        }
    }

    pub fn with_search_terms<I, S>(mut self, search_terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.search_terms = search_terms
            .into_iter()
            .map(Into::into)
            .filter(|term| !term.trim().is_empty())
            .collect();
        self
    }

    pub fn with_concurrency_safe(mut self, concurrency_safe: bool) -> Self {
        self.concurrency_safe = concurrency_safe;
        self
    }

    pub fn with_requires_user_interaction(mut self, requires_user_interaction: bool) -> Self {
        self.requires_user_interaction = requires_user_interaction;
        self
    }

    pub fn with_load_priority(mut self, load_priority: ToolLoadPriority) -> Self {
        self.load_priority = load_priority;
        self
    }

    pub fn with_deferred_loading(self) -> Self {
        self.with_load_priority(ToolLoadPriority::Deferred)
    }

    pub fn with_always_load(self) -> Self {
        self.with_load_priority(ToolLoadPriority::Always)
    }

    pub fn should_load_by_default(&self) -> bool {
        !matches!(self.load_priority, ToolLoadPriority::Deferred)
    }

    pub fn is_deferred(&self) -> bool {
        matches!(self.load_priority, ToolLoadPriority::Deferred)
    }
}

pub fn json_schema_for<T>() -> serde_json::Value
where
    T: JsonSchema,
{
    let mut value = serde_json::to_value(schema_for!(T))
        .expect("schemars should always serialize generated schema");
    if let Some(object) = value.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    sanitize_schema_json(value)
}

fn sanitize_schema_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut object) => {
            object.remove("$schema");
            object.remove("title");
            let cleaned = object
                .into_iter()
                .map(|(key, value)| (key, sanitize_schema_json(value)))
                .collect();
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sanitize_schema_json).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{BashToolInput, ReadToolInput};

    #[test]
    fn builtin_definition_defaults_to_behavior_derived_metadata() {
        let definition = ToolDefinition::builtin::<ReadToolInput>(
            "inspect",
            "Inspect project state.",
            ToolBehavior::ReadOnly,
        );

        assert!(definition.read_only);
        assert!(definition.concurrency_safe);
        assert_eq!(definition.load_priority, ToolLoadPriority::Standard);
        assert!(definition.should_load_by_default());
        assert!(!definition.is_deferred());
    }

    #[test]
    fn builder_helpers_customize_loading_and_search_terms() {
        let definition = ToolDefinition::builtin::<BashToolInput>(
            "bash",
            "Run a shell command.",
            ToolBehavior::Mutating,
        )
        .with_search_terms(["shell", "", "terminal"])
        .with_concurrency_safe(false)
        .with_requires_user_interaction(true)
        .with_deferred_loading();

        assert_eq!(definition.search_terms, vec!["shell", "terminal"]);
        assert!(!definition.concurrency_safe);
        assert!(definition.requires_user_interaction);
        assert_eq!(definition.load_priority, ToolLoadPriority::Deferred);
        assert!(!definition.should_load_by_default());
        assert!(definition.is_deferred());
    }
}
