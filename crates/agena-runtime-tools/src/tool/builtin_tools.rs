use agena_plugin_host::registry::RegisteredTool;

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
        // Tool API handlers are protocol transport, not authority-bearing
        // execution tools. Keep all five functions available and enforce the
        // model profile on the execution tool selected inside `tools_call`.
        if crate::tool::is_tool_api_handler(tool) {
            return true;
        }
        self.is_contract_enabled(&tool.definition.permissions)
    }

    /// Model-profile gating reads the permission contract directly: the
    /// declared `read_only` and `task` flags are authority-bearing and never
    /// come from tags.
    pub fn is_contract_enabled(
        &self,
        contract: &agena_plugin_host::sdk::ToolPermissionContract,
    ) -> bool {
        match self.profile {
            BuiltinToolProfile::Full => true,
            BuiltinToolProfile::ReadOnly => {
                contract.read_only && !contract.shell && !contract.interactive
            }
            BuiltinToolProfile::NoTask => !contract.task,
        }
    }
}
