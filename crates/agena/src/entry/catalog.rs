use crate::agent::Agent;
use crate::plugin::registry::PluginEntry as RegistryPluginEntry;
use crate::plugin::sdk::ToolTag;

use crate::plugin::sdk::Plugin;
use crate::plugins::provided::{
    cron as provided_cron, fs as provided_fs, lsp as provided_lsp, settings as provided_settings,
    shell as provided_shell, web as provided_web, workflow as provided_workflow,
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
        tool: &RegistryPluginEntry,
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

    pub fn tools(&self) -> Vec<RegistryPluginEntry> {
        tool_decls()
            .into_iter()
            .map(|decl| RegistryPluginEntry::new(decl.name.clone(), decl))
            .filter(|tool| self.is_tool_enabled(tool))
            .collect()
    }

    pub fn is_tool_enabled(&self, tool: &RegistryPluginEntry) -> bool {
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

fn tool_decls() -> Vec<crate::plugin::sdk::PluginToolDecl> {
    let mut decls = Vec::new();
    decls.extend(provided_lsp::LspPlugin::new().manifest().entries);
    decls.extend(provided_cron::CronPlugin::new().manifest().entries);
    decls.extend(provided_fs::new_plugin().manifest().entries);
    decls.extend(provided_settings::SettingsPlugin::new().manifest().entries);
    decls.extend(provided_shell::new_plugin().manifest().entries);
    decls.extend(provided_web::new_plugin().manifest().entries);
    decls.extend(provided_workflow::new_plugin().manifest().entries);
    decls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_catalog_marks_read_tools_as_always_loaded() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.tools();

        let fs = definitions
            .iter()
            .find(|tool| tool.exposed_name == "fs")
            .expect("fs tool should exist");

        assert!(fs.has_tag(ToolTag::ReadOnly));
        assert!(fs.decl.concurrency_safe);
        assert!(!fs.is_deferred());
        assert!(fs.should_load_by_default());
    }

    #[test]
    fn tool_catalog_defers_mutating_and_task_tools() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.tools();

        for tool_name in ["shell", "task"] {
            let definition = definitions
                .iter()
                .find(|tool| tool.exposed_name == tool_name)
                .unwrap_or_else(|| panic!("missing tool definition for {tool_name}"));
            assert!(definition.is_deferred(), "{tool_name} should be deferred");
        }
    }

    #[test]
    fn readonly_profile_filters_by_tags_not_name() {
        let catalog = ToolCatalog::for_model(Some("readonly-model"));
        let agent = Agent::new("test", crate::permission::PermissionPolicy::allow_all());
        let readonly_plugin = RegistryPluginEntry::new(
            "fixture_read",
            crate::plugin::sdk::PluginToolDecl::new(
                "fixture_read",
                serde_json::json!({"type": "object"}),
            )
            .description("read from a fixture plugin")
            .tags([ToolTag::ReadOnly, ToolTag::Network])
            .concurrency_safe(true),
        );
        let mutating_tool = RegistryPluginEntry::new(
            "fixture",
            crate::plugin::sdk::PluginToolDecl::new(
                "apply_patch",
                serde_json::json!({"type": "object"}),
            )
            .description("patch files")
            .tags([ToolTag::Mutating, ToolTag::FilesystemWrite])
            .concurrency_safe(false),
        );
        let task_plugin = RegistryPluginEntry::new(
            "fixture_task",
            crate::plugin::sdk::PluginToolDecl::new(
                "fixture_task",
                serde_json::json!({"type": "object"}),
            )
            .description("delegate work")
            .tags([ToolTag::Task, ToolTag::Subtask])
            .concurrency_safe(false),
        );

        assert!(
            catalog
                .availability_for_definition(&agent, &readonly_plugin)
                .enabled
        );
        assert!(
            !catalog
                .availability_for_definition(&agent, &mutating_tool)
                .enabled
        );
        assert!(
            !catalog
                .availability_for_definition(&agent, &task_plugin)
                .enabled
        );
    }
}
