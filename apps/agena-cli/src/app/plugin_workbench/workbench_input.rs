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
            selected_toolbar_action: 0,
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
                refreshed.selected_toolbar_action = dialog.selected_toolbar_action;
                refreshed.selected_cell = dialog.selected_cell;
                refreshed.show_diff = dialog.show_diff;
                refreshed.drilldown_stack =
                    rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
                refresh_plugin_workbench_filter(&mut refreshed);
                if let Some(plugin_id) = selected_plugin_id {
                    if let Some(index) = refreshed.visible_plugins.iter().position(|visible| {
                        refreshed
                            .plugins
                            .get(*visible)
                            .is_some_and(|plugin| plugin.plugin_id == plugin_id)
                    }) {
                        refreshed.selected_plugin = index;
                    }
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
                if let Some(path) = selected_path {
                    if let Some((section_index, row_index)) = refreshed
                        .selected_plugin()
                        .and_then(|plugin| find_row_position(plugin, refreshed.config_view, &path))
                    {
                        refreshed.selected_section = section_index;
                        refreshed.selected_node = row_index;
                    }
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
        refreshed.selected_toolbar_action = dialog.selected_toolbar_action;
        refreshed.selected_cell = dialog.selected_cell;
        refreshed.show_diff = dialog.show_diff;
        refreshed.selected_diagnostic = dialog.selected_diagnostic;
        refreshed.selected_diff_row = dialog.selected_diff_row;
        refreshed.drilldown_stack =
            rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
        refreshed.actions = dialog.actions.clone();
        refreshed.selection = dialog.selection.clone();
        refresh_plugin_workbench_filter(&mut refreshed);
        if let Some(plugin_id) = selected_plugin_id {
            if let Some(index) = refreshed.visible_plugins.iter().position(|visible| {
                refreshed
                    .plugins
                    .get(*visible)
                    .is_some_and(|plugin| plugin.plugin_id == plugin_id)
            }) {
                refreshed.selected_plugin = index;
            }
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
        if let Some(path) = selected_path {
            if let Some((section_index, row_index)) = refreshed
                .selected_plugin()
                .and_then(|plugin| find_row_position(plugin, refreshed.config_view, &path))
            {
                refreshed.selected_section = section_index;
                refreshed.selected_node = row_index;
            }
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
            Some(KeyAction::Open) => {
                dialog.mode = PluginWorkbenchMode::Detail;
                dialog.detail_tab = PluginDetailTab::Config;
                dialog.selected_section = 0;
                dialog.selected_node = 0;
                false
            }
            Some(KeyAction::Refresh) => {
                self.refresh_plugin_workbench(dialog);
                false
            }
            Some(KeyAction::TransportFilter) => {
                dialog.transport_filter = next_transport_filter(dialog.transport_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            Some(KeyAction::ConfigFilter) => {
                dialog.config_filter = next_config_filter(dialog.config_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            Some(KeyAction::PageUp) => {
                move_index_page(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    -1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                false
            }
            Some(KeyAction::PageDown) => {
                move_index_page(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                false
            }
            Some(KeyAction::Home) => {
                dialog.selected_plugin = 0;
                false
            }
            Some(KeyAction::End) => {
                dialog.selected_plugin = dialog.visible_plugins.len().saturating_sub(1);
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
                dialog.query.flush_all_pending_input();
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
            return match resolve_tui_key(KeyContext::PluginDetail, key) {
                Some(KeyAction::Back) => {
                    dialog.mode = PluginWorkbenchMode::List;
                    false
                }
                Some(KeyAction::PreviousTab) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    dialog.detail_tab = dialog.detail_tab.move_by(-1);
                    false
                }
                Some(KeyAction::NextTab) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    dialog.detail_tab = dialog.detail_tab.move_by(1);
                    false
                }
                _ => self.handle_plugin_config_key(key, dialog),
            };
        }

        match resolve_tui_key(KeyContext::PluginDetail, key) {
            Some(KeyAction::Back) => {
                dialog.mode = PluginWorkbenchMode::List;
                false
            }
            Some(KeyAction::Refresh) => {
                self.refresh_plugin_workbench(dialog);
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
            Some(KeyAction::PageUp) if dialog.detail_tab != PluginDetailTab::Config => {
                move_detail_scroll(dialog, -10);
                false
            }
            Some(KeyAction::PageDown) if dialog.detail_tab != PluginDetailTab::Config => {
                move_detail_scroll(dialog, 10);
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
                Some(KeyAction::Close | KeyAction::CloseDiff | KeyAction::ShowDiff) => {
                    dialog.show_diff = false;
                    return false;
                }
                _ => return false,
            }
        }
        if compact_layout {
            match action {
                Some(KeyAction::Edit) if dialog.config_focus == PluginConfigFocus::Toolbar => {
                    self.run_compact_toolbar_action(dialog);
                    return false;
                }
                Some(KeyAction::Edit) if dialog.config_focus == PluginConfigFocus::Structure => {
                    dialog.config_focus = PluginConfigFocus::Editor;
                    return false;
                }
                Some(KeyAction::MoveRight) if dialog.config_focus == PluginConfigFocus::Toolbar => {
                    move_index(
                        &mut dialog.selected_toolbar_action,
                        COMPACT_TOOLBAR_ACTIONS.len(),
                        1,
                    );
                    return false;
                }
                Some(KeyAction::MoveLeft) if dialog.config_focus == PluginConfigFocus::Toolbar => {
                    move_index(
                        &mut dialog.selected_toolbar_action,
                        COMPACT_TOOLBAR_ACTIONS.len(),
                        -1,
                    );
                    return false;
                }
                Some(KeyAction::MoveRight)
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    dialog.config_focus = PluginConfigFocus::Editor;
                    return false;
                }
                Some(KeyAction::MoveLeft)
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    return false;
                }
                Some(KeyAction::MoveLeft) if dialog.config_focus == PluginConfigFocus::Editor => {
                    if !self.move_selected_main_config_cell(dialog, -1) {
                        dialog.config_focus = PluginConfigFocus::Structure;
                    }
                    return false;
                }
                Some(KeyAction::MoveRight) if dialog.config_focus == PluginConfigFocus::Editor => {
                    self.move_selected_main_config_cell(dialog, 1);
                    return false;
                }
                Some(KeyAction::MoveUp) if dialog.config_focus == PluginConfigFocus::Toolbar => {
                    dialog.config_focus = PluginConfigFocus::Structure;
                    return false;
                }
                Some(KeyAction::MoveDown) if dialog.config_focus == PluginConfigFocus::Toolbar => {
                    dialog.config_focus = PluginConfigFocus::Structure;
                    return false;
                }
                _ => {}
            }
        }
        match action {
            Some(KeyAction::NextTab) => {
                dialog.config_focus = next_config_focus(dialog.config_focus, compact_layout);
                false
            }
            Some(KeyAction::PreviousTab) => {
                dialog.config_focus = previous_config_focus(dialog.config_focus, compact_layout);
                false
            }
            Some(KeyAction::Save) => {
                self.save_selected_plugin_config(dialog);
                false
            }
            Some(KeyAction::Validate) => {
                self.validate_selected_plugin_config(dialog);
                false
            }
            Some(KeyAction::InsertDefaults) => {
                self.insert_selected_plugin_defaults(dialog);
                false
            }
            Some(KeyAction::Actions) => {
                self.open_selected_config_actions(dialog);
                false
            }
            Some(KeyAction::Delete) => {
                if dialog.config_focus == PluginConfigFocus::Diagnostics {
                    self.jump_to_selected_bottom_item(dialog);
                } else {
                    self.delete_selected_config_node(dialog);
                }
                false
            }
            Some(KeyAction::ShowDiff) => {
                dialog.show_diff = !dialog.show_diff;
                if !compact_layout {
                    dialog.clamp_selection();
                }
                false
            }
            Some(KeyAction::Restart) => {
                self.restart_selected_plugin(dialog);
                false
            }
            Some(KeyAction::Reset) => {
                self.reset_selected_plugin_config_to_defaults(dialog);
                false
            }
            Some(KeyAction::Add) => {
                self.open_add_config_value_editor(dialog);
                false
            }
            Some(KeyAction::SelectType) => {
                self.open_config_type_selector(dialog);
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
            Some(KeyAction::PageUp) => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => {}
                    PluginConfigFocus::Structure => {
                        move_selected_config_section(dialog, -(CONFIG_EDITOR_PAGE_SIZE as isize));
                    }
                    PluginConfigFocus::Diagnostics => {
                        move_selected_bottom_panel_row(dialog, -(CONFIG_EDITOR_PAGE_SIZE as isize));
                    }
                    _ => move_selected_config_node(dialog, -(CONFIG_EDITOR_PAGE_SIZE as isize)),
                }
                false
            }
            Some(KeyAction::PageDown) => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => {}
                    PluginConfigFocus::Structure => {
                        move_selected_config_section(dialog, CONFIG_EDITOR_PAGE_SIZE as isize);
                    }
                    PluginConfigFocus::Diagnostics => {
                        move_selected_bottom_panel_row(dialog, CONFIG_EDITOR_PAGE_SIZE as isize);
                    }
                    _ => move_selected_config_node(dialog, CONFIG_EDITOR_PAGE_SIZE as isize),
                }
                false
            }
            Some(KeyAction::Home) => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => dialog.selected_toolbar_action = 0,
                    PluginConfigFocus::Structure => dialog.selected_section = 0,
                    PluginConfigFocus::Diagnostics => match dialog.show_diff {
                        true => dialog.selected_diff_row = 0,
                        false => dialog.selected_diagnostic = 0,
                    },
                    _ => dialog.selected_node = 0,
                }
                dialog.clamp_selection();
                false
            }
            Some(KeyAction::End) => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => {
                        dialog.selected_toolbar_action =
                            COMPACT_TOOLBAR_ACTIONS.len().saturating_sub(1);
                    }
                    PluginConfigFocus::Structure => {
                        dialog.selected_section = dialog
                            .selected_plugin()
                            .map(|plugin| plugin.sections.len().saturating_sub(1))
                            .unwrap_or_default();
                    }
                    PluginConfigFocus::Diagnostics => {
                        if dialog.show_diff {
                            dialog.selected_diff_row = dialog
                                .selected_plugin()
                                .map(|plugin| plugin.diff.len().saturating_sub(1))
                                .unwrap_or_default();
                        } else {
                            dialog.selected_diagnostic = dialog
                                .selected_plugin()
                                .map(plugin_all_diagnostics)
                                .map(|diagnostics| diagnostics.len().saturating_sub(1))
                                .unwrap_or_default();
                        }
                    }
                    _ => {
                        dialog.selected_node = dialog
                            .selected_section()
                            .map(|section| {
                                section_row_count(section, dialog.config_view).saturating_sub(1)
                            })
                            .unwrap_or_default();
                    }
                }
                dialog.clamp_selection();
                false
            }
            Some(KeyAction::MoveLeft) => {
                dialog.config_focus = previous_config_focus(dialog.config_focus, compact_layout);
                false
            }
            Some(KeyAction::MoveRight) => {
                dialog.config_focus = next_config_focus(dialog.config_focus, compact_layout);
                false
            }
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
                move_index(&mut overlay.selected_action, overlay.actions.len(), -1);
                false
            }
            Some(KeyAction::MoveDown) => {
                move_index(&mut overlay.selected_action, overlay.actions.len(), 1);
                false
            }
            Some(KeyAction::Home) => {
                overlay.selected_action = 0;
                false
            }
            Some(KeyAction::End) => {
                overlay.selected_action = overlay.actions.len().saturating_sub(1);
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
                move_index(&mut overlay.selected_item, overlay.items.len(), -1);
                false
            }
            Some(KeyAction::MoveDown) => {
                move_index(&mut overlay.selected_item, overlay.items.len(), 1);
                false
            }
            Some(KeyAction::Home) => {
                overlay.selected_item = 0;
                false
            }
            Some(KeyAction::End) => {
                overlay.selected_item = overlay.items.len().saturating_sub(1);
                false
            }
            Some(KeyAction::Toggle) if overlay.multi => {
                if let Some(item) = overlay.items.get_mut(overlay.selected_item) {
                    item.checked = !item.checked;
                }
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
            Some(KeyAction::PageUp) => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index_page(
                    &mut overlay.selected_row,
                    count,
                    -1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            Some(KeyAction::PageDown) => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index_page(&mut overlay.selected_row, count, 1, CONFIG_EDITOR_PAGE_SIZE);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            Some(KeyAction::Home) => {
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                overlay.selected_row = 0;
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            Some(KeyAction::End) => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                overlay.selected_row = count.saturating_sub(1);
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
            Some(KeyAction::Add) => {
                self.open_add_config_value_editor_for_path(
                    dialog,
                    overlay_snapshot.plugin_id.clone(),
                    overlay_snapshot.path.clone(),
                );
                false
            }
            Some(KeyAction::Actions) => {
                self.open_selected_config_actions(dialog);
                false
            }
            Some(KeyAction::SelectType) => {
                self.open_config_type_selector(dialog);
                false
            }
            Some(KeyAction::Delete) => {
                self.delete_drilldown_selected_row(dialog);
                false
            }
            Some(KeyAction::Edit) => {
                self.open_drilldown_selected_row_editor(dialog);
                false
            }
            _ => false,
        }
    }
}
use super::{
    App, COMPACT_TOOLBAR_ACTIONS, CONFIG_EDITOR_PAGE_SIZE, ConfigRowCell, Editor,
    EditorDialogKeyResult, KeyEvent, KeyModifiers, PLUGIN_WORKBENCH_LOG_LIMIT, PluginConfigFilter,
    PluginConfigFocus, PluginConfigView, PluginDetailTab, PluginTransportFilter,
    PluginWorkbenchMode, PluginWorkbenchOverlay, UiResult, build_plugin_workbench_plugin,
    drilldown_row_count, drilldown_selected_row_cell, drive_editor_dialog_key,
    filtered_plugin_indices, find_row_position, move_detail_scroll, move_index, move_index_page,
    move_selected_bottom_panel_row, move_selected_config_node, move_selected_config_section,
    next_config_filter, next_config_focus, next_transport_filter, plugin_all_diagnostics,
    plugin_uses_compact_config_layout, previous_config_focus, rebuild_drilldown_stack,
    refresh_plugin_workbench_filter, section_row_count,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
