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
    Plugin {
        plugin_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub behavior: ToolBehavior,
    pub source: ToolSource,
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
        }
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
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items.into_iter().map(sanitize_schema_json).collect(),
        ),
        other => other,
    }
}
