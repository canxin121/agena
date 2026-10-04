//! Pointer dispatch follows the same topmost surface and reducers as keyboard input.
use crate::*;
use agena_tui_components::pointer::PointerAction;
use crossterm::event::{KeyCode, KeyModifiers};

impl App {
    pub(crate) fn handle_surface_pointer(&mut self, mouse: MouseEvent) -> bool {
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && !self.btw_area.contains((mouse.column, mouse.row).into())
        {
            self.btw_focus = None;
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && !self.plan_area.contains((mouse.column, mouse.row).into())
        {
            self.plan_focus = None;
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && !self.work_area.contains((mouse.column, mouse.row).into())
        {
            self.work_focus = None;
        }
        if let Some(action) = self.pointer_targets.action(mouse) {
            self.cancel_active_pointer_gesture();
            self.cancel_surface_selection();
            self.apply_pointer_action(action, true);
            self.pointer_targets = Default::default();
            return true;
        }
        let modal = self.overlay.is_some()
            || self.context_help.is_some()
            || self.prompt_history_search.is_some()
            || self.slash_command_suggestions.is_some()
            || self.file_mention_suggestions.is_some();
        if self.current_route_is_main() && !modal {
            return false;
        }
        if matches!(self.current_route, Route::Hub(_)) && !modal {
            return false;
        }
        // Wheel events belong to the topmost route/dialog. Never scroll chat behind it.
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            let hit = self.pointer_targets.action(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                ..mouse
            });
            if let Some(
                action @ (PointerAction::List { .. }
                | PointerAction::FocusList(_)
                | PointerAction::PickerRow(_)
                | PointerAction::PickerResults
                | PointerAction::Named(
                    "plugin-section" | "plugin-node" | "plugin-drilldown",
                )
                | PointerAction::NamedIndex(
                    "plugin-section" | "plugin-node" | "plugin-drilldown",
                    _,
                )),
            ) = hit
            {
                self.apply_pointer_action(action, false);
            }
            let code = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                KeyCode::Up
            } else {
                KeyCode::Down
            };
            for _ in 0..3 {
                self.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
            }
        }
        true
    }

    fn apply_pointer_action(&mut self, action: PointerAction, activate: bool) {
        match action {
            PointerAction::Key(key) => self.handle_key_event(key),
            PointerAction::Named(name) => match name {
                "session-summary" => self.open_session_model_chooser(),
                "session-thinking" | "session-speed" => {
                    let step = if name == "session-thinking" {
                        SessionModelModeStep::ThinkingMode
                    } else {
                        SessionModelModeStep::SpeedMode
                    };
                    if let Err(error) = self.open_session_model_mode_overlay(step) {
                        self.flash_error(error.to_string());
                    }
                }
                "session-token-usage" => self.open_usage_dashboard(),
                name if name.starts_with("work-") => self.handle_session_work_action(name),
                "plan" => self.open_plan_viewer(),
                name if name.starts_with("plan-") => self.handle_inline_plan_action(name),
                "btw" => self.open_btw(""),
                name if name.starts_with("btw-") => self.handle_btw_action(name),
                "side" => self.handle_side_command(""),
                "side-parent" => self.open_parent_session(),
                "activities" => self.open_activities_panel(),
                "history" => self.open_prompt_history_search(),
                "approval" => self.maybe_auto_open_pending_interactive_overlay(),
                "transcript-search" => self.open_transcript_search_overlay(true),
                "permission-scroll-up" | "permission-scroll-down" => {
                    if let Some(Overlay::Permission(dialog)) = &self.overlay {
                        dialog
                            .presentation
                            .scroll_by(if name == "permission-scroll-up" {
                                -3
                            } else {
                                3
                            });
                    }
                }
                "plugin-section" | "plugin-node" | "plugin-drilldown" => {
                    if let Route::PluginWorkbench(dialog) = &mut self.current_route {
                        dialog.config_focus = if name == "plugin-section" {
                            agena_tui_plugin_workbench::PluginConfigFocus::Structure
                        } else {
                            agena_tui_plugin_workbench::PluginConfigFocus::Editor
                        };
                    }
                }
                _ => {}
            },
            PointerAction::List {
                panel,
                index,
                selected,
            } => {
                if activate
                    && matches!(self.current_route, Route::PermissionStudio(_))
                    && self.overlay.is_none()
                {
                    let Route::PermissionStudio(mut dialog) =
                        std::mem::replace(&mut self.current_route, Route::Main)
                    else {
                        unreachable!()
                    };
                    if dialog
                        .nav
                        .items
                        .get(index)
                        .is_some_and(|item| item.selectable)
                    {
                        dialog.nav.selected = index;
                        set_permission_studio_pane_focus(
                            &mut dialog,
                            PermissionStudioPaneFocus::Navigation,
                        );
                        self.apply_permission_studio_nav_selection(&mut dialog);
                    }
                    self.current_route = Route::PermissionStudio(dialog);
                    return;
                }
                let can_activate = self.focus_pointer_list(panel);
                if activate {
                    let key = if index < selected {
                        KeyCode::Up
                    } else {
                        KeyCode::Down
                    };
                    for _ in 0..index.abs_diff(selected) {
                        self.handle_key_event(KeyEvent::new(key, KeyModifiers::NONE));
                    }
                    if can_activate {
                        self.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            PointerAction::FocusList(panel) => {
                self.focus_pointer_list(panel);
            }
            PointerAction::NamedIndex("work-row", index) => self.handle_work_row(index),
            PointerAction::NamedIndex("plugin-tab", index) => {
                if let Route::PluginWorkbench(dialog) = &mut self.current_route
                    && let Some(tab) = agena_tui_plugin_workbench::PluginDetailTab::ALL.get(index)
                {
                    dialog.navigation.detail_tab = *tab;
                    dialog.config_scroll = 0;
                }
            }
            PointerAction::NamedIndex("plugin-tool", index) => {
                if let Route::PluginWorkbench(dialog) = &mut self.current_route {
                    dialog.selected_tool = index;
                    dialog.tool_result = None;
                    dialog.config_scroll = 0;
                    dialog.clamp_selection();
                    if activate {
                        self.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            PointerAction::NamedIndex(
                name @ ("plugin-section" | "plugin-node" | "plugin-drilldown"),
                index,
            ) => {
                if let Route::PluginWorkbench(dialog) = &mut self.current_route {
                    use agena_tui_plugin_workbench::{ConfigRowCell, PluginConfigFocus};
                    dialog.config_focus = if name == "plugin-section" {
                        PluginConfigFocus::Structure
                    } else {
                        PluginConfigFocus::Editor
                    };
                    if activate {
                        match name {
                            "plugin-section" => {
                                dialog.selected_section = index;
                                dialog.selected_node = 0;
                            }
                            "plugin-drilldown" => {
                                if let Some(drilldown) = dialog.current_drilldown_mut() {
                                    drilldown.selected_row = index;
                                    drilldown.selected_cell = ConfigRowCell::Value;
                                }
                            }
                            _ => {
                                dialog.selected_node = index;
                                dialog.selected_cell = ConfigRowCell::Value;
                            }
                        }
                        dialog.clamp_selection();
                    }
                }
                if activate && name != "plugin-section" {
                    self.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            action => {
                let next_key = if self.context_help.is_some() {
                    None
                } else if let Some(overlay) = self.overlay.as_mut() {
                    match overlay {
                        Overlay::Choice(dialog) => {
                            dialog.presentation.handle_pointer(&action, activate)
                        }
                        Overlay::PathBrowser(dialog) => {
                            dialog.presentation.handle_pointer(&action, activate)
                        }
                        Overlay::SessionSearch(dialog) => dialog.handle_pointer(&action, activate),
                        Overlay::Timeline(dialog) => dialog.handle_pointer(&action, activate),
                        _ => None,
                    }
                } else if let Some(history) = self.prompt_history_search.as_mut() {
                    history.handle_pointer(&action, activate)
                } else if let Some(picker) = self.file_mention_suggestions.as_mut() {
                    picker.handle_pointer(&action, activate)
                } else if let Some(picker) = self.slash_command_suggestions.as_mut() {
                    picker.handle_pointer(&action, activate)
                } else {
                    match &mut self.current_route {
                        Route::SessionSearch(dialog) => dialog.handle_pointer(&action, activate),
                        Route::CommandPalette(dialog) => {
                            dialog.presentation.handle_pointer(&action, activate)
                        }
                        Route::SessionNavigation(dialog) => {
                            dialog.presentation.handle_pointer(&action, activate)
                        }
                        Route::SelectionPicker(dialog) => {
                            dialog.presentation.handle_pointer(&action, activate)
                        }
                        Route::SessionModelChooser(dialog) => {
                            dialog.handle_pointer(&action, activate)
                        }
                        Route::Timeline(dialog) => dialog.handle_pointer(&action, activate),
                        Route::PluginWorkbench(dialog) => {
                            if let Some(picker) = dialog.actions.as_mut() {
                                picker.presentation.handle_pointer(&action, activate)
                            } else if let Some(picker) = dialog.selection.as_mut() {
                                picker.presentation.handle_pointer(&action, activate)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                };
                if let Some(key) = next_key {
                    self.handle_key_event(KeyEvent::new(key, KeyModifiers::NONE));
                }
            }
        }
    }

    fn focus_pointer_list(&mut self, panel: usize) -> bool {
        if self.context_help.is_some() {
            return false;
        }
        if let Some(overlay) = self.overlay.as_mut() {
            return match overlay {
                Overlay::ProviderStudio(dialog) => focus_provider_list(dialog, panel),
                _ => true,
            };
        }
        match &mut self.current_route {
            Route::SettingsStudio(dialog) | Route::ClientVersionsStudio(dialog) => {
                dialog.state.set_focus(if panel == 0 {
                    SettingsStudioFocus::Navigation
                } else {
                    SettingsStudioFocus::Items
                });
                panel != 0
            }
            Route::PermissionStudio(dialog) => {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Navigation);
                false
            }
            Route::ProviderStudio(dialog) => focus_provider_list(dialog, panel),
            _ => true,
        }
    }
}

fn focus_provider_list(dialog: &mut ProviderStudioOverlay, panel: usize) -> bool {
    if !dialog.show_provider_list
        && dialog.detail_page.is_none()
        && dialog.model_page.is_none()
        && dialog.editor.is_none()
    {
        dialog.selection.set_focus(match panel {
            0 => ProviderStudioFocus::Fields,
            1 => ProviderStudioFocus::Adapters,
            _ => ProviderStudioFocus::Models,
        });
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> App {
        App::new_with_backend(
            TuiBackend::remote_mock(),
            LaunchOptions::default(),
            I18n::english(),
        )
    }

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn find_target(app: &App, width: u16, height: u16, wanted: PointerAction) -> MouseEvent {
        for y in 0..height {
            for x in 0..width {
                let event = mouse(MouseEventKind::Down(MouseButton::Left), x, y);
                if app.pointer_targets.action(event) == Some(wanted.clone()) {
                    return event;
                }
            }
        }
        panic!("missing visible pointer target: {wanted:?}");
    }

    #[tokio::test]
    async fn topmost_help_owns_clicks_and_wheel_without_changing_plan_or_chat() {
        let mut app = app();
        let mut state = PlanViewerState::new(7);
        state.markdown = Some((0..100).map(|n| format!("- step {n}\n")).collect());
        app.current_route = Route::PlanViewer(state);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let refresh = find_target(
            &app,
            100,
            24,
            agena_tui_components::pointer::key(KeyCode::Char('r')),
        );
        assert!(refresh.column < 30);
        app.open_context_help();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        // The underlying plan's refresh target must be absent from this frame.
        for y in 0..24 {
            for x in 0..100 {
                assert_ne!(
                    app.pointer_targets.action(mouse(
                        MouseEventKind::Down(MouseButton::Left),
                        x,
                        y
                    )),
                    Some(agena_tui_components::pointer::key(KeyCode::Char('r')))
                );
            }
        }
        app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, 99, 23));
        let Route::PlanViewer(state) = &app.current_route else {
            panic!()
        };
        assert_eq!(state.presentation.scroll(), 0);
        assert_eq!(state.request_id, 0);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let close = find_target(
            &app,
            100,
            24,
            agena_tui_components::pointer::key(KeyCode::Esc),
        );
        app.handle_mouse_event(close);
        assert!(app.context_help.is_none());
        assert!(matches!(app.current_route, Route::PlanViewer(_)));
        // A second queued press cannot activate the old help target on the plan.
        assert_eq!(app.pointer_targets.action(close), None);
    }

    #[tokio::test]
    async fn plan_pointer_refresh_coalesces_and_wheel_scrolls_the_plan() {
        let mut app = app();
        app.transcript.session_id = Some(99);
        let mut state = PlanViewerState::new(7);
        state.markdown = Some((0..100).map(|n| format!("- step {n}\n")).collect());
        app.current_route = Route::PlanViewer(state);
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, 20, 10));
        let Route::PlanViewer(state) = &app.current_route else {
            panic!()
        };
        assert_eq!(state.presentation.scroll(), 3);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let refresh = find_target(
            &app,
            80,
            20,
            agena_tui_components::pointer::key(KeyCode::Char('r')),
        );
        app.handle_mouse_event(refresh);
        let request_id = app.next_usage_request_id;
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_mouse_event(refresh);
        assert_eq!(app.next_usage_request_id, request_id);
        let Route::PlanViewer(state) = &app.current_route else {
            panic!()
        };
        assert_eq!(
            state.session_id, 7,
            "plan actions retain the session that opened the viewer"
        );
        assert!(state.loading);
    }

    #[tokio::test]
    async fn composer_click_places_unicode_cursor_and_resize_discards_old_targets() {
        let mut app = app();
        app.composer.set_text("a你bc".into());
        app.focus = agena_tui::main_focus::Focus::Transcript;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let editor = app.surface_layout.composer_editor;
        app.handle_mouse_event(mouse(
            MouseEventKind::Down(MouseButton::Left),
            editor.x + 3,
            editor.y,
        ));
        app.handle_mouse_event(mouse(
            MouseEventKind::Up(MouseButton::Left),
            editor.x + 3,
            editor.y,
        ));
        assert_eq!(app.focus, agena_tui::main_focus::Focus::Composer);
        assert_eq!(app.composer.cursor(), "a你".len());
        assert!(app.surface_selection.is_none());
        app.current_route = Route::PlanViewer(PlanViewerState::new(7));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let close = find_target(
            &app,
            80,
            24,
            agena_tui_components::pointer::key(KeyCode::Esc),
        );
        app.handle_terminal_event(crossterm::event::Event::Resize(40, 15));
        assert_eq!(app.pointer_targets.action(close), None);
    }
}
