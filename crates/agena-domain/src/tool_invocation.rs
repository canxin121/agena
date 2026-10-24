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
