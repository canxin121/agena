//! Skill management shared by the Web and terminal clients. Model-driven tool
//! calls use the same manager through the permission-aware skills plugin.

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

    fn skill_manager(&self, workspace: std::path::PathBuf) -> SkillManager {
        let global_root = agena_storage_layout::StorageLayout::current()
            .root()
            .join("skills");
        let mut declared = Vec::new();
        for status in self.plugin_runtime().plugin_statuses() {
            let plugin_id = status.plugin_id.to_string();
            if let Some(manifest) = self
                .plugin_runtime()
                .plugin_inspect(&plugin_id)
                .filter(|plugin| plugin.authority.is_some())
                .and_then(|plugin| plugin.manifest)
            {
                for skill in manifest.skills {
                    declared.push(DeclaredSkill {
                        plugin_id: plugin_id.clone(),
                        version: manifest.version.clone(),
                        name: skill.name,
                        description: skill.description,
                        instructions: skill.instructions,
                        aliases: skill.aliases,
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
            .with_declared_skills(declared)
            .with_disabled_names(disabled)
    }

    pub async fn list_skills(
        &self,
        workspace_id: Option<i64>,
    ) -> Result<SkillCatalog, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        self.run_blocking(move |app| app.skill_manager(workspace).list())
            .await?
            .map_err(skill_error)
    }

    pub async fn get_skill(
        &self,
        workspace_id: Option<i64>,
        id: String,
    ) -> Result<SkillDetail, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        self.run_blocking(move |app| app.skill_manager(workspace).get(&id))
            .await?
            .map_err(skill_error)
    }

    pub async fn create_skill(
        &self,
        workspace_id: Option<i64>,
        request: SkillCreateRequest,
    ) -> Result<SkillDetail, ApplicationError> {
        let workspace = self.skills_workspace_root(workspace_id).await?;
        self.run_blocking(move |app| {
            app.skill_manager(workspace)
                .create(request.scope, &request.document)
        })
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
        self.run_blocking(move |app| {
            app.skill_manager(workspace).update(
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
        self.run_blocking(move |app| {
            app.skill_manager(workspace)
                .remove(&id, Some(&expected_revision))
        })
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
        self.run_blocking(move |app| {
            app.skill_manager(workspace).set_enabled(
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
        self.run_blocking(move |app| app.skill_manager(workspace).read_resource(&id, &path))
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
        self.run_blocking(move |app| {
            app.skill_manager(workspace).write_resource(
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
        self.run_blocking(move |app| {
            app.skill_manager(workspace)
                .remove_resource(&id, &path, Some(&expected_revision))
        })
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
        self.run_blocking(move |app| {
            app.skill_manager(workspace)
                .preview_install(&loaded.package, request.scope)
        })
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
        self.run_blocking(move |app| {
            app.skill_manager(workspace).install(
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
