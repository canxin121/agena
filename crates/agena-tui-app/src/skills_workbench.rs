//! Skill-library workbench: shared API effects and the existing TUI surfaces.

mod input;
mod installation;
mod render;
mod state;

pub(crate) use state::SkillsWorkbench;

use agena_api::skills::{SkillDetail, SkillResource};

use crate::{App, Route};
use state::SkillPane;

impl App {
    pub(crate) fn open_skills_workbench(&mut self) {
        let mut dialog = SkillsWorkbench::new(self.application.workspace_id());
        self.request_skills_refresh(&mut dialog);
        self.current_route = Route::SkillsWorkbench(Box::new(dialog));
    }

    fn with_skills_workbench(
        &mut self,
        instance: u64,
        update: impl FnOnce(&mut Self, &mut SkillsWorkbench),
    ) {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        match route {
            Route::SkillsWorkbench(mut dialog) => {
                if dialog.instance == instance {
                    if dialog.workspace_id == self.application.workspace_id() {
                        update(self, &mut dialog);
                    } else {
                        dialog.busy = false;
                        dialog.loading = false;
                        dialog.detail_loading = false;
                        dialog.resource_loading = false;
                        dialog.error = Some("The active workspace changed. Close and reopen Skills for the current workspace.".to_owned());
                    }
                }
                self.current_route = Route::SkillsWorkbench(dialog);
            }
            other => self.current_route = other,
        }
    }

    fn request_skills_refresh(&mut self, dialog: &mut SkillsWorkbench) {
        let instance = dialog.instance;
        let workspace_id = dialog.workspace_id;
        let request = dialog.begin_catalog_request();
        let selected = dialog.selected_skill().map(|skill| skill.id.clone());
        self.dispatch_backend_operation(
            move |application| async move {
                Ok::<_, anyhow::Error>(application.client().list_skills(workspace_id).await?)
            },
            move |app, result| {
                app.with_skills_workbench(instance, |app, dialog| {
                    if dialog.catalog_request != request {
                        return;
                    }
                    dialog.loading = false;
                    match result {
                        Ok(mut catalog) => {
                            catalog.skills.sort_by(|left, right| {
                                left.name.cmp(&right.name).then(left.id.cmp(&right.id))
                            });
                            dialog.catalog = catalog;
                            dialog.reselect(selected.as_deref());
                            app.request_selected_skill(dialog);
                        }
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                });
            },
        );
    }

    fn request_selected_skill(&mut self, dialog: &mut SkillsWorkbench) {
        dialog.detail = None;
        dialog.resource = None;
        dialog.file_index = 0;
        dialog.scroll = 0;
        dialog.detail_request = dialog.detail_request.wrapping_add(1);
        dialog.resource_request = dialog.resource_request.wrapping_add(1);
        let Some(id) = dialog.selected_skill().map(|skill| skill.id.clone()) else {
            dialog.detail_loading = false;
            return;
        };
        let instance = dialog.instance;
        let workspace_id = dialog.workspace_id;
        let request = dialog.detail_request;
        dialog.detail_loading = true;
        dialog.error = None;
        let requested_id = id.clone();
        self.dispatch_backend_operation(
            move |application| async move {
                Ok::<_, anyhow::Error>(application.client().get_skill(workspace_id, &id).await?)
            },
            move |app, result| {
                app.with_skills_workbench(instance, |_, dialog| {
                    if dialog.detail_request != request
                        || dialog.selected_skill().map(|skill| skill.id.as_str())
                            != Some(requested_id.as_str())
                    {
                        return;
                    }
                    dialog.detail_loading = false;
                    match result {
                        Ok(detail) => dialog.install_detail(detail),
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                });
            },
        );
    }

    fn request_selected_skill_resource(&mut self, dialog: &mut SkillsWorkbench) {
        let Some(detail) = dialog.detail.as_ref() else {
            return;
        };
        let Some(path) = dialog.selected_file().map(str::to_owned) else {
            return;
        };
        dialog.scroll = 0;
        dialog.resource_request = dialog.resource_request.wrapping_add(1);
        dialog.error = None;
        if path == "SKILL.md" {
            dialog.resource = Some(SkillResource {
                path,
                content: detail.document.clone(),
                revision: detail.summary.revision.clone(),
            });
            dialog.resource_loading = false;
            return;
        }
        let id = detail.summary.id.clone();
        let requested_id = id.clone();
        let requested_path = path.clone();
        let instance = dialog.instance;
        let workspace_id = dialog.workspace_id;
        let request = dialog.resource_request;
        dialog.resource = None;
        dialog.resource_loading = true;
        self.dispatch_backend_operation(
            move |application| async move {
                Ok::<_, anyhow::Error>(
                    application
                        .client()
                        .get_skill_resource(workspace_id, &id, &path)
                        .await?,
                )
            },
            move |app, result| {
                app.with_skills_workbench(instance, |_, dialog| {
                    if dialog.resource_request != request
                        || dialog
                            .detail
                            .as_ref()
                            .map(|detail| detail.summary.id.as_str())
                            != Some(requested_id.as_str())
                        || dialog.selected_file() != Some(requested_path.as_str())
                    {
                        return;
                    }
                    dialog.resource_loading = false;
                    match result {
                        Ok(resource) => dialog.resource = Some(resource),
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                });
            },
        );
    }

    fn finish_skill_mutation(
        &mut self,
        dialog: &mut SkillsWorkbench,
        result: crate::UiResult<SkillMutationResult>,
    ) {
        dialog.busy = false;
        match result {
            Ok(result) => {
                dialog.editor = None;
                dialog.confirm = None;
                dialog.installation = None;
                dialog.error = None;
                self.flash_success(result.message);
                if let Some(detail) = result.detail {
                    let id = detail.summary.id.clone();
                    dialog.query.clear();
                    dialog.scope_filter = None;
                    dialog.upsert_summary(detail.summary.clone());
                    dialog.reselect(Some(&id));
                    dialog.install_detail(detail);
                }
                self.request_skills_refresh(dialog);
            }
            Err(error) => dialog.error = Some(error.to_string()),
        }
    }
}

struct SkillMutationResult {
    message: String,
    detail: Option<SkillDetail>,
}

impl SkillMutationResult {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            detail: None,
        }
    }

    fn detail(detail: SkillDetail, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            detail: Some(detail),
        }
    }
}
