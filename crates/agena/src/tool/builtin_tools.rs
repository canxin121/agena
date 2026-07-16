use crate::agent::Agent;
use crate::plugin::registry::RegisteredTool;
use crate::plugin::sdk::{PluginManifest, ToolTag};

use crate::plugin::sdk::Plugin;
use crate::plugins::provided::code as provided_code;
use crate::plugins::provided::tool_api as provided_tool_api;
use crate::plugins::provided::{
    agent as provided_agent, cron as provided_cron, fs as provided_fs,
    interaction as provided_interaction, lsp as provided_lsp, mcp as provided_mcp,
    planning as provided_planning, repo as provided_repo, schema_lab as provided_schema_lab,
    session as provided_session, settings as provided_settings, shell as provided_shell,
    skills as provided_skills, tasks as provided_tasks,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinToolProfile {
    Full,
    ReadOnly,
    NoTask,
}

impl BuiltinToolProfile {
    pub fn infer(model_id: Option<&str>) -> Self {
        let Some(model_id) = model_id else {
            return Self::Full;
        };
        let lowered = model_id.to_ascii_lowercase();
        if lowered.contains("readonly") || lowered.contains("read_only") {
            return Self::ReadOnly;
        }
        if lowered.contains("no-task") || lowered.contains("chat") {
            return Self::NoTask;
        }
        Self::Full
    }
}

#[derive(Debug, Clone)]
pub struct ToolAvailability {
    pub tool_name: String,
    pub enabled: bool,
    pub reason: String,
}

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

    pub fn availability_for_definition(
        &self,
        agent: &Agent,
        tool: &RegisteredTool,
    ) -> ToolAvailability {
        let enabled = self.is_tool_enabled(tool);
        let reason = if agent.disable {
            format!("agent '{}' is disabled", agent.name)
        } else if enabled {
            format!(
                "tool '{}' enabled for {:?} profile",
                crate::tool::compact_tool_call_name(tool.canonical_name().as_str()),
                self.profile
            )
        } else {
            format!(
                "tool '{}' disabled for {:?} profile",
                crate::tool::compact_tool_call_name(tool.canonical_name().as_str()),
                self.profile
            )
        };
        ToolAvailability {
            tool_name: crate::tool::compact_tool_call_name(tool.canonical_name().as_str()),
            enabled: enabled && !agent.disable,
            reason,
        }
    }

    pub fn tools(&self) -> Vec<RegisteredTool> {
        tool_entries()
            .into_iter()
            .filter(|tool| self.is_tool_enabled(tool))
            .collect()
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
    extend_manifest_entries(&mut entries, crate::web::new_web_plugin().manifest());
    extend_manifest_entries(&mut entries, crate::memory::new_memory_plugin().manifest());
    extend_manifest_entries(&mut entries, provided_mcp::static_manifest());
    if crate::tool::schema_lab_builtin_enabled() {
        extend_manifest_entries(
            &mut entries,
            provided_schema_lab::SchemaLabPlugin::new().manifest(),
        );
    }
    entries
}

fn extend_manifest_entries(entries: &mut Vec<RegisteredTool>, manifest: PluginManifest) {
    let plugin_key = crate::plugin::PluginKey::new(manifest.namespace, manifest.name)
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
