use super::super::{
    App, ConfirmOverlay, Frame, Overlay, Rect, Route, SurfaceMode, render_confirm_dialog,
    render_overlay_line_input_dialog, sanitize_display_text,
};
use crate::ui_text;

impl App {
    pub(crate) fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        let Some(overlay) = &self.overlay else {
            return;
        };

        match overlay {
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
            Overlay::SettingsValueEdit(dialog) => {
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

    pub(crate) fn render_route(&self, frame: &mut Frame, area: Rect) {
        self.render_route_content(frame, area, &self.current_route);
        self.render_overlay(frame, area);
    }

    pub(crate) fn render_route_content(&self, frame: &mut Frame, area: Rect, route: &Route) {
        match route {
            Route::Main => {}
            Route::Usage(dialog) => {
                self.render_usage_dashboard(frame, area, dialog, SurfaceMode::Route)
            }
            Route::Activities(dialog) => self.render_activities_panel(frame, area, dialog),
            Route::PlanViewer(dialog) => self.render_plan_viewer(frame, area, dialog),
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
            Route::CommandPalette(dialog) => {
                agena_tui::command_palette::render_overlay(
                    frame,
                    area,
                    &dialog.presentation,
                    &self.i18n,
                );
            }
            Route::SkillPicker(dialog) => {
                agena_tui::selection_picker::render_overlay(
                    frame,
                    area,
                    &dialog.presentation,
                    &self.i18n,
                );
            }
            Route::SkillStudio(dialog) => self.render_skill_studio(frame, area, dialog),
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
