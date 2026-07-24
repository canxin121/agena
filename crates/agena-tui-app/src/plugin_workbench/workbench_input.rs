impl App {
    pub(crate) fn build_plugin_workbench(&self, query: &str) -> UiResult<PluginWorkbenchOverlay> {
        let sources = self
            .backend
            .config_json_sources()
            .map_err(|error| error.to_string())?;
        let locale = self.i18n.locale_tag();
        let statuses = self.backend.plugin_statuses();
        let mut plugins = statuses
            .into_iter()
            .map(|status| {
                let plugin_id = status.plugin_id.clone();
                let inspect = self.backend.plugin_inspect(&plugin_id.to_string());
                let logs = self.backend.plugin_logs(
                    &plugin_id.to_string(),
                    None,
                    PLUGIN_WORKBENCH_LOG_LIMIT,
                );
                build_plugin_workbench_plugin(&sources, locale.as_str(), status, inspect, logs)
            })
            .collect::<Vec<_>>();
        plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));

        Ok(PluginWorkbenchOverlay {
            title: "Plugins".to_owned(),
            list: PluginWorkbenchListPresentation::new(
                plugin_workbench_list_items(&plugins),
                query,
            ),
            navigation: PluginWorkbenchNavigation::new(),
            plugins,
            config_view: PluginConfigView::Effective,
            config_focus: PluginConfigFocus::Structure,
            selected_section: 0,
            selected_node: 0,
            selected_cell: ConfigRowCell::Value,
            selected_diagnostic: 0,
            selected_diff_row: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            drilldown_stack: Vec::new(),
            actions: None,
            selection: None,
            editor: None,
        })
    }

    pub(crate) fn refresh_plugin_workbench(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let query = dialog.list.query.text().to_owned();
        let selected_plugin_id = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone());
        let selected_section_key = dialog.selected_section().map(|section| section.key.clone());
        let selected_path = dialog.selected_row().map(|row| row.primary_path.clone());
        match self.build_plugin_workbench(query.as_str()) {
            Ok(mut refreshed) => {
                refreshed.navigation = dialog.navigation;
                refreshed.list.transport_filter = dialog.list.transport_filter;
                refreshed.list.config_filter = dialog.list.config_filter;
                refreshed
                    .list
                    .replace_items(plugin_workbench_list_items(&refreshed.plugins));
                refreshed.config_view = dialog.config_view;
                refreshed.config_focus = dialog.config_focus;
                refreshed.selected_cell = dialog.selected_cell;
                refreshed.show_diff = dialog.show_diff;
                refreshed.drilldown_stack =
                    rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
                if let Some(plugin_id) = selected_plugin_id {
                    refreshed.list.select_key(&plugin_id);
                }
                if let Some(section_key) = selected_section_key {
                    refreshed.selected_section = refreshed
                        .selected_plugin()
                        .and_then(|plugin| {
                            plugin
                                .sections
                                .iter()
                                .position(|section| section.key == section_key)
                        })
                        .unwrap_or_default();
                }
                if let Some(path) = selected_path
                    && let Some((section_index, row_index)) = refreshed
                        .selected_plugin()
                        .and_then(|plugin| find_row_position(plugin, refreshed.config_view, &path))
                {
                    refreshed.selected_section = section_index;
                    refreshed.selected_node = row_index;
                }
                refreshed.clamp_selection();
                *dialog = refreshed;
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn refresh_restored_plugin_workbench(
        &self,
        dialog: PluginWorkbenchOverlay,
    ) -> PluginWorkbenchOverlay {
        let query = dialog.list.query.text().to_owned();
        let selected_plugin_id = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone());
        let selected_section_key = dialog.selected_section().map(|section| section.key.clone());
        let selected_path = dialog.selected_row().map(|row| row.primary_path.clone());
        let Ok(mut refreshed) = self.build_plugin_workbench(query.as_str()) else {
            return dialog;
        };
        refreshed.navigation = dialog.navigation;
        refreshed.list.transport_filter = dialog.list.transport_filter;
        refreshed.list.config_filter = dialog.list.config_filter;
        refreshed
            .list
            .replace_items(plugin_workbench_list_items(&refreshed.plugins));
        refreshed.config_view = dialog.config_view;
        refreshed.config_focus = dialog.config_focus;
        refreshed.selected_cell = dialog.selected_cell;
        refreshed.show_diff = dialog.show_diff;
        refreshed.selected_diagnostic = dialog.selected_diagnostic;
        refreshed.selected_diff_row = dialog.selected_diff_row;
        refreshed.drilldown_stack =
            rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
        refreshed.actions = dialog.actions.clone();
        refreshed.selection = dialog.selection.clone();
        if let Some(plugin_id) = selected_plugin_id {
            refreshed.list.select_key(&plugin_id);
        }
        if let Some(section_key) = selected_section_key {
            refreshed.selected_section = refreshed
                .selected_plugin()
                .and_then(|plugin| {
                    plugin
                        .sections
                        .iter()
                        .position(|section| section.key == section_key)
                })
                .unwrap_or_default();
        }
        if let Some(path) = selected_path
            && let Some((section_index, row_index)) = refreshed
                .selected_plugin()
                .and_then(|plugin| find_row_position(plugin, refreshed.config_view, &path))
        {
            refreshed.selected_section = section_index;
            refreshed.selected_node = row_index;
        }
        refreshed.clamp_selection();
        refreshed
    }

    pub(crate) fn handle_plugin_workbench_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        if dialog.actions.is_some() {
            return self.handle_plugin_config_actions_key(key, dialog);
        }
        if dialog.selection.is_some() {
            return self.handle_plugin_config_selection_key(key, dialog);
        }
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => return false,
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                    return false;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) =
                        self.commit_plugin_config_editor(dialog, action, input.as_str())
                    {
                        self.flash_error(error);
                    } else {
                        dialog.editor = None;
                    }
                    return false;
                }
            }
        }
        if dialog.current_drilldown().is_some() {
            return self.handle_plugin_config_drilldown_key(key, dialog);
        }

        match dialog.navigation.mode {
            PluginWorkbenchMode::List => self.handle_plugin_workbench_list_key(key, dialog),
            PluginWorkbenchMode::Detail => self.handle_plugin_workbench_detail_key(key, dialog),
        }
    }

    pub(crate) fn handle_plugin_workbench_list_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        match handle_plugin_workbench_navigation_key(&mut dialog.navigation, key, false) {
            PluginWorkbenchNavigationEffect::Close => true,
            PluginWorkbenchNavigationEffect::OpenSelected => {
                dialog.selected_section = 0;
                dialog.selected_node = 0;
                false
            }
            PluginWorkbenchNavigationEffect::ScrollDetail(_) => false,
            PluginWorkbenchNavigationEffect::KeepOpen => {
                match handle_plugin_workbench_list_key(&mut dialog.list, key) {
                    PluginWorkbenchListEffect::Refresh => {
                        self.refresh_plugin_workbench(dialog);
                        false
                    }
                    PluginWorkbenchListEffect::KeepOpen => false,
                }
            }
        }
    }

    pub(crate) fn handle_plugin_workbench_detail_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        if dialog.navigation.detail_tab == PluginDetailTab::Config {
            return self.handle_plugin_config_key(key, dialog);
        }

        match handle_plugin_workbench_navigation_key(&mut dialog.navigation, key, false) {
            PluginWorkbenchNavigationEffect::ScrollDetail(delta) => {
                move_detail_scroll(dialog, delta);
                false
            }
            PluginWorkbenchNavigationEffect::Close
            | PluginWorkbenchNavigationEffect::OpenSelected
            | PluginWorkbenchNavigationEffect::KeepOpen => false,
        }
    }

    pub(crate) fn handle_plugin_config_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let compact_layout = dialog
            .selected_plugin()
            .is_some_and(plugin_uses_compact_config_layout);
        let action = resolve_tui_key(KeyContext::PluginConfig, key);
        if compact_layout && dialog.show_diff {
            match action {
                Some(KeyAction::Back | KeyAction::PluginDiff) => {
                    dialog.show_diff = false;
                    return false;
                }
                _ => return false,
            }
        }
        if compact_layout {
            match action {
                Some(KeyAction::PluginValidate) => {
                    self.run_compact_toolbar_action(dialog, CompactToolbarAction::Validate);
                    return false;
                }
                Some(KeyAction::PluginReset) => {
                    self.run_compact_toolbar_action(dialog, CompactToolbarAction::ResetAll);
                    return false;
                }
                Some(KeyAction::PluginDiff) => {
                    self.run_compact_toolbar_action(dialog, CompactToolbarAction::Diff);
                    return false;
                }
                Some(KeyAction::PluginSave) => {
                    self.run_compact_toolbar_action(dialog, CompactToolbarAction::Save);
                    return false;
                }
                Some(KeyAction::PluginRestart) => {
                    self.run_compact_toolbar_action(dialog, CompactToolbarAction::Restart);
                    return false;
                }
                Some(KeyAction::Edit) if dialog.config_focus == PluginConfigFocus::Structure => {
                    return false;
                }
                Some(KeyAction::MoveRight)
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    return false;
                }
                Some(KeyAction::MoveLeft)
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    return false;
                }
                Some(KeyAction::MoveLeft) if dialog.config_focus == PluginConfigFocus::Editor => {
                    self.move_selected_main_config_cell(dialog, -1);
                    return false;
                }
                Some(KeyAction::MoveRight) if dialog.config_focus == PluginConfigFocus::Editor => {
                    self.move_selected_main_config_cell(dialog, 1);
                    return false;
                }
                _ => {}
            }
        }
        match action {
            Some(KeyAction::Back) => {
                dialog.navigation.return_to_list();
                false
            }
            Some(KeyAction::NextTab) => {
                dialog.config_focus = next_config_focus(dialog.config_focus, compact_layout);
                false
            }
            Some(KeyAction::PreviousTab) => {
                dialog.config_focus = previous_config_focus(dialog.config_focus, compact_layout);
                false
            }
            Some(KeyAction::Delete) if dialog.config_focus == PluginConfigFocus::Editor => {
                self.delete_selected_config_node(dialog);
                false
            }
            Some(KeyAction::Edit) => {
                if dialog.config_focus == PluginConfigFocus::Diagnostics {
                    self.jump_to_selected_bottom_item(dialog);
                } else {
                    self.open_selected_config_value_editor(dialog);
                }
                false
            }
            // This layout has three or more focus areas. Horizontal arrows
            // stay local to a focused toolbar/editor; only Tab chords cross
            // focus-area boundaries.
            Some(KeyAction::MoveLeft) => false,
            Some(KeyAction::MoveRight) => false,
            Some(KeyAction::MoveUp) => {
                match dialog.config_focus {
                    PluginConfigFocus::Structure => move_selected_config_section(dialog, -1),
                    PluginConfigFocus::Diagnostics => move_selected_bottom_panel_row(dialog, -1),
                    _ => move_selected_config_node(dialog, -1),
                }
                false
            }
            Some(KeyAction::MoveDown) => {
                match dialog.config_focus {
                    PluginConfigFocus::Structure => move_selected_config_section(dialog, 1),
                    PluginConfigFocus::Diagnostics => move_selected_bottom_panel_row(dialog, 1),
                    _ => move_selected_config_node(dialog, 1),
                }
                false
            }
            _ => false,
        }
    }

    pub(crate) fn handle_plugin_config_actions_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let action = match resolve_tui_key(KeyContext::PluginConfigActions, key) {
            Some(KeyAction::Close) => agena_tui::plugin_workbench::PluginConfigPickerAction::Close,
            Some(KeyAction::MoveUp) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::MoveUp
            }
            Some(KeyAction::MoveDown) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::MoveDown
            }
            Some(KeyAction::PageUp) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::PageUp
            }
            Some(KeyAction::PageDown) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::PageDown
            }
            Some(KeyAction::Accept) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::Accept
            }
            _ => return false,
        };
        let effect = match dialog.actions.as_mut() {
            Some(overlay) => agena_tui::plugin_workbench::reduce_plugin_config_picker(
                &mut overlay.presentation,
                action,
            ),
            None => return false,
        };
        match effect {
            agena_tui::plugin_workbench::PluginConfigPickerEffect::Close => dialog.actions = None,
            agena_tui::plugin_workbench::PluginConfigPickerEffect::Activate { key } => {
                self.commit_plugin_config_action(dialog, key);
            }
            agena_tui::plugin_workbench::PluginConfigPickerEffect::KeepOpen => {}
        }
        false
    }

    pub(crate) fn handle_plugin_config_selection_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let action = match resolve_tui_key(KeyContext::PluginConfigSelection, key) {
            Some(KeyAction::Close) => agena_tui::plugin_workbench::PluginConfigPickerAction::Close,
            Some(KeyAction::MoveUp) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::MoveUp
            }
            Some(KeyAction::MoveDown) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::MoveDown
            }
            Some(KeyAction::PageUp) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::PageUp
            }
            Some(KeyAction::PageDown) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::PageDown
            }
            Some(KeyAction::Toggle) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::Toggle
            }
            Some(KeyAction::Accept) => {
                agena_tui::plugin_workbench::PluginConfigPickerAction::Accept
            }
            _ => return false,
        };
        let effect = match dialog.selection.as_mut() {
            Some(overlay) => agena_tui::plugin_workbench::reduce_plugin_config_picker(
                &mut overlay.presentation,
                action,
            ),
            None => return false,
        };
        match effect {
            agena_tui::plugin_workbench::PluginConfigPickerEffect::Close => dialog.selection = None,
            agena_tui::plugin_workbench::PluginConfigPickerEffect::Activate { key } => {
                if let Err(error) = self.commit_plugin_config_selection(dialog, key) {
                    self.flash_error(error);
                } else {
                    dialog.selection = None;
                }
            }
            agena_tui::plugin_workbench::PluginConfigPickerEffect::KeepOpen => {}
        }
        false
    }

    pub(crate) fn handle_plugin_config_drilldown_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let Some(overlay_snapshot) = dialog.current_drilldown().cloned() else {
            return false;
        };
        let view = dialog.config_view;
        match resolve_tui_key(KeyContext::PluginDrilldown, key) {
            Some(KeyAction::Back) => {
                dialog.drilldown_stack.pop();
                false
            }
            Some(KeyAction::MoveUp) => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index(&mut overlay.selected_row, count, -1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            Some(KeyAction::MoveDown) => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index(&mut overlay.selected_row, count, 1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            Some(KeyAction::MoveLeft) => {
                self.move_selected_drilldown_cell(dialog, -1);
                false
            }
            Some(KeyAction::MoveRight) => {
                self.move_selected_drilldown_cell(dialog, 1);
                false
            }
            Some(KeyAction::Edit) => {
                self.open_drilldown_selected_row_editor(dialog);
                false
            }
            Some(KeyAction::Delete) => {
                self.delete_drilldown_selected_row(dialog);
                false
            }
            _ => false,
        }
    }
}

use super::{
    App, CompactToolbarAction, ConfigRowCell, EditorDialogKeyResult, KeyEvent,
    PLUGIN_WORKBENCH_LOG_LIMIT, PluginConfigFocus, PluginConfigView, PluginDetailTab,
    PluginWorkbenchListPresentation, PluginWorkbenchMode, PluginWorkbenchNavigation,
    PluginWorkbenchOverlay, UiResult, build_plugin_workbench_plugin, drilldown_row_count,
    drilldown_selected_row_cell, drive_editor_dialog_key, find_row_position, move_detail_scroll,
    move_index, move_selected_bottom_panel_row, move_selected_config_node,
    move_selected_config_section, next_config_focus, plugin_uses_compact_config_layout,
    plugin_workbench_list_items, previous_config_focus, rebuild_drilldown_stack,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::plugin_workbench::{
    PluginWorkbenchListEffect, PluginWorkbenchNavigationEffect,
    handle_key as handle_plugin_workbench_navigation_key,
    handle_list_key as handle_plugin_workbench_list_key,
};
