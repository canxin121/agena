use crate::{StructuredObject, ToolInvocation};

/// Stable plugin-facing projection of a tool invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInvocation {
    pub tool_name: String,
    pub plugin_name: Option<String>,
    pub input: StructuredObject,
}

impl PluginInvocation {
    pub fn from_tool_invocation(invocation: &ToolInvocation) -> Self {
        Self {
            tool_name: invocation.name.clone(),
            plugin_name: invocation.plugin_name.clone(),
            input: invocation.input.clone(),
        }
    }
}
