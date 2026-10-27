//! Pointer dispatch follows the same topmost surface and reducers as keyboard input.
use crate::*;
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui_components::pointer::PointerAction;
use crossterm::event::{KeyCode, KeyModifiers};

impl App {
    /// Marker toggles change neither route nor row geometry until the next
    /// frame. Keep the hit map for the still-visible toolbar so a second
    /// click in the same input batch is not silently discarded.
    pub(crate) fn is_session_mark_key(&self, key: KeyEvent) -> bool {
        if self.context_help.is_some() {
            return false;
        }
        let search = match &self.overlay {
            Some(Overlay::SessionSearch(dialog)) => Some(dialog),
            Some(_) => return false,
            None => match &self.current_route {
                Route::SessionSearch(dialog) => Some(dialog),
                _ => None,
            },
        };
        let context = match search {
            Some(dialog)
                if dialog.focus == agena_tui_components::SearchPickerFocus::Results
                    || key.modifiers.contains(KeyModifiers::ALT) =>
            {
                KeyContext::SessionSearch
            }
            Some(_) => return false,
            None => match &self.current_route {
                Route::Hub(state)
                    if !state.search_active || key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    KeyContext::Hub
                }
                _ => return false,
            },
        };
        matches!(
            resolve_tui_key(context, key),
            Some(KeyAction::HubTogglePinned | KeyAction::ToggleFavorite)
        )
    }

    pub(crate) fn handle_surface_pointer(&mut self, mouse: MouseEvent) -> bool {
        if let Some(action) = self.pointer_targets.action(mouse) {
            let keep_targets =
                matches!(&action, PointerAction::Key(key) if self.is_session_mark_key(*key));
            self.cancel_active_pointer_gesture();
            self.cancel_surface_selection();
            self.apply_pointer_action(action, true);
            if !keep_targets {
                self.pointer_targets = Default::default();
            }
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
                "plan" => self.open_plan_viewer_fullscreen(),
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
                if self.overlay.is_none()
                    && self.context_help.is_none()
                    && let Route::SettingsStudio(dialog) | Route::ClientVersionsStudio(dialog) =
                        &mut self.current_route
                {
                    if activate {
                        if dialog.state.select_visible(panel, index) {
                            self.handle_key_event(KeyEvent::new(
                                KeyCode::Enter,
                                KeyModifiers::NONE,
                            ));
                        }
                    } else {
                        dialog.state.set_focus(if panel == 0 {
                            SettingsStudioFocus::Navigation
                        } else {
                            SettingsStudioFocus::Items
                        });
                    }
                    return;
                }
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
            Route::SkillsWorkbench(dialog) => {
                dialog.focus_list(panel);
                true
            }
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
