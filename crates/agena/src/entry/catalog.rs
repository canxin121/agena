use crate::agent::Agent;

use super::{EntryBehavior, EntryDefinition, EntrySource};
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
        definition: &EntryDefinition,
    ) -> ToolAvailability {
        let enabled = self.is_behavior_enabled(definition.behavior);
        let reason = if agent.disable {
            format!("agent '{}' is disabled", agent.name)
        } else if enabled {
            format!(
                "tool '{}' enabled for {:?} profile",
                definition.name, self.profile
            )
        } else {
            format!(
                "tool '{}' disabled for {:?} profile",
                definition.name, self.profile
            )
        };
        ToolAvailability {
            tool_name: definition.name.clone(),
            enabled: enabled && !agent.disable,
            reason,
        }
    }

    pub fn first_party_definitions(&self) -> Vec<EntryDefinition> {
        first_party_decls()
            .into_iter()
            .map(|decl| EntryDefinition::from_decl(decl.name.clone(), &decl, EntrySource::FirstParty))
            .filter(|definition| self.is_behavior_enabled(definition.behavior))
            .collect()
    }

    pub fn is_behavior_enabled(&self, behavior: EntryBehavior) -> bool {
        match self.profile {
            ModelToolProfile::Full => true,
            ModelToolProfile::ReadOnly => behavior == EntryBehavior::ReadOnly,
            ModelToolProfile::NoTask => behavior != EntryBehavior::Task,
        }
    }
}

fn first_party_decls() -> Vec<crate::plugin::sdk::PluginEntryDecl> {
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
    fn first_party_catalog_marks_read_tools_as_always_loaded() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.first_party_definitions();

        let read = definitions
            .iter()
            .find(|tool| tool.name == "read")
            .expect("read first-party should exist");
        let grep = definitions
            .iter()
            .find(|tool| tool.name == "grep")
            .expect("grep first-party should exist");

        assert!(read.read_only);
        assert!(read.concurrency_safe);
        assert!(!read.is_deferred());
        assert!(grep.should_load_by_default());
    }

    #[test]
    fn first_party_catalog_defers_mutating_and_task_tools() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.first_party_definitions();

        for tool_name in ["bash", "apply_patch", "task", "notebook_edit", "powershell"] {
            let definition = definitions
                .iter()
                .find(|tool| tool.name == tool_name)
                .unwrap_or_else(|| panic!("missing first-party definition for {tool_name}"));
            assert!(definition.is_deferred(), "{tool_name} should be deferred");
        }
    }

    #[test]
    fn readonly_profile_filters_by_behavior_not_name() {
        let catalog = ToolCatalog::for_model(Some("readonly-model"));
        let agent = Agent::new("test", crate::permission::PermissionPolicy::allow_all());
        let readonly_plugin = EntryDefinition::plugin(
            "third_party_read",
            "read from a third-party plugin",
            serde_json::json!({"type": "object"}),
            EntryBehavior::ReadOnly,
            "third_party",
        );
        let mutating_first_party = EntryDefinition::first_party::<crate::message::ApplyPatchToolInput>(
            "apply_patch",
            "patch files",
            EntryBehavior::Mutating,
        );
        let task_plugin = EntryDefinition::plugin(
            "third_party_task",
            "delegate work",
            serde_json::json!({"type": "object"}),
            EntryBehavior::Task,
            "third_party",
        );

        assert!(
            catalog
                .availability_for_definition(&agent, &readonly_plugin)
                .enabled
        );
        assert!(
            !catalog
                .availability_for_definition(&agent, &mutating_first_party)
                .enabled
        );
        assert!(
            !catalog
                .availability_for_definition(&agent, &task_plugin)
                .enabled
        );
    }
}
