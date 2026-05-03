use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, EntryLoadPriority as SdkEntryLoadPriority,
    PluginEntryDecl as SdkPluginEntryDecl,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryBehavior {
    ReadOnly,
    Mutating,
    Task,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntrySource {
    Builtin,
    Plugin { plugin_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryLoadPriority {
    Always,
    #[default]
    Standard,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub behavior: EntryBehavior,
    pub source: EntrySource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_terms: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub concurrency_safe: bool,
    #[serde(default)]
    pub requires_user_interaction: bool,
    #[serde(default)]
    pub load_priority: EntryLoadPriority,
    /// When true, instructs providers that support it (e.g. OpenAI) to enforce strict
    /// JSON schema validation on function call arguments.
    #[serde(default)]
    pub strict: bool,
}

impl EntryDefinition {
    pub fn builtin<T>(name: &str, description: &str, behavior: EntryBehavior) -> Self
    where
        T: JsonSchema,
    {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema: json_schema_for::<T>(),
            behavior,
            source: EntrySource::Builtin,
            search_terms: Vec::new(),
            read_only: behavior == EntryBehavior::ReadOnly,
            concurrency_safe: behavior == EntryBehavior::ReadOnly,
            requires_user_interaction: false,
            load_priority: EntryLoadPriority::Standard,
            strict: false,
        }
    }

    pub fn plugin(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
        behavior: EntryBehavior,
        plugin_name: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: sanitize_schema_json(input_schema),
            behavior,
            source: EntrySource::Plugin {
                plugin_name: plugin_name.into(),
            },
            search_terms: Vec::new(),
            read_only: behavior == EntryBehavior::ReadOnly,
            concurrency_safe: behavior == EntryBehavior::ReadOnly,
            requires_user_interaction: false,
            load_priority: EntryLoadPriority::Standard,
            strict: false,
        }
    }

    pub fn from_decl(
        name: impl Into<String>,
        decl: &SdkPluginEntryDecl,
        source: EntrySource,
    ) -> Self {
        let behavior = sdk_tool_behavior(decl.behavior);
        let read_only = behavior == EntryBehavior::ReadOnly;
        Self {
            name: name.into(),
            description: decl.description.clone().unwrap_or_default(),
            input_schema: sanitize_schema_json(decl.input_schema.clone()),
            behavior,
            source,
            search_terms: decl
                .search_terms
                .iter()
                .map(String::as_str)
                .filter(|term| !term.trim().is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            read_only,
            concurrency_safe: decl.concurrency_safe.unwrap_or(read_only),
            requires_user_interaction: decl.requires_user_interaction,
            load_priority: sdk_load_priority(decl.load_priority),
            strict: decl.strict,
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

    pub fn with_load_priority(mut self, load_priority: EntryLoadPriority) -> Self {
        self.load_priority = load_priority;
        self
    }

    pub fn with_deferred_loading(self) -> Self {
        self.with_load_priority(EntryLoadPriority::Deferred)
    }

    pub fn with_always_load(self) -> Self {
        self.with_load_priority(EntryLoadPriority::Always)
    }

    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    pub fn should_load_by_default(&self) -> bool {
        !matches!(self.load_priority, EntryLoadPriority::Deferred)
    }

    pub fn is_deferred(&self) -> bool {
        matches!(self.load_priority, EntryLoadPriority::Deferred)
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

fn sdk_tool_behavior(b: SdkEntryBehavior) -> EntryBehavior {
    match b {
        SdkEntryBehavior::ReadOnly => EntryBehavior::ReadOnly,
        SdkEntryBehavior::WriteSandboxed | SdkEntryBehavior::WriteUnsandboxed => {
            EntryBehavior::Mutating
        }
        SdkEntryBehavior::Task => EntryBehavior::Task,
    }
}

fn sdk_load_priority(priority: SdkEntryLoadPriority) -> EntryLoadPriority {
    match priority {
        SdkEntryLoadPriority::Always => EntryLoadPriority::Always,
        SdkEntryLoadPriority::Standard => EntryLoadPriority::Standard,
        SdkEntryLoadPriority::Deferred => EntryLoadPriority::Deferred,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{BashToolInput, ReadToolInput};
    use crate::plugin::sdk::{EntryLoadPriority as SdkEntryLoadPriority, PluginEntryDecl};

    #[test]
    fn builtin_definition_defaults_to_behavior_derived_metadata() {
        let definition = EntryDefinition::builtin::<ReadToolInput>(
            "inspect",
            "Inspect project state.",
            EntryBehavior::ReadOnly,
        );

        assert!(definition.read_only);
        assert!(definition.concurrency_safe);
        assert_eq!(definition.load_priority, EntryLoadPriority::Standard);
        assert!(definition.should_load_by_default());
        assert!(!definition.is_deferred());
    }

    #[test]
    fn builder_helpers_customize_loading_and_search_terms() {
        let definition = EntryDefinition::builtin::<BashToolInput>(
            "bash",
            "Run a shell command.",
            EntryBehavior::Mutating,
        )
        .with_search_terms(["shell", "", "terminal"])
        .with_concurrency_safe(false)
        .with_requires_user_interaction(true)
        .with_deferred_loading();

        assert_eq!(definition.search_terms, vec!["shell", "terminal"]);
        assert!(!definition.concurrency_safe);
        assert!(definition.requires_user_interaction);
        assert_eq!(definition.load_priority, EntryLoadPriority::Deferred);
        assert!(!definition.should_load_by_default());
        assert!(definition.is_deferred());
    }

    #[test]
    fn definition_from_decl_preserves_manifest_metadata() {
        let decl = PluginEntryDecl::new("search", serde_json::json!({"type": "object"}))
            .description("Search tools")
            .search_terms(["discover", "catalog"])
            .deferred_load()
            .concurrency_safe(false)
            .requires_user_interaction(true)
            .strict(true);

        let definition = EntryDefinition::from_decl("search", &decl, EntrySource::Builtin);

        assert_eq!(definition.description, "Search tools");
        assert_eq!(definition.search_terms, vec!["discover", "catalog"]);
        assert!(!definition.concurrency_safe);
        assert!(definition.requires_user_interaction);
        assert!(definition.strict);
        assert_eq!(definition.load_priority, EntryLoadPriority::Deferred);
    }

    #[test]
    fn definition_from_decl_keeps_readonly_default_concurrency() {
        let mut decl = PluginEntryDecl::new("read", serde_json::json!({"type": "object"}));
        decl.load_priority = SdkEntryLoadPriority::Always;

        let definition = EntryDefinition::from_decl("read", &decl, EntrySource::Builtin);

        assert!(definition.read_only);
        assert!(definition.concurrency_safe);
        assert_eq!(definition.load_priority, EntryLoadPriority::Always);
    }
}
