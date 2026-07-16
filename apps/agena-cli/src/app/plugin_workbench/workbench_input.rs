impl App {
    pub(in crate::app) fn build_plugin_workbench(
        &self,
        query: &str,
    ) -> UiResult<PluginWorkbenchOverlay> {
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

        let visible_plugins = filtered_plugin_indices(
            plugins.as_slice(),
            query,
            PluginTransportFilter::All,
            PluginConfigFilter::All,
        );
        Ok(PluginWorkbenchOverlay {
            title: "Plugins".to_owned(),
            query: Editor::from_text(query.to_owned()),
            mode: PluginWorkbenchMode::List,
            transport_filter: PluginTransportFilter::All,
            config_filter: PluginConfigFilter::All,
            plugins,
            visible_plugins,
            selected_plugin: 0,
            detail_tab: PluginDetailTab::Config,
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

    pub(in crate::app) fn refresh_plugin_workbench(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let query = dialog.query.text().to_owned();
        let selected_plugin_id = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone());
        let selected_section_key = dialog.selected_section().map(|section| section.key.clone());
        let selected_path = dialog.selected_row().map(|row| row.primary_path.clone());
        match self.build_plugin_workbench(query.as_str()) {
            Ok(mut refreshed) => {
                refreshed.mode = dialog.mode;
                refreshed.transport_filter = dialog.transport_filter;
                refreshed.config_filter = dialog.config_filter;
                refreshed.detail_tab = dialog.detail_tab;
                refreshed.config_view = dialog.config_view;
                refreshed.config_focus = dialog.config_focus;
                refreshed.selected_cell = dialog.selected_cell;
                refreshed.show_diff = dialog.show_diff;
                refreshed.drilldown_stack =
                    rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
                refresh_plugin_workbench_filter(&mut refreshed);
                if let Some(plugin_id) = selected_plugin_id
                    && let Some(index) = refreshed.visible_plugins.iter().position(|visible| {
                        refreshed
                            .plugins
                            .get(*visible)
                            .is_some_and(|plugin| plugin.plugin_id == plugin_id)
                    })
                {
                    refreshed.selected_plugin = index;
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

    pub(in crate::app) fn refresh_restored_plugin_workbench(
        &self,
        dialog: PluginWorkbenchOverlay,
    ) -> PluginWorkbenchOverlay {
        let query = dialog.query.text().to_owned();
        let selected_plugin_id = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone());
        let selected_section_key = dialog.selected_section().map(|section| section.key.clone());
        let selected_path = dialog.selected_row().map(|row| row.primary_path.clone());
        let Ok(mut refreshed) = self.build_plugin_workbench(query.as_str()) else {
            return dialog;
        };
        refreshed.mode = dialog.mode;
        refreshed.transport_filter = dialog.transport_filter;
        refreshed.config_filter = dialog.config_filter;
        refreshed.detail_tab = dialog.detail_tab;
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
        refresh_plugin_workbench_filter(&mut refreshed);
        if let Some(plugin_id) = selected_plugin_id
            && let Some(index) = refreshed.visible_plugins.iter().position(|visible| {
                refreshed
                    .plugins
                    .get(*visible)
                    .is_some_and(|plugin| plugin.plugin_id == plugin_id)
            })
        {
            refreshed.selected_plugin = index;
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

    pub(in crate::app) fn handle_plugin_workbench_key(
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

        match dialog.mode {
            PluginWorkbenchMode::List => self.handle_plugin_workbench_list_key(key, dialog),
            PluginWorkbenchMode::Detail => self.handle_plugin_workbench_detail_key(key, dialog),
        }
    }

    pub(in crate::app) fn handle_plugin_workbench_list_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        match resolve_tui_key(KeyContext::PluginList, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::PluginCycleTransport) => {
                dialog.transport_filter = next_transport_filter(dialog.transport_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            Some(KeyAction::PluginCycleConfig) => {
                dialog.config_filter = next_config_filter(dialog.config_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            Some(KeyAction::Refresh) => {
                self.refresh_plugin_workbench(dialog);
                false
            }
            Some(KeyAction::Open) => {
                dialog.mode = PluginWorkbenchMode::Detail;
                dialog.detail_tab = PluginDetailTab::Config;
                dialog.selected_section = 0;
                dialog.selected_node = 0;
                false
            }
            Some(KeyAction::MoveUp) => {
                move_index(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    -1,
                );
                false
            }
            Some(KeyAction::MoveDown) => {
                move_index(&mut dialog.selected_plugin, dialog.visible_plugins.len(), 1);
                false
            }
            _ => {
                let before = dialog.query.text().to_owned();
                dialog.query.handle_line_input_key(key);
                if dialog.query.text() != before {
                    refresh_plugin_workbench_filter(dialog);
                }
                false
            }
        }
    }

    pub(in crate::app) fn handle_plugin_workbench_detail_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        if dialog.detail_tab == PluginDetailTab::Config {
            return self.handle_plugin_config_key(key, dialog);
        }

        match resolve_tui_key(KeyContext::PluginDetail, key) {
            Some(KeyAction::Back) => {
                dialog.mode = PluginWorkbenchMode::List;
                false
            }
            Some(KeyAction::NextTab) => {
                dialog.detail_tab = dialog.detail_tab.move_by(1);
                false
            }
            Some(KeyAction::PreviousTab) => {
                dialog.detail_tab = dialog.detail_tab.move_by(-1);
                false
            }
            Some(KeyAction::MoveUp) if dialog.detail_tab != PluginDetailTab::Config => {
                move_detail_scroll(dialog, -1);
                false
            }
            Some(KeyAction::MoveDown) if dialog.detail_tab != PluginDetailTab::Config => {
                move_detail_scroll(dialog, 1);
                false
            }
            _ => false,
        }
    }

    pub(in crate::app) fn handle_plugin_config_key(
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
                dialog.mode = PluginWorkbenchMode::List;
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

    pub(in crate::app) fn handle_plugin_config_actions_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let Some(overlay) = dialog.actions.as_mut() else {
            return false;
        };
        match resolve_tui_key(KeyContext::PluginConfigActions, key) {
            Some(KeyAction::Close) => {
                dialog.actions = None;
                false
            }
            Some(KeyAction::MoveUp) => {
                overlay.move_selection(-1);
                false
            }
            Some(KeyAction::MoveDown) => {
                overlay.move_selection(1);
                false
            }
            Some(KeyAction::PageUp) => {
                overlay.move_selection_page(-1);
                false
            }
            Some(KeyAction::PageDown) => {
                overlay.move_selection_page(1);
                false
            }
            Some(KeyAction::Accept) => {
                self.commit_plugin_config_action(dialog);
                false
            }
            _ => false,
        }
    }

    pub(in crate::app) fn handle_plugin_config_selection_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let Some(overlay) = dialog.selection.as_mut() else {
            return false;
        };
        match resolve_tui_key(KeyContext::PluginConfigSelection, key) {
            Some(KeyAction::Close) => {
                dialog.selection = None;
                false
            }
            Some(KeyAction::MoveUp) => {
                overlay.move_selection(-1);
                false
            }
            Some(KeyAction::MoveDown) => {
                overlay.move_selection(1);
                false
            }
            Some(KeyAction::PageUp) => {
                overlay.move_selection_page(-1);
                false
            }
            Some(KeyAction::PageDown) => {
                overlay.move_selection_page(1);
                false
            }
            Some(KeyAction::Toggle) if overlay.meta.multi => {
                if let Some(item) = overlay.selected_item_mut() {
                    item.checked = !item.checked;
                }
                overlay.toggle_selected();
                false
            }
            Some(KeyAction::Accept) => {
                if let Err(error) = self.commit_plugin_config_selection(dialog) {
                    self.flash_error(error);
                } else {
                    dialog.selection = None;
                }
                false
            }
            _ => false,
        }
    }

    pub(in crate::app) fn handle_plugin_config_drilldown_key(
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
    App, CompactToolbarAction, ConfigRowCell, Editor, EditorDialogKeyResult, KeyEvent,
    PLUGIN_WORKBENCH_LOG_LIMIT, PluginConfigFilter, PluginConfigFocus, PluginConfigView,
    PluginDetailTab, PluginTransportFilter, PluginWorkbenchMode, PluginWorkbenchOverlay, UiResult,
    build_plugin_workbench_plugin, drilldown_row_count, drilldown_selected_row_cell,
    drive_editor_dialog_key, filtered_plugin_indices, find_row_position, move_detail_scroll,
    move_index, move_selected_bottom_panel_row, move_selected_config_node,
    move_selected_config_section, next_config_filter, next_config_focus, next_transport_filter,
    plugin_uses_compact_config_layout, previous_config_focus, rebuild_drilldown_stack,
    refresh_plugin_workbench_filter,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
