use crate::tool::ToolExecutor;
use agena_domain::{AccessKind, PermissionDecision, ToolInvocation};
use agena_skills::{DeclaredSkill, SkillManager};
use std::sync::Arc;

impl ToolExecutor {
    /// The same Skill catalog and path policy used for explicit Skill reads.
    /// Loading a profile's instructions has no interactive approval workflow.
    pub async fn profile_skill_documents(
        &self,
        names: &[String],
    ) -> Result<Vec<Result<String, String>>, String> {
        let invocation = ToolInvocation::new("agena.skills.get", Default::default());
        if !matches!(
            self.authorize_invocation(&invocation),
            Ok((_, PermissionDecision::Allow))
        ) {
            return Err("Preloading Skills requires read access to agena.skills.get".into());
        }
        let layout = agena_storage_layout::StorageLayout::current();
        let global = self.is_global();
        let workspace = if global {
            layout.root().join("global-skill-workspace")
        } else {
            self.workspace_root().to_path_buf()
        };
        let plugins = self.plugin_manager().plugins();
        let discovery = plugins
            .iter()
            .find(|plugin| plugin.key().to_string() == "agena.skills")
            .and_then(|plugin| {
                serde_json::from_value(plugin.configured_plugin().settings.clone()).ok()
            })
            .unwrap_or_default();
        let declared = plugins
            .iter()
            .filter(|plugin| self.plugin_is_enabled(&plugin.key()))
            .flat_map(|plugin| {
                plugin.manifest.skills.iter().map(|skill| DeclaredSkill {
                    plugin_id: plugin.key().to_string(),
                    version: plugin.manifest.version.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    instructions: skill.instructions.clone(),
                    aliases: skill.aliases.clone(),
                })
            })
            .collect();
        let principal = self.principal().clone();
        let root = self.workspace_root().to_path_buf();
        let names = names.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut manager = SkillManager::new(workspace, layout.root().join("skills"))
                .with_declared_skills(declared);
            if global {
                manager = manager.with_instance_only_roots();
            }
            let manager = manager
                .with_discovery_settings(discovery)
                .map_err(|error| error.message)?
                .with_read_filter(Arc::new(move |path| {
                    matches!(
                        principal.authorize_path_access(AccessKind::Read, &root, path),
                        PermissionDecision::Allow
                    )
                }));
            let catalog = manager.list().map_err(|error| error.message)?;
            Ok(names
                .into_iter()
                .map(|name| {
                    let skill = catalog
                        .skills
                        .iter()
                        .find(|skill| {
                            skill.name == name && skill.enabled && skill.valid && !skill.shadowed
                        })
                        .ok_or_else(|| {
                            format!(
                                "Skill {name:?} is missing, disabled, or outside the readable scope"
                            )
                        })?;
                    manager
                        .get(&skill.id)
                        .map(|detail| detail.document)
                        .map_err(|error| error.message)
                })
                .collect())
        })
        .await
        .map_err(|error| error.to_string())?
    }
}
