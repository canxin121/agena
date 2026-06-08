use crate::agent::Agent;
use crate::plugin::registry::RegisteredTool;
use crate::plugin::sdk::{PluginManifest, ToolTag};

use crate::plugin::sdk::Plugin;
use crate::plugins::provided::catalog as provided_catalog;
use crate::plugins::provided::code as provided_code;
use crate::plugins::provided::{
    cron as provided_cron, fs as provided_fs, lsp as provided_lsp, planning as provided_planning,
    repo as provided_repo, runtime as provided_runtime, schema_lab as provided_schema_lab,
    settings as provided_settings, shell as provided_shell, tasks as provided_tasks,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelToolProfile {
    Full,
    ReadOnly,
    NoTask,
}

impl ModelToolProfile {
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
pub struct ToolCatalog {
    profile: ModelToolProfile,
}

impl ToolCatalog {
    pub fn for_model(model_id: Option<&str>) -> Self {
        Self {
            profile: ModelToolProfile::infer(model_id),
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
                tool.exposed_name, self.profile
            )
        } else {
            format!(
                "tool '{}' disabled for {:?} profile",
                tool.exposed_name, self.profile
            )
        };
        ToolAvailability {
            tool_name: tool.exposed_name.clone(),
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
        self.are_tags_enabled(&tool.effective_tags())
    }

    pub fn are_tags_enabled(&self, tags: &[ToolTag]) -> bool {
        match self.profile {
            ModelToolProfile::Full => true,
            ModelToolProfile::ReadOnly => tags.iter().any(|tag| tag == &ToolTag::ReadOnly),
            ModelToolProfile::NoTask => !tags.iter().any(|tag| tag == &ToolTag::Task),
        }
    }
}

fn tool_entries() -> Vec<RegisteredTool> {
    let mut entries = Vec::new();
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
        provided_catalog::CatalogPlugin::new().manifest(),
    );
    extend_manifest_entries(
        &mut entries,
        provided_runtime::RuntimePlugin::new().manifest(),
    );
    extend_manifest_entries(
        &mut entries,
        provided_planning::PlanningPlugin::new().manifest(),
    );
    extend_manifest_entries(&mut entries, provided_tasks::TasksPlugin::new().manifest());
    extend_manifest_entries(&mut entries, provided_repo::RepoPlugin::new().manifest());
    extend_manifest_entries(&mut entries, crate::web::new_web_plugin().manifest());
    extend_manifest_entries(&mut entries, crate::memory::new_memory_plugin().manifest());
    if crate::tool::schema_lab_builtin_enabled() {
        extend_manifest_entries(
            &mut entries,
            provided_schema_lab::SchemaLabPlugin::new().manifest(),
        );
    }
    entries
}

fn extend_manifest_entries(entries: &mut Vec<RegisteredTool>, manifest: PluginManifest) {
    let plugin_name = manifest.name;
    entries.extend(
        manifest
            .tools
            .into_iter()
            .map(|decl| RegisteredTool::new(plugin_name.clone(), decl)),
    );
}
