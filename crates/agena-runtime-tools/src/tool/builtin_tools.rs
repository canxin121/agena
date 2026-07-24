use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::ToolTag;

use agena_tool::BuiltinToolProfile;

#[derive(Debug, Clone)]
pub struct BuiltinToolSet {
    profile: BuiltinToolProfile,
}

impl BuiltinToolSet {
    pub fn for_model(model_id: Option<&str>) -> Self {
        Self {
            profile: BuiltinToolProfile::infer(model_id),
        }
    }

    pub fn is_tool_enabled(&self, tool: &RegisteredTool) -> bool {
        // Tool API handlers are protocol transport, not authority-bearing
        // execution tools. Keep all five functions available and enforce the
        // model profile on the execution tool selected inside `tools_call`.
        if crate::tool::is_tool_api_handler(tool) {
            return true;
        }
        self.are_tags_enabled(&tool.effective_tags())
    }

    pub fn are_tags_enabled(&self, tags: &[ToolTag]) -> bool {
        match self.profile {
            BuiltinToolProfile::Full => true,
            BuiltinToolProfile::ReadOnly => tags.iter().any(|tag| tag == &ToolTag::ReadOnly),
            BuiltinToolProfile::NoTask => !tags.iter().any(|tag| tag == &ToolTag::Task),
        }
    }
}
