//! Skill management shared by the Web and terminal clients. Model-driven tool
//! calls use the same manager through the permission-aware skills plugin.

use std::path::PathBuf;

use agena_api::skills::*;
use agena_skills::source::load_install_source;
use agena_skills::{DeclaredSkill, SkillError, SkillErrorKind, SkillManager};

use crate::{Application, ApplicationError};

impl Application {
    async fn skills_workspace_root(
        &self,
        workspace_id: Option<i64>,
    ) -> Result<std::path::PathBuf, ApplicationError> {
        match workspace_id {
            None => Ok(self.workspace_root().to_owned()),
            Some(id) => {
                let workspace = self.service().get_workspace(id).await?.ok_or_else(|| {
                    ApplicationError::not_found("The selected workspace no longer exists.")
                })?;
                Ok(std::path::PathBuf::from(workspace.path))
            }
        }
    }

    fn skill_manager(&self, workspace: PathBuf, mut declared: Vec<DeclaredSkill>) -> SkillManager {
        let global_root = agena_storage_layout::StorageLayout::current()
            .root()
            .join("skills");
        for status in self.plugin_runtime().plugin_statuses() {
            let plugin_id = status.plugin_id.to_string();
            let Some(plugin) = self
                .plugin_runtime()
                .plugin_inspect(&plugin_id)
                .filter(|plugin| plugin.authority.is_some())
            else {
                continue;
            };
            if let Some(manifest) = plugin.manifest {
                for skill in manifest.skills {
                    declared.push(DeclaredSkill {
                        plugin_id: plugin_id.clone(),
                        version: manifest.version.clone(),
                        name: skill.name,
                        description: skill.description,
                        instructions: skill.instructions,
                        aliases: skill.aliases,
                        invocation: None,
                        resource_base: None,
                    });
                }
            }
        }
        let additional = self
            .plugin_runtime()
            .plugin_inspect("agena.commands")
            .filter(|plugin| plugin.authority.is_some())
            .and_then(|plugin| plugin.configured_plugin)
            .and_then(|plugin| plugin.settings.get("additional_roots").cloned())
            .and_then(|roots| serde_json::from_value::<Vec<std::path::PathBuf>>(roots).ok())
            .unwrap_or_default();
        let disabled = self
            .plugin_runtime()
            .plugin_inspect("agena.commands")
            .filter(|plugin| plugin.authority.is_some())
            .and_then(|plugin| plugin.configured_plugin)
            .and_then(|plugin| plugin.settings.get("disabled").cloned())
            .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
            .unwrap_or_default();
        SkillManager::new(workspace, global_root)
            .with_additional_roots(additional)
            .with_exact_skill_dirs(self.plugin_runtime().agent_plugin_skill_dirs())
            .with_declared_skills(declared)
            .with_disabled_names(disabled)
    }

    async fn workspace_skill_manager(&self, workspace: PathBuf) -> SkillManager {
        let provided = self.plugin_runtime().agent_plugin_provided_skills(&workspace).await;
        self.skill_manager(workspace, provided)
    }

    pub async fn list_skills(
        &self,
        workspace_id: Option<i64>,
    ) -> Result<SkillCatalog, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| manager.list()).await?.map_err(skill_error)
    }

    pub async fn list_skills_for_session(&self, session_id: i64) -> Result<SkillCatalog, ApplicationError> {
        let scope = self.plugin_runtime().agent_plugin_session_skills(session_id)
            .await
            .map_err(ApplicationError::from_failure)?;
        let manager = match scope {
            Some((workspace, provided)) => self.skill_manager(workspace, provided),
            None => {
                let root = agena_storage_layout::StorageLayout::current().root().to_path_buf();
                SkillManager::new(root.join("global-skill-workspace"), root.join("skills"))
                    .with_instance_only_roots()
            }
        };
        self.run_blocking(move |_| manager.list()).await?.map_err(skill_error)
    }

    pub async fn get_skill(
        &self,
        workspace_id: Option<i64>,
        id: String,
    ) -> Result<SkillDetail, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| manager.get(&id))
            .await?
            .map_err(skill_error)
    }

    pub async fn create_skill(
        &self,
        workspace_id: Option<i64>,
        request: SkillCreateRequest,
    ) -> Result<SkillDetail, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| manager.create(request.scope, &request.document))
        .await?
        .map_err(skill_error)
    }

    pub async fn update_skill(
        &self,
        workspace_id: Option<i64>,
        id: String,
        request: SkillUpdateRequest,
    ) -> Result<SkillDetail, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        require_revision(&request.expected_revision)?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| {
            manager.update(
                &id,
                &request.document,
                Some(&request.expected_revision),
            )
        })
        .await?
        .map_err(skill_error)
    }

    pub async fn delete_skill(
        &self,
        workspace_id: Option<i64>,
        id: String,
        expected_revision: String,
    ) -> Result<SkillDeleteResult, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        require_revision(&expected_revision)?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| manager.remove(&id, Some(&expected_revision)))
        .await?
        .map(|skill| SkillDeleteResult {
            id: skill.id,
            removed: true,
        })
        .map_err(skill_error)
    }

    pub async fn set_skill_enabled(
        &self,
        workspace_id: Option<i64>,
        id: String,
        request: SkillEnabledRequest,
    ) -> Result<SkillDetail, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        require_revision(&request.expected_revision)?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| {
            manager.set_enabled(
                &id,
                request.enabled,
                Some(&request.expected_revision),
            )
        })
        .await?
        .map_err(skill_error)
    }

    pub async fn get_skill_resource(
        &self,
        workspace_id: Option<i64>,
        id: String,
        path: String,
    ) -> Result<SkillResource, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| manager.read_resource(&id, &path))
            .await?
            .map_err(skill_error)
    }

    pub async fn write_skill_resource(
        &self,
        workspace_id: Option<i64>,
        id: String,
        request: SkillResourceWriteRequest,
    ) -> Result<SkillResource, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| {
            manager.write_resource(
                &id,
                &request.path,
                &request.content,
                Some(request.expected_revision.as_deref().unwrap_or("")),
            )
        })
        .await?
        .map_err(skill_error)
    }

    pub async fn delete_skill_resource(
        &self,
        workspace_id: Option<i64>,
        id: String,
        path: String,
        expected_revision: String,
    ) -> Result<SkillDetail, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        require_revision(&expected_revision)?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| manager.remove_resource(&id, &path, Some(&expected_revision)))
        .await?
        .map_err(skill_error)
    }

    pub async fn validate_skill(
        &self,
        _workspace_id: Option<i64>,
        request: SkillValidationRequest,
    ) -> Result<SkillValidation, ApplicationError> {
        self.run_blocking(move |_| {
            agena_skills::validate_document(&request.document, request.name.as_deref())
        })
        .await
    }

    pub async fn preview_skill_install(
        &self,
        workspace_id: Option<i64>,
        request: SkillInstallRequest,
    ) -> Result<SkillInstallPreview, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        let loaded = load_install_source(
            request.source,
            workspace.clone(),
            |_| async { Ok::<(), ApplicationError>(()) },
            skill_error,
        )
        .await?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| manager.preview_install(&loaded.package, request.scope))
        .await?
        .map_err(skill_error)
    }

    pub async fn install_skills(
        &self,
        workspace_id: Option<i64>,
        request: SkillInstallRequest,
    ) -> Result<SkillInstallResult, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        let loaded = load_install_source(
            request.source,
            workspace.clone(),
            |_| async { Ok::<(), ApplicationError>(()) },
            skill_error,
        )
        .await?;
        loaded
            .verify_revision(request.expected_source_revision.as_deref())
            .map_err(skill_error)?;
        let manager = self.workspace_skill_manager(workspace).await;
        self.run_blocking(move |_| {
            manager.install(
                &loaded.package,
                request.scope,
                &request.selected,
                request.replace,
                Some(loaded.source),
            )
        })
        .await?
        .map_err(skill_error)
    }
}

fn require_revision(revision: &str) -> Result<(), ApplicationError> {
    if revision.is_empty() {
        return Err(ApplicationError::bad_request(
            "A current revision is required. Refresh the skill and retry.",
        ));
    }
    Ok(())
}

fn skill_error(error: SkillError) -> ApplicationError {
    match error.kind {
        SkillErrorKind::NotFound => ApplicationError::not_found_with_diagnostic(
            "The skill or resource was not found. Refresh the library.",
            &error,
        ),
        SkillErrorKind::Conflict => ApplicationError::conflict_with_diagnostic(
            "The skill or installation source changed. Refresh or preview it again before saving.",
            &error,
        ),
        SkillErrorKind::ReadOnly => ApplicationError::bad_request_with_diagnostic(
            "This skill is read-only. Install a copy into the workspace or global library to edit it.",
            &error,
        ),
        SkillErrorKind::Invalid | SkillErrorKind::TooLarge => {
            ApplicationError::bad_request_error(&error)
        }
        SkillErrorKind::Io => ApplicationError::internal_error(&error),
    }
}
