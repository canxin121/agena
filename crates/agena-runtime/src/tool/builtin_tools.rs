use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::ToolTag;

#[cfg(test)]
use crate::plugins::provided::code as provided_code;
#[cfg(test)]
use crate::plugins::provided::tool_api as provided_tool_api;
#[cfg(test)]
use crate::plugins::provided::{
    agent as provided_agent, cron as provided_cron, fs as provided_fs,
    interaction as provided_interaction, lsp as provided_lsp, planning as provided_planning,
    repo as provided_repo, schema_lab as provided_schema_lab, session as provided_session,
    settings as provided_settings, shell as provided_shell, skills as provided_skills,
    tasks as provided_tasks,
};
#[cfg(test)]
use agena_plugin_host::sdk::Plugin;
#[cfg(test)]
use agena_plugin_host::sdk::PluginManifest;
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

#[cfg(test)]
fn tool_entries() -> Vec<RegisteredTool> {
    let mut entries = Vec::new();
    extend_manifest_entries(
        &mut entries,
        provided_skills::SkillsPlugin::new().manifest(),
    );
    extend_manifest_entries(&mut entries, provided_code::new_plugin().manifest());
    extend_manifest_entries(&mut entries, provided_lsp::LspPlugin::new().manifest());
    extend_manifest_entries(&mut entries, provided_cron::CronPlugin::new().manifest());
    extend_manifest_entries(&mut entries, provided_fs::new_plugin().manifest());
    extend_manifest_entries(
        &mut entries,
        provided_settings::SettingsPlugin::new().manifest(),
    );
    extend_manifest_entries(&mut entries, provided_shell::new_plugin().manifest());
    extend_manifest_entries(
        &mut entries,
        provided_tool_api::ToolApiPlugin::new().manifest(),
    );
    extend_manifest_entries(&mut entries, provided_agent::AgentPlugin::new().manifest());
    extend_manifest_entries(
        &mut entries,
        provided_session::SessionPlugin::new().manifest(),
    );
    extend_manifest_entries(
        &mut entries,
        provided_interaction::InteractionPlugin::new().manifest(),
    );
    extend_manifest_entries(
        &mut entries,
        provided_planning::PlanPlugin::new().manifest(),
    );
    extend_manifest_entries(&mut entries, provided_tasks::TasksPlugin::new().manifest());
    extend_manifest_entries(
        &mut entries,
        provided_repo::SnapshotPlugin::new().manifest(),
    );
    extend_manifest_entries(&mut entries, agena_runtime::new_web_plugin().manifest());
    extend_manifest_entries(&mut entries, agena_runtime::new_memory_plugin().manifest());
    if crate::tool::schema_lab_builtin_enabled() {
        extend_manifest_entries(
            &mut entries,
            provided_schema_lab::SchemaLabPlugin::new().manifest(),
        );
    }
    entries
}

#[cfg(test)]
fn extend_manifest_entries(entries: &mut Vec<RegisteredTool>, manifest: PluginManifest) {
    let plugin_key = agena_plugin_host::PluginKey::new(manifest.namespace, manifest.name)
        .expect("built-in plugin manifest key should be valid");
    entries.extend(manifest.tools.into_iter().map(|definition| {
        RegisteredTool::new(plugin_key.clone(), definition)
            .expect("built-in tool definition should be valid")
    }));
}

#[cfg(test)]
mod tests {
    use super::{BuiltinToolSet, tool_entries};

    #[test]
    fn read_only_profile_keeps_tool_api_but_filters_mutating_execution_tools() {
        let tools = tool_entries();
        let tools_call = tools
            .iter()
            .find(|tool| tool.canonical_name() == "agena.tools.call")
            .expect("tools_call API handler");
        let session_rename = tools
            .iter()
            .find(|tool| tool.canonical_name() == "agena.session.rename")
            .expect("mutating execution tool");
        let tool_set = BuiltinToolSet::for_model(Some("test-readonly-model"));

        assert!(tool_set.is_tool_enabled(tools_call));
        assert!(!tool_set.is_tool_enabled(session_rename));
    }
}
