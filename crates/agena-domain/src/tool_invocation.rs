use serde::{Deserialize, Serialize};

use crate::StructuredObject;

/// A plugin execution invocation: stable target name plus its input payload.
/// Runtime protocol calls are represented separately by `ToolApiCall`.
///
/// This is a durable, provider-neutral value. Concrete tool execution and
/// message presentation deliberately live outside the domain crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolInvocation {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default)]
    pub input: StructuredObject,
}

impl ToolInvocation {
    pub fn new(name: impl Into<String>, input: StructuredObject) -> Self {
        Self {
            name: name.into(),
            plugin_name: None,
            input,
        }
    }

    pub fn plugin_named(
        name: impl Into<String>,
        plugin_name: impl Into<String>,
        input: StructuredObject,
    ) -> Self {
        Self {
            name: name.into(),
            plugin_name: Some(plugin_name.into()),
            input,
        }
    }
}
