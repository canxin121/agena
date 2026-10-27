use super::super::{
    App, ConfirmOverlay, Frame, Overlay, Rect, Route, SurfaceMode, render_confirm_dialog,
    render_overlay_line_input_dialog, sanitize_display_text,
};
use crate::ui_text;
use agena_tui_components::{EditorDialogSpec, render_editor_dialog};
use ratatui::widgets::{Paragraph, Wrap};

impl App {
    pub(crate) fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        let Some(overlay) = &self.overlay else {
            return;
        };

        match overlay {
            Overlay::HubProject(dialog) => {
                // Errors remain visible even when a long server path needs
                // more prompt rows than this terminal can accommodate.
                let prompt = match &dialog.error {
                    Some(error) => format!("{error}\n{}", dialog.editor.prompt),
                    None => dialog.editor.prompt.clone(),
                };
                let footer = self.i18n.text(if dialog.submitting {
                    "hub-project-submitting"
                } else {
                    "hub-project-footer"
                });
                let prompt = sanitize_display_text(&prompt);
                let prompt_height = Paragraph::new(prompt.as_str())
                    .wrap(Wrap { trim: false })
                    .line_count(SurfaceMode::Overlay.content_width(area, 88).max(1))
                    .clamp(1, area.height.saturating_sub(8).max(1) as usize)
                    as u16;
                render_editor_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    &EditorDialogSpec {
                        title: sanitize_display_text(&dialog.editor.title).into(),
                        prompt: prompt.into(),
                        footer: sanitize_display_text(&footer).into(),
                        target_width: 88,
                        multiline: false,
                        prompt_height_bounds: (prompt_height, prompt_height),
                        footer_height_bounds: (1, 2),
                    },
                    &dialog.editor.input,
                );
            }
            Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                render_overlay_line_input_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    sanitize_display_text(dialog.title.as_str()).into(),
                    sanitize_display_text(dialog.prompt.as_str()).into(),
                    sanitize_display_text(ui_text::t(&self.i18n, "overlay-line-footer")).into(),
                    &dialog.input,
                );
            }
            Overlay::TranscriptFoldLoad(dialog) => {
                render_overlay_line_input_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    sanitize_display_text(dialog.title.as_str()).into(),
                    sanitize_display_text(dialog.prompt.as_str()).into(),
                    sanitize_display_text(ui_text::t(&self.i18n, "transcript-load-count-footer"))
                        .into(),
                    &dialog.input,
                );
            }
            Overlay::SettingsValueEdit(dialog) => {
                let title = sanitize_display_text(dialog.title.as_str()).into();
                let prompt = sanitize_display_text(dialog.prompt.as_str()).into();
                let footer =
                    sanitize_display_text(ui_text::t(&self.i18n, "overlay-line-footer")).into();
                render_overlay_line_input_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    title,
                    prompt,
                    footer,
                    &dialog.input,
                );
            }

            Overlay::SessionPromptEdit(dialog) => {
                agena_tui_components::render_workbench_editor_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    sanitize_display_text(dialog.title.as_str()).into(),
                    sanitize_display_text(dialog.prompt.as_str()).into(),
                    sanitize_display_text(dialog.footer.as_str()).into(),
                    true,
                    &dialog.input,
                );
            }
            Overlay::QueueEditor(dialog) => self.render_queue_editor(frame, area, dialog),
            Overlay::Choice(dialog) => {
                agena_tui::choice::render_overlay(frame, area, &dialog.presentation, &self.i18n);
            }
            Overlay::PathBrowser(dialog) => {
                agena_tui::path_browser::render_overlay(
                    frame,
                    area,
                    &dialog.presentation,
                    &self.i18n,
                );
            }
            Overlay::Permission(dialog) => {
                agena_tui::permission_prompt::render_overlay(
                    frame,
                    area,
                    &dialog.presentation,
                    dialog.auto_approve.as_ref(),
                    &self.i18n,
                );
            }
            Overlay::Confirm(dialog) => {
                self.render_confirm_overlay(frame, area, dialog);
            }
            Overlay::SessionSearch(dialog) => {
                agena_tui_session::session_search::render_overlay(frame, area, dialog, &self.i18n);
            }
            Overlay::Timeline(dialog) => {
                agena_tui::timeline::render_overlay(frame, area, dialog, &self.i18n);
            }
            Overlay::ProviderStudio(dialog) => {
                self.render_provider_studio_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
            Overlay::ModelCatalogStudio(dialog) => {
                self.render_model_catalog_studio_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
        }
    }

    pub(crate) fn render_route(&mut self, frame: &mut Frame, area: Rect) {
        let route = self.current_route.clone();
        self.render_route_content(frame, area, &route);
        self.render_overlay(frame, area);
    }

    pub(crate) fn render_route_content(&mut self, frame: &mut Frame, area: Rect, route: &Route) {
        match route {
            Route::Main => {}
            Route::Usage(dialog) => {
                self.render_usage_dashboard(frame, area, dialog, SurfaceMode::Route)
            }
            Route::Activities(dialog) => self.render_activities_panel(frame, area, dialog),
            Route::PlanViewer(dialog) => self.render_plan_viewer(frame, area, dialog),
            Route::SessionWork(_) => self.render_session_work_panel(frame, area),
            Route::Btw(_) => self.render_btw_panel(frame, area),
            Route::SettingsStudio(dialog) => {
                self.render_settings_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::ClientVersionsStudio(dialog) => {
                self.render_settings_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::PermissionStudio(dialog) => {
                self.render_permission_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::PermissionRuleStudio(dialog) => {
                self.render_permission_rule_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::SessionSearch(dialog) => {
                agena_tui_session::session_search::render_overlay(frame, area, dialog, &self.i18n);
            }
            Route::Hub(state) => self.render_hub(frame, area, state, SurfaceMode::Route),
            Route::CommandPalette(dialog) => {
                agena_tui::command_palette::render_overlay(
                    frame,
                    area,
                    &dialog.presentation,
                    &self.i18n,
                );
            }
            Route::SessionNavigation(dialog) => {
                agena_tui_session::session_navigation::render_overlay(
                    frame,
                    area,
                    &dialog.presentation,
                    &self.i18n,
                );
            }
            Route::SelectionPicker(dialog) => {
                agena_tui::selection_picker::render_overlay(
                    frame,
                    area,
                    &dialog.presentation,
                    &self.i18n,
                );
            }
            Route::SessionModelChooser(dialog) => {
                agena_tui::model_chooser::render_overlay(frame, area, dialog, &self.i18n);
            }
            Route::Timeline(dialog) => {
                agena_tui::timeline::render_overlay(frame, area, dialog, &self.i18n);
            }
            Route::PluginWorkbench(dialog) => {
                self.render_plugin_workbench(frame, area, dialog, SurfaceMode::Route);
            }
            Route::SkillsWorkbench(dialog) => {
                self.render_skills_workbench(frame, area, dialog);
            }
            Route::ProviderStudio(dialog) => {
                self.render_provider_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::ModelCatalogStudio(dialog) => {
                self.render_model_catalog_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
        }
    }

    pub(crate) fn render_confirm_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ConfirmOverlay,
    ) {
        render_confirm_dialog(frame, area, dialog, |text| sanitize_display_text(text));
    }
}
