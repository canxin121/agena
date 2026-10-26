use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::ToolTag;

use agena_tool::BuiltinToolProfile;

#[derive(Debug, Clone)]
/// Set of builtin tools.
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
        self.is_tag_set_enabled(&tool.effective_tags())
    }

    /// Model-profile gating reads the tool's declared tags. They are
    /// self-description — the authority to act is decided by the policy layer —
    /// and decide only which builtin profile offers the tool.
    pub fn is_tag_set_enabled(&self, tags: &[ToolTag]) -> bool {
        match self.profile {
            BuiltinToolProfile::Full => true,
            BuiltinToolProfile::ReadOnly => ToolTag::is_read_only(tags),
            BuiltinToolProfile::NoAgent => !ToolTag::is_agent(tags),
        }
    }
}
