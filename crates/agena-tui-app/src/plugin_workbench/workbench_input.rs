impl App {
    pub(crate) fn open_plugin_workbench_detail(&mut self, plugin_id: &str, tab: Option<&str>) {
        let detail_tab = match tab {
            None => PluginDetailTab::Config,
            Some(tab) => match PluginDetailTab::from_id(tab) {
                Some(tab) => tab,
                None => {
                    self.flash_warning(format!(
                        "plugin `{plugin_id}` requested unsupported Workbench tab `{tab}`; opening Config"
                    ));
                    PluginDetailTab::Config
                }
            },
        };
        match self.build_plugin_workbench("") {
            Ok(mut workbench) => {
                if !workbench.open_plugin_detail(plugin_id, detail_tab) {
                    self.flash_warning(format!("plugin `{plugin_id}` is not available"));
                    return;
                }
                self.current_route = Route::PluginWorkbench(Box::new(workbench));
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn build_plugin_workbench(&self, query: &str) -> UiResult<PluginWorkbenchOverlay> {
        let sources = crate::app_backend::config::config_json_sources(&self.application)
            .map_err(crate::UiFailure::internal)?;
        let locale = self.i18n.locale_tag();
        let statuses = crate::app_backend::plugin_effects::plugin_statuses(&self.application);
        let mut plugins = statuses
            .into_iter()
            .map(|status| {
                let plugin_id = status.plugin_id.clone();
                let inspect = crate::app_backend::plugin_effects::plugin_inspect(
                    &self.application,
                    &plugin_id,
                );
                let logs = crate::app_backend::plugin_effects::plugin_logs(
                    &self.application,
                    &plugin_id,
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
            selected_tool: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            drilldown_stack: Vec::new(),
            actions: None,
            selection: None,
            editor: None,
            tool_editor: None,
            tool_result: None,
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
                refreshed.selected_tool = dialog.selected_tool;
                refreshed.show_diff = dialog.show_diff;
                refreshed.tool_result = dialog.tool_result.clone();
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
        refreshed.selected_tool = dialog.selected_tool;
        refreshed.drilldown_stack =
            rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
        refreshed.actions = dialog.actions.clone();
        refreshed.selection = dialog.selection.clone();
        refreshed.tool_editor = dialog.tool_editor.clone();
        refreshed.tool_result = dialog.tool_result.clone();
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
        if let Some(editor) = dialog.tool_editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => return false,
                EditorDialogKeyResult::Close => {
                    dialog.tool_editor = None;
                    return false;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) =
                        self.commit_plugin_tool_editor(dialog, action, input.as_str())
                    {
                        self.flash_error(error);
                    } else {
                        dialog.tool_editor = None;
                    }
                    return false;
                }
            }
        }
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

        if dialog.navigation.detail_tab == PluginDetailTab::Tools {
            match resolve_tui_key(KeyContext::PluginDetail, key) {
                Some(KeyAction::MoveUp) => {
                    let count = dialog
                        .selected_plugin()
                        .map(|plugin| plugin.tools.len())
                        .unwrap_or_default();
                    move_index(&mut dialog.selected_tool, count, -1);
                    dialog.tool_result = None;
                    return false;
                }
                Some(KeyAction::MoveDown) => {
                    let count = dialog
                        .selected_plugin()
                        .map(|plugin| plugin.tools.len())
                        .unwrap_or_default();
                    move_index(&mut dialog.selected_tool, count, 1);
                    dialog.tool_result = None;
                    return false;
                }
                Some(KeyAction::Open) => {
                    self.open_selected_plugin_tool_editor(dialog);
                    return false;
                }
                _ => {}
            }
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

    fn open_selected_plugin_tool_editor(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        let Some(tool) = plugin.tools.get(dialog.selected_tool) else {
            self.flash_warning("this plugin does not expose any tools".to_owned());
            return;
        };
        let mut input =
            default_value_for_schema(&tool.contract.input_schema, &tool.contract.input_schema);
        if input.is_null() {
            input = serde_json::json!({});
        }
        let input = serde_json::to_string_pretty(&input).unwrap_or_else(|_| "{}".to_owned());
        let summary = tool
            .docs
            .help
            .as_deref()
            .or(tool.docs.summary.as_deref())
            .unwrap_or("Run this plugin tool.");
        let metadata_tags = if tool.tags.is_empty() {
            "none declared".to_owned()
        } else {
            tool.tags
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        dialog.tool_editor = Some(EditorDialogState::new(
            format!("Run {} / {}", plugin.plugin_id, tool.name),
            format!(
                "{summary} Tags: {metadata_tags}. Edit the JSON arguments below. Submitting is a one-shot approval for this exact tool call; persisted deny rules still apply."
            ),
            "Ctrl+S validate and run · Esc cancel · Enter newline".to_owned(),
            true,
            Editor::from_text(input),
            PluginToolInvocationAction {
                plugin_id: plugin.plugin_id.clone(),
                tool_name: tool.name.clone(),
            },
        ));
    }

    fn commit_plugin_tool_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        action: PluginToolInvocationAction,
        input: &str,
    ) -> UiResult<()> {
        let value = serde_json::from_str::<JsonValue>(input).map_err(|error| {
            crate::UiFailure::invalid_with_diagnostic(
                "The tool arguments are not valid JSON.",
                error,
            )
        })?;
        if !value.is_object() {
            return Err(crate::UiFailure::message(
                "plugin tool arguments must be a JSON object",
            ));
        }
        let schema = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == action.plugin_id)
            .and_then(|plugin| {
                plugin
                    .tools
                    .iter()
                    .find(|tool| tool.name == action.tool_name)
            })
            .map(|tool| tool.contract.input_schema.clone())
            .ok_or_else(|| crate::UiFailure::message("The plugin tool is no longer available."))?;
        agena_plugin_host::loader::validate_json_schema_value(&schema, &value).map_err(
            |error| {
                crate::UiFailure::invalid_with_diagnostic(
                    "The tool arguments do not match the plugin schema.",
                    error,
                )
            },
        )?;
        let session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
            .ok_or_else(|| {
                crate::UiFailure::message("The plugin tool requires an active session.")
            })?;
        let plugin_id = action.plugin_id;
        let tool_name = action.tool_name;
        let request_plugin_id = plugin_id.clone();
        let request_tool_name = tool_name.clone();
        self.dispatch_backend_operation(
            move |application| async move {
                crate::app_backend::plugin_effects::invoke_plugin_workbench_tool(
                    &application,
                    request_plugin_id.as_str(),
                    request_tool_name.as_str(),
                    value,
                    Some(session_id),
                )
                .await
            },
            move |app, result| {
                let (output, succeeded) = match result {
                    Ok(output) => {
                        app.flash_success("plugin tool completed".to_owned());
                        (
                            if output.trim().is_empty() {
                                "Tool completed successfully with no output.".to_owned()
                            } else {
                                output
                            },
                            true,
                        )
                    }
                    Err(error) => {
                        let error = error.to_string();
                        app.flash_error(error.clone());
                        (error, false)
                    }
                };
                let route = std::mem::replace(&mut app.current_route, Route::Main);
                app.current_route = match route {
                    Route::PluginWorkbench(mut dialog) => {
                        dialog.tool_result = Some(PluginToolInvocationResult {
                            plugin_id,
                            tool_name,
                            output,
                            succeeded,
                        });
                        Route::PluginWorkbench(dialog)
                    }
                    route => route,
                };
            },
        );
        Ok(())
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
            Some(KeyAction::Close) => agena_tui_plugin_workbench::PluginConfigPickerAction::Close,
            Some(KeyAction::MoveUp) => agena_tui_plugin_workbench::PluginConfigPickerAction::MoveUp,
            Some(KeyAction::MoveDown) => {
                agena_tui_plugin_workbench::PluginConfigPickerAction::MoveDown
            }
            Some(KeyAction::PageUp) => agena_tui_plugin_workbench::PluginConfigPickerAction::PageUp,
            Some(KeyAction::PageDown) => {
                agena_tui_plugin_workbench::PluginConfigPickerAction::PageDown
            }
            Some(KeyAction::Accept) => agena_tui_plugin_workbench::PluginConfigPickerAction::Accept,
            _ => return false,
        };
        let effect = match dialog.actions.as_mut() {
            Some(overlay) => agena_tui_plugin_workbench::reduce_plugin_config_picker(
                &mut overlay.presentation,
                action,
            ),
            None => return false,
        };
        match effect {
            agena_tui_plugin_workbench::PluginConfigPickerEffect::Close => dialog.actions = None,
            agena_tui_plugin_workbench::PluginConfigPickerEffect::Activate { key } => {
                self.commit_plugin_config_action(dialog, key);
            }
            agena_tui_plugin_workbench::PluginConfigPickerEffect::KeepOpen => {}
        }
        false
    }

    pub(crate) fn handle_plugin_config_selection_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let action = match resolve_tui_key(KeyContext::PluginConfigSelection, key) {
            Some(KeyAction::Close) => agena_tui_plugin_workbench::PluginConfigPickerAction::Close,
            Some(KeyAction::MoveUp) => agena_tui_plugin_workbench::PluginConfigPickerAction::MoveUp,
            Some(KeyAction::MoveDown) => {
                agena_tui_plugin_workbench::PluginConfigPickerAction::MoveDown
            }
            Some(KeyAction::PageUp) => agena_tui_plugin_workbench::PluginConfigPickerAction::PageUp,
            Some(KeyAction::PageDown) => {
                agena_tui_plugin_workbench::PluginConfigPickerAction::PageDown
            }
            Some(KeyAction::Toggle) => agena_tui_plugin_workbench::PluginConfigPickerAction::Toggle,
            Some(KeyAction::Accept) => agena_tui_plugin_workbench::PluginConfigPickerAction::Accept,
            _ => return false,
        };
        let effect = match dialog.selection.as_mut() {
            Some(overlay) => agena_tui_plugin_workbench::reduce_plugin_config_picker(
                &mut overlay.presentation,
                action,
            ),
            None => return false,
        };
        match effect {
            agena_tui_plugin_workbench::PluginConfigPickerEffect::Close => dialog.selection = None,
            agena_tui_plugin_workbench::PluginConfigPickerEffect::Activate { key } => {
                if let Err(error) = self.commit_plugin_config_selection(dialog, key) {
                    self.flash_error(error);
                } else {
                    dialog.selection = None;
                }
            }
            agena_tui_plugin_workbench::PluginConfigPickerEffect::KeepOpen => {}
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
    App, CompactToolbarAction, ConfigRowCell, Editor, EditorDialogKeyResult, EditorDialogState,
    JsonValue, KeyEvent, PLUGIN_WORKBENCH_LOG_LIMIT, PluginConfigFocus, PluginConfigView,
    PluginDetailTab, PluginToolInvocationAction, PluginToolInvocationResult,
    PluginWorkbenchListPresentation, PluginWorkbenchMode, PluginWorkbenchNavigation,
    PluginWorkbenchOverlay, UiResult, build_plugin_workbench_plugin, default_value_for_schema,
    drilldown_row_count, drilldown_selected_row_cell, drive_editor_dialog_key, find_row_position,
    move_detail_scroll, move_index, move_selected_bottom_panel_row, move_selected_config_node,
    move_selected_config_section, next_config_focus, plugin_uses_compact_config_layout,
    plugin_workbench_list_items, previous_config_focus, rebuild_drilldown_stack,
};
use crate::Route;
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui_plugin_workbench::{
    PluginWorkbenchListEffect, PluginWorkbenchNavigationEffect,
    handle_key as handle_plugin_workbench_navigation_key,
    handle_list_key as handle_plugin_workbench_list_key,
};
