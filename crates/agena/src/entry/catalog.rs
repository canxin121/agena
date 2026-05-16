use crate::agent::Agent;
use crate::plugin::registry::PluginEntry as RegistryPluginEntry;
use crate::plugin::sdk::ToolTag;

use crate::plugin::sdk::Plugin;
use crate::plugins::bundled::{
    cron as bundled_cron, fs as bundled_fs, lsp as bundled_lsp, shell as bundled_shell,
    web as bundled_web, workflow as bundled_workflow,
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
        match self.profile {
            ModelToolProfile::Full => true,
            ModelToolProfile::ReadOnly => tool.has_tag(ToolTag::ReadOnly),
            ModelToolProfile::NoTask => !tool.has_tag(ToolTag::Task),
        }
    }
}

fn tool_decls() -> Vec<crate::plugin::sdk::PluginToolDecl> {
    let mut decls = Vec::new();
    decls.extend(bundled_lsp::LspPlugin::new().manifest().entries);
    decls.extend(bundled_cron::CronPlugin::new().manifest().entries);
    decls.extend(bundled_fs::new_plugin().manifest().entries);
    decls.extend(bundled_shell::new_plugin().manifest().entries);
    decls.extend(bundled_web::new_plugin().manifest().entries);
    decls.extend(bundled_workflow::new_plugin().manifest().entries);
    decls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_catalog_marks_read_tools_as_always_loaded() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.tools();

        let read = definitions
            .iter()
            .find(|tool| tool.exposed_name == "read")
            .expect("read tool should exist");
        let grep = definitions
            .iter()
            .find(|tool| tool.exposed_name == "grep")
            .expect("grep tool should exist");

        assert!(read.has_tag(ToolTag::ReadOnly));
        assert!(read.decl.concurrency_safe);
        assert!(!read.is_deferred());
        assert!(grep.should_load_by_default());
    }

    #[test]
    fn tool_catalog_defers_mutating_and_task_tools() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.tools();

        for tool_name in ["bash", "apply_patch", "task", "notebook_edit", "powershell"] {
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
            "third_party",
            crate::plugin::sdk::PluginToolDecl::new(
                "third_party_read",
                serde_json::json!({"type": "object"}),
            )
            .description("read from a third-party plugin")
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
            "third_party",
            crate::plugin::sdk::PluginToolDecl::new(
                "third_party_task",
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
