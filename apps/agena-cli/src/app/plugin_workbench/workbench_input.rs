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
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.mode = PluginWorkbenchMode::Detail;
                dialog.detail_tab = PluginDetailTab::Config;
                dialog.selected_section = 0;
                dialog.selected_node = 0;
                false
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_plugin_workbench(dialog);
                false
            }
            KeyCode::Char('t') => {
                dialog.transport_filter = next_transport_filter(dialog.transport_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            KeyCode::Char('c') => {
                dialog.config_filter = next_config_filter(dialog.config_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            KeyCode::PageUp => {
                move_index_page(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    -1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                false
            }
            KeyCode::PageDown => {
                move_index_page(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                false
            }
            KeyCode::Home => {
                dialog.selected_plugin = 0;
                false
            }
            KeyCode::End => {
                dialog.selected_plugin = dialog.visible_plugins.len().saturating_sub(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_index(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    -1,
                );
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
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
            return match key.code {
                KeyCode::Esc => {
                    dialog.mode = PluginWorkbenchMode::List;
                    false
                }
                KeyCode::Left | KeyCode::Char('h')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    dialog.detail_tab = dialog.detail_tab.move_by(-1);
                    false
                }
                KeyCode::Right | KeyCode::Char('l')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    dialog.detail_tab = dialog.detail_tab.move_by(1);
                    false
                }
                _ => self.handle_plugin_config_key(key, dialog),
            };
        }

        match key.code {
            KeyCode::Esc => {
                dialog.mode = PluginWorkbenchMode::List;
                false
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_plugin_workbench(dialog);
                false
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                dialog.detail_tab = dialog.detail_tab.move_by(1);
                false
            }
            KeyCode::BackTab => {
                dialog.detail_tab = dialog.detail_tab.move_by(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.detail_tab = dialog.detail_tab.move_by(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.detail_tab = dialog.detail_tab.move_by(-1);
                false
            }
            KeyCode::PageUp if dialog.detail_tab != PluginDetailTab::Config => {
                move_detail_scroll(dialog, -10);
                false
            }
            KeyCode::PageDown if dialog.detail_tab != PluginDetailTab::Config => {
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
        if compact_layout && dialog.show_diff {
            match key.code {
                KeyCode::Esc | KeyCode::Char('d') | KeyCode::Char('D') => {
                    dialog.show_diff = false;
                    return false;
                }
                _ => return false,
            }
        }
        if compact_layout {
            match key.code {
                KeyCode::Enter if dialog.config_focus == PluginConfigFocus::Toolbar => {
                    self.run_compact_toolbar_action(dialog);
                    return false;
                }
                KeyCode::Enter if dialog.config_focus == PluginConfigFocus::Structure => {
                    dialog.config_focus = PluginConfigFocus::Editor;
                    return false;
                }
                KeyCode::Right | KeyCode::Char('l')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    move_index(
                        &mut dialog.selected_toolbar_action,
                        COMPACT_TOOLBAR_ACTIONS.len(),
                        1,
                    );
                    return false;
                }
                KeyCode::Left | KeyCode::Char('h')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    move_index(
                        &mut dialog.selected_toolbar_action,
                        COMPACT_TOOLBAR_ACTIONS.len(),
                        -1,
                    );
                    return false;
                }
                KeyCode::Right | KeyCode::Char('l')
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    dialog.config_focus = PluginConfigFocus::Editor;
                    return false;
                }
                KeyCode::Left | KeyCode::Char('h')
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    return false;
                }
                KeyCode::Left | KeyCode::Char('h')
                    if dialog.config_focus == PluginConfigFocus::Editor =>
                {
                    if !self.move_selected_main_config_cell(dialog, -1) {
                        dialog.config_focus = PluginConfigFocus::Structure;
                    }
                    return false;
                }
                KeyCode::Right | KeyCode::Char('l')
                    if dialog.config_focus == PluginConfigFocus::Editor =>
                {
                    self.move_selected_main_config_cell(dialog, 1);
                    return false;
                }
                KeyCode::Up | KeyCode::Char('k')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    dialog.config_focus = PluginConfigFocus::Structure;
                    return false;
                }
                KeyCode::Down | KeyCode::Char('j')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    dialog.config_focus = PluginConfigFocus::Structure;
                    return false;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Tab if key.modifiers.is_empty() => {
                dialog.config_focus = next_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::BackTab => {
                dialog.config_focus = previous_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.save_selected_plugin_config(dialog);
                false
            }
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.validate_selected_plugin_config(dialog);
                false
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                self.insert_selected_plugin_defaults(dialog);
                false
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.open_selected_config_actions(dialog);
                false
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if dialog.config_focus == PluginConfigFocus::Diagnostics {
                    self.jump_to_selected_bottom_item(dialog);
                } else {
                    self.delete_selected_config_node(dialog);
                }
                false
            }
            KeyCode::Char('D') => {
                dialog.show_diff = !dialog.show_diff;
                if !compact_layout {
                    dialog.clamp_selection();
                }
                false
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.restart_selected_plugin(dialog);
                false
            }
            KeyCode::Char('r') => {
                self.reset_selected_plugin_config_to_defaults(dialog);
                false
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.open_add_config_value_editor(dialog);
                false
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.open_config_type_selector(dialog);
                false
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if dialog.config_focus == PluginConfigFocus::Diagnostics {
                    self.jump_to_selected_bottom_item(dialog);
                } else {
                    self.open_selected_config_value_editor(dialog);
                }
                false
            }
            KeyCode::PageUp => {
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
            KeyCode::PageDown => {
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
            KeyCode::Home => {
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
            KeyCode::End => {
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
            KeyCode::Left | KeyCode::Char('h') => {
                dialog.config_focus = previous_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                dialog.config_focus = next_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match dialog.config_focus {
                    PluginConfigFocus::Structure => move_selected_config_section(dialog, -1),
                    PluginConfigFocus::Diagnostics => move_selected_bottom_panel_row(dialog, -1),
                    _ => move_selected_config_node(dialog, -1),
                }
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
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
        match key.code {
            KeyCode::Esc => {
                dialog.actions = None;
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_index(&mut overlay.selected_action, overlay.actions.len(), -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_index(&mut overlay.selected_action, overlay.actions.len(), 1);
                false
            }
            KeyCode::Home => {
                overlay.selected_action = 0;
                false
            }
            KeyCode::End => {
                overlay.selected_action = overlay.actions.len().saturating_sub(1);
                false
            }
            KeyCode::Enter => {
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
        match key.code {
            KeyCode::Esc => {
                dialog.selection = None;
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_index(&mut overlay.selected_item, overlay.items.len(), -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_index(&mut overlay.selected_item, overlay.items.len(), 1);
                false
            }
            KeyCode::Home => {
                overlay.selected_item = 0;
                false
            }
            KeyCode::End => {
                overlay.selected_item = overlay.items.len().saturating_sub(1);
                false
            }
            KeyCode::Char(' ') if overlay.multi => {
                if let Some(item) = overlay.items.get_mut(overlay.selected_item) {
                    item.checked = !item.checked;
                }
                false
            }
            KeyCode::Enter => {
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
        match key.code {
            KeyCode::Esc => {
                dialog.drilldown_stack.pop();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index(&mut overlay.selected_row, count, -1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index(&mut overlay.selected_row, count, 1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::PageUp => {
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
            KeyCode::PageDown => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index_page(&mut overlay.selected_row, count, 1, CONFIG_EDITOR_PAGE_SIZE);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::Home => {
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                overlay.selected_row = 0;
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::End => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                overlay.selected_row = count.saturating_sub(1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.move_selected_drilldown_cell(dialog, -1);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.move_selected_drilldown_cell(dialog, 1);
                false
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.open_add_config_value_editor_for_path(
                    dialog,
                    overlay_snapshot.plugin_id.clone(),
                    overlay_snapshot.path.clone(),
                );
                false
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.open_selected_config_actions(dialog);
                false
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.open_config_type_selector(dialog);
                false
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_drilldown_selected_row(dialog);
                false
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                self.open_drilldown_selected_row_editor(dialog);
                false
            }
            _ => false,
        }
    }
}
use super::{
    App, COMPACT_TOOLBAR_ACTIONS, CONFIG_EDITOR_PAGE_SIZE, ConfigRowCell, Editor,
    EditorDialogKeyResult, KeyCode, KeyEvent, KeyModifiers, PLUGIN_WORKBENCH_LOG_LIMIT,
    PluginConfigFilter, PluginConfigFocus, PluginConfigView, PluginDetailTab,
    PluginTransportFilter, PluginWorkbenchMode, PluginWorkbenchOverlay, UiResult,
    build_plugin_workbench_plugin, drilldown_row_count, drilldown_selected_row_cell,
    drive_editor_dialog_key, filtered_plugin_indices, find_row_position, move_detail_scroll,
    move_index, move_index_page, move_selected_bottom_panel_row, move_selected_config_node,
    move_selected_config_section, next_config_filter, next_config_focus, next_transport_filter,
    plugin_all_diagnostics, plugin_uses_compact_config_layout, previous_config_focus,
    rebuild_drilldown_stack, refresh_plugin_workbench_filter, section_row_count,
};
