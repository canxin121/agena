use serde::{Deserialize, Serialize};

use crate::{StructuredObject, ToolApiFunction};

/// A dynamic tool invocation: stable name plus a structured payload.
///
/// This is a durable, provider-neutral value. Concrete tool execution and
/// message presentation deliberately live outside the domain crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolInvocation {
    /// Provider-facing Tool API identity for calls that originated from a
    /// model function call. Direct execution-tool runs leave this unset. When
    /// set, `name` is the same exact underscore-form protocol name and
    /// `plugin_name` remains unset.
    #[serde(
        default,
        rename = "gateway_function",
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_api_function: Option<ToolApiFunction>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default)]
    pub input: StructuredObject,
}

impl ToolInvocation {
    pub fn new(name: impl Into<String>, input: StructuredObject) -> Self {
        Self {
            tool_api_function: None,
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
            tool_api_function: None,
            name: name.into(),
            plugin_name: Some(plugin_name.into()),
            input,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToolInvocation;
    use crate::StructuredObject;

    #[test]
    fn direct_and_plugin_invocations_preserve_their_stable_identity() {
        let direct = ToolInvocation::new("read", StructuredObject::default());
        assert_eq!(direct.name, "read");
        assert_eq!(direct.plugin_name, None);
        assert_eq!(direct.tool_api_function, None);

        let plugin = ToolInvocation::plugin_named("search", "example", StructuredObject::default());
        assert_eq!(plugin.name, "search");
        assert_eq!(plugin.plugin_name.as_deref(), Some("example"));
    }

    #[test]
    fn removed_direct_provider_identity_does_not_deserialize() {
        let error = serde_json::from_value::<ToolInvocation>(serde_json::json!({
            "name": "fs.read",
            "provider_function_name": "fs_read",
            "input": {}
        }))
        .expect_err("removed direct provider identity must be rejected");
        assert!(error.to_string().contains("provider_function_name"));
    }
}
