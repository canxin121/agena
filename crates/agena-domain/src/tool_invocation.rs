use serde::{Deserialize, Serialize};

use crate::{StructuredObject, ToolApiFunction};

/// The provider envelope that produced an execution invocation.
///
/// `arguments` is kept separately from [`ToolInvocation::input`] on purpose:
/// for `tools_call`, the provider arguments are `{ tool, input }`, while the
/// executable invocation is the selected `tool` and its inner `input` object.
/// This lets every runtime layer operate on the real target without losing the
/// exact provider call that must be replayed in model history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolApiCall {
    pub function: ToolApiFunction,
    #[serde(default)]
    pub arguments: StructuredObject,
}

/// A dynamic tool invocation: stable name plus a structured payload.
///
/// This is a durable, provider-neutral value. Concrete tool execution and
/// message presentation deliberately live outside the domain crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolInvocation {
    /// Exact provider call that originated this invocation. Direct
    /// application invocations leave this unset. In particular, a
    /// `tools_call` invocation stores the real execution-tool name and input
    /// below; the gateway is provenance, never an executable outer tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_api_call: Option<ToolApiCall>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default)]
    pub input: StructuredObject,
}

impl ToolInvocation {
    pub fn new(name: impl Into<String>, input: StructuredObject) -> Self {
        Self {
            tool_api_call: None,
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
            tool_api_call: None,
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
        assert_eq!(direct.tool_api_call, None);

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
