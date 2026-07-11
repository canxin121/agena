impl App {
    pub(in crate::app) fn activate_agent_studio_selection(
        &mut self,
        dialog: &mut AgentStudioOverlay,
    ) -> bool {
        let Some(item) = dialog.workbench.list.selected_item().cloned() else {
            return false;
        };
        match item.action {
            AgentStudioAction::Edit(field) => {
                if !dialog.editable {
                    self.flash_warning(agent_read_only_edit_message(&self.i18n));
                    return false;
                }
                self.open_agent_studio_editor(dialog, field);
            }
            AgentStudioAction::OpenPermissionWorkbench => {
                self.route_stack.push(Route::AgentStudio(dialog.clone()));
                self.open_agent_permission_studio(dialog.agent_name.as_str());
            }
            AgentStudioAction::OpenSource => self.open_agent_profile_source(&dialog.profile),
        }
        false
    }

    pub(in crate::app) fn open_agent_studio_editor(
        &mut self,
        dialog: &mut AgentStudioOverlay,
        field: AgentStudioField,
    ) {
        let (title, prompt, footer, multiline, input) =
            agent_studio_editor_config(&self.i18n, &dialog.profile, field);
        dialog.workbench.editor = Some(AgentStudioEditor::new(
            title,
            prompt,
            footer,
            multiline,
            input,
            AgentStudioEditorAction::Field(field),
        ));
    }

    pub(in crate::app) fn commit_agent_studio_editor(
        &mut self,
        dialog: &mut AgentStudioOverlay,
        action: AgentStudioEditorAction,
        input: String,
    ) -> UiResult<()> {
        match action {
            AgentStudioEditorAction::Field(field) => {
                match dialog.storage {
                    AgentProfileStorage::Config => {
                        let (path, value) = agent_studio_field_setting_value(
                            &self.i18n,
                            dialog.agent_name.as_str(),
                            field,
                            input.as_str(),
                        )?;
                        if let Some(value) = value {
                            self.block_on_async(
                                self.backend.set_config_setting(path.as_str(), value),
                            )
                            .map_err(|error| error.to_string())?;
                            self.flash_success(settings_path_updated_message(
                                &self.i18n,
                                path.as_str(),
                            ));
                        } else {
                            self.block_on_async(self.backend.delete_config_setting(path.as_str()))
                                .map_err(|error| error.to_string())?;
                            self.flash_success(settings_path_cleared_message(
                                &self.i18n,
                                path.as_str(),
                            ));
                        }
                    }
                    AgentProfileStorage::Markdown => {
                        let mut profile = dialog.profile.clone();
                        apply_agent_studio_field_to_profile(&mut profile, field, input.as_str());
                        self.persist_agent_markdown_profile(&profile)?;
                    }
                    AgentProfileStorage::BuiltIn | AgentProfileStorage::Runtime => {
                        return Err(agent_read_only_edit_message(&self.i18n));
                    }
                }
                self.refresh_agent_studio_overlay(dialog);
            }
        }
        Ok(())
    }

    pub(in crate::app) fn persist_agent_markdown_profile(
        &mut self,
        profile: &AgentProfile,
    ) -> UiResult<()> {
        let path = profile
            .source_path
            .as_ref()
            .ok_or_else(|| agent_read_only_edit_message(&self.i18n))?;
        let text = agent_markdown_document(&profile.frontmatter, profile.prompt.as_str())?;
        fs::write(path, text).map_err(|error| {
            self.i18n.text_args(
                "flash-agent-source-write-failed",
                &crate::fl_args!(
                    "path" => path.display().to_string(),
                    "error" => error.to_string(),
                ),
            )
        })?;
        self.block_on_async(self.backend.reload_runtime())
            .map_err(|error| error.to_string())?;
        self.flash_success(self.i18n.text_args(
            "flash-agent-source-updated",
            &crate::fl_args!("path" => path.display().to_string()),
        ));
        Ok(())
    }
}
use super::{
    AgentProfile, AgentProfileStorage, AgentStudioAction, AgentStudioEditor,
    AgentStudioEditorAction, AgentStudioField, AgentStudioOverlay, App, Route, UiResult,
};
use crate::app::{
    agent_markdown_document, agent_studio_editor_config, agent_studio_field_setting_value,
    apply_agent_studio_field_to_profile,
};
use crate::app::{
    agent_read_only_edit_message, settings_path_cleared_message, settings_path_updated_message,
};
use std::fs;
