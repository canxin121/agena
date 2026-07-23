impl App {
    pub(in crate::app) fn save_selected_plugin_config(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        let save_block = {
            let Some(plugin) = dialog.selected_plugin_mut() else {
                return;
            };
            recompute_plugin_config_state(plugin);
            plugin_save_block_reason(plugin)
        };
        if let Some(reason) = save_block {
            self.flash_error(reason);
            return;
        }
        let Some(plugin) = dialog.selected_plugin().cloned() else {
            return;
        };
        let mut configured_plugin_value = plugin_config_record_value(&plugin);
        let Some(plugin_object) = configured_plugin_value.as_object_mut() else {
            self.flash_error(format!(
                "plugin `{}` config record is not an object",
                plugin.plugin_id
            ));
            return;
        };
        plugin_object.insert("config".to_owned(), persisted_plugin_config_value(&plugin));
        let path = format!(
            "plugins.list.{}",
            quote_settings_segment(plugin.plugin_id.as_str())
        );
        match self.block_on_async(
            self.backend
                .set_config_setting(path.as_str(), configured_plugin_value),
        ) {
            Ok(_) => {
                self.flash_success(format!("saved plugin config for {}", plugin.plugin_id));
                self.refresh_plugin_workbench(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn validate_selected_plugin_config(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        recompute_plugin_config_state(plugin);
        if plugin
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            self.flash_warning(format!(
                "{} config has {} issue(s)",
                plugin.plugin_id,
                plugin.diagnostics.len()
            ));
        } else {
            self.flash_success(format!("{} config is valid", plugin.plugin_id));
        }
    }

    pub(in crate::app) fn run_compact_toolbar_action(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        action: CompactToolbarAction,
    ) {
        match action {
            CompactToolbarAction::Validate => self.validate_selected_plugin_config(dialog),
            CompactToolbarAction::ResetAll => self.reset_selected_plugin_config_to_defaults(dialog),
            CompactToolbarAction::Diff => {
                dialog.show_diff = !dialog.show_diff;
                dialog.clamp_selection();
            }
            CompactToolbarAction::Save => self.save_selected_plugin_config(dialog),
            CompactToolbarAction::Restart => self.restart_selected_plugin(dialog),
        }
    }

    pub(in crate::app) fn restart_selected_plugin(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        self.flash_info(format!(
            "restart is not available for {} from this screen",
            plugin.plugin_id
        ));
    }

    pub(in crate::app) fn reset_selected_plugin_config_to_defaults(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        plugin.draft_config = plugin.default_config.clone();
        plugin.draft_override = JsonValue::Null;
        plugin.branch_drafts.clear();
        recompute_plugin_config_state(plugin);
        self.flash_success(format!(
            "reset {} config to plugin defaults",
            plugin.plugin_id
        ));
    }

    pub(in crate::app) fn delete_selected_config_node(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        let Some(row) = dialog.selected_row().cloned() else {
            return;
        };
        if row.primary_path.is_empty() {
            self.flash_warning("root config cannot be deleted".to_owned());
            return;
        }
        let Some(selected_plugin_id) = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone())
        else {
            return;
        };
        let (changed, blocked) = if let Some(plugin) = dialog.selected_plugin_mut() {
            let outcome = apply_reset_paths(
                &mut plugin.draft_config,
                &plugin.default_config,
                plugin.schema.as_ref(),
                &row_paths(&row).into_iter().cloned().collect::<Vec<_>>(),
            );
            if outcome.changed {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            (outcome.changed, outcome.blocked)
        } else {
            (false, Vec::new())
        };
        if changed {
            select_config_path(
                dialog,
                selected_plugin_id.as_str(),
                row.primary_path.as_slice(),
            );
        }
        if let Some(message) = reset_paths_warning_message(blocked.as_slice()) {
            self.flash_warning(message);
        }
    }

    pub(in crate::app) fn open_selected_config_actions(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        let plugin = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == context.plugin_id);
        let primary_action = plugin.and_then(|plugin| {
            config_row_primary_action(
                plugin,
                &context.row.editor,
                context.row.primary_path.as_slice(),
                context.row.additional_paths.as_slice(),
            )
        });
        let mut actions = Vec::new();
        if context.row.type_mode.is_switchable() {
            let description = if context.row.type_mode == ConfigRowTypeMode::SelectShape {
                format!("Switch the active shape for {}.", context.row.title)
            } else {
                format!("Switch the active type for {}.", context.row.title)
            };
            actions.push(PluginConfigActionCandidate {
                label: context.row.type_mode.action_label().to_owned(),
                description,
                action: PluginConfigAction::SelectType {
                    plugin_id: context.plugin_id.clone(),
                    path: context.row.primary_path.clone(),
                },
            });
        }
        if let Some(plugin) = plugin
            && let ConfigRowEditor::Structured { path } = &context.row.editor
            && let Some(value) = get_value_at_path(&plugin.draft_config, path)
        {
            if value.is_array() && can_append_array_item(plugin, path.as_slice()) {
                actions.push(PluginConfigActionCandidate {
                    label: "Add Item".to_owned(),
                    description: format!("Append a new default item to {}.", context.row.title),
                    action: PluginConfigAction::AppendArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: path.clone(),
                    },
                });
            }
            if value.is_object()
                && object_add_field_block_reason(plugin.schema.as_ref(), &plugin.draft_config, path)
                    .is_none()
            {
                actions.push(PluginConfigActionCandidate {
                    label: "Add Field".to_owned(),
                    description: format!("Add a new field inside {}.", context.row.title),
                    action: PluginConfigAction::PromptAddObjectField {
                        plugin_id: context.plugin_id.clone(),
                        path: path.clone(),
                    },
                });
            }
        }
        if let Some(plugin) = plugin
            && let Some(info) = array_item_action_info(plugin, context.row.primary_path.as_slice())
        {
            if info.can_insert_before {
                actions.push(PluginConfigActionCandidate {
                    label: "Insert Before".to_owned(),
                    description: "Insert a new default item before this array item.".to_owned(),
                    action: PluginConfigAction::InsertArrayItemBefore {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
            if info.can_insert_after {
                actions.push(PluginConfigActionCandidate {
                    label: "Insert After".to_owned(),
                    description: "Insert a new default item after this array item.".to_owned(),
                    action: PluginConfigAction::InsertArrayItemAfter {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
            if info.can_duplicate {
                actions.push(PluginConfigActionCandidate {
                    label: "Duplicate Item".to_owned(),
                    description: format!("Duplicate {} inside this array.", context.row.title),
                    action: PluginConfigAction::DuplicateArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
            if info.can_move_up {
                actions.push(PluginConfigActionCandidate {
                    label: "Move Up".to_owned(),
                    description: "Move this array item one position earlier.".to_owned(),
                    action: PluginConfigAction::MoveArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                        direction: -1,
                    },
                });
            }
            if info.can_move_down {
                actions.push(PluginConfigActionCandidate {
                    label: "Move Down".to_owned(),
                    description: "Move this array item one position later.".to_owned(),
                    action: PluginConfigAction::MoveArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                        direction: 1,
                    },
                });
            }
            if info.can_remove {
                actions.push(PluginConfigActionCandidate {
                    label: "Remove Item".to_owned(),
                    description: format!("Remove {} from this array.", context.row.title),
                    action: PluginConfigAction::RemoveArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
        }
        let rename_allowed = context.row.additional_paths.is_empty()
            && match plugin {
                Some(plugin) => {
                    row_rename_action_allowed(plugin, context.row.primary_path.as_slice())
                }
                None => path_key_info(context.row.primary_path.as_slice()).is_some(),
            };
        if rename_allowed {
            actions.push(PluginConfigActionCandidate {
                label: "Rename Field".to_owned(),
                description: format!("Rename the key for {}.", context.row.title),
                action: PluginConfigAction::RenameField {
                    plugin_id: context.plugin_id.clone(),
                    path: context.row.primary_path.clone(),
                },
            });
        }
        let field_paths = row_paths(&context.row)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        actions.push(PluginConfigActionCandidate {
            label: "Reset Field".to_owned(),
            description: format!("Restore {} to the plugin default value.", context.row.title),
            action: PluginConfigAction::ResetField {
                plugin_id: context.plugin_id.clone(),
                paths: field_paths,
                focus_path: context.row.primary_path.clone(),
            },
        });
        actions.push(PluginConfigActionCandidate {
            label: "Reset Group".to_owned(),
            description: format!(
                "Restore every field in {} to the plugin defaults.",
                context.group_title
            ),
            action: PluginConfigAction::ResetGroup {
                plugin_id: context.plugin_id,
                paths: context.group_paths,
                focus_path: context.row.primary_path,
            },
        });
        let selected_action =
            prioritize_config_actions(actions.as_mut_slice(), context.cell, primary_action);
        let title = if primary_action.is_some() {
            "More Actions".to_owned()
        } else {
            "Field Actions".to_owned()
        };
        let rows = actions
            .iter()
            .enumerate()
            .map(
                |(index, item)| agena_tui::plugin_workbench::PluginConfigPickerItem {
                    key: format!("plugin-config-action:{index:08}"),
                    label: item.label.clone(),
                    detail: Some(item.description.clone()),
                    initially_selected: index == selected_action,
                },
            )
            .collect::<Vec<_>>();
        let actions = actions
            .into_iter()
            .enumerate()
            .map(|(index, item)| (format!("plugin-config-action:{index:08}"), item.action))
            .collect();
        let presentation = agena_tui::plugin_workbench::new_plugin_config_picker(
            title,
            context.row.title,
            config_actions_overlay_footer(primary_action),
            "No actions available".to_owned(),
            false,
            rows,
        );
        dialog.actions = Some(PluginConfigActionOverlay {
            presentation,
            actions,
        });
    }

    pub(in crate::app) fn commit_plugin_config_action(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        key: String,
    ) {
        let Some(overlay) = dialog.actions.clone() else {
            return;
        };
        let Some(action) = overlay.actions.get(key.as_str()).cloned() else {
            dialog.actions = None;
            return;
        };
        match action {
            PluginConfigAction::SelectType { plugin_id, path } => {
                self.focus_config_path(dialog, plugin_id.as_str(), path.as_slice());
                dialog.clamp_selection();
                self.open_config_type_selector(dialog);
            }
            PluginConfigAction::AppendArrayItem { plugin_id, path } => {
                self.append_config_array_item(dialog, plugin_id.as_str(), path.as_slice());
            }
            PluginConfigAction::PromptAddObjectField { plugin_id, path } => {
                self.open_add_config_value_editor_for_path(dialog, plugin_id, path);
            }
            PluginConfigAction::InsertArrayItemBefore { plugin_id, path } => {
                self.insert_array_item(dialog, plugin_id.as_str(), path.as_slice(), false);
            }
            PluginConfigAction::InsertArrayItemAfter { plugin_id, path } => {
                self.insert_array_item(dialog, plugin_id.as_str(), path.as_slice(), true);
            }
            PluginConfigAction::DuplicateArrayItem { plugin_id, path } => {
                self.duplicate_array_item(dialog, plugin_id.as_str(), path.as_slice());
            }
            PluginConfigAction::MoveArrayItem {
                plugin_id,
                path,
                direction,
            } => {
                self.move_array_item(dialog, plugin_id.as_str(), path.as_slice(), direction);
            }
            PluginConfigAction::RemoveArrayItem { plugin_id, path } => {
                self.remove_array_item(dialog, plugin_id.as_str(), path.as_slice());
            }
            PluginConfigAction::RenameField { plugin_id, path } => {
                self.open_rename_field_editor(dialog, plugin_id, path);
            }
            PluginConfigAction::ResetField {
                plugin_id,
                paths,
                focus_path,
            }
            | PluginConfigAction::ResetGroup {
                plugin_id,
                paths,
                focus_path,
            } => {
                self.reset_config_paths(dialog, plugin_id.as_str(), paths.as_slice(), &focus_path);
            }
        }
        dialog.actions = None;
    }

    pub(in crate::app) fn jump_to_selected_bottom_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        let target_path = if dialog.show_diff {
            plugin
                .diff
                .get(dialog.selected_diff_row)
                .map(|row| row.path.clone())
        } else {
            plugin_all_diagnostics(plugin)
                .get(dialog.selected_diagnostic)
                .map(|diagnostic| diagnostic.path.clone())
        };
        let Some(target_path) = target_path else {
            return;
        };
        let plugin_id = plugin.plugin_id.clone();
        if dialog
            .selected_plugin()
            .and_then(|plugin| find_row_position(plugin, dialog.config_view, &target_path))
            .is_none()
        {
            dialog.config_view = PluginConfigView::Effective;
        }
        self.focus_config_path(dialog, plugin_id.as_str(), target_path.as_slice());
        dialog.config_focus = PluginConfigFocus::Editor;
    }

    pub(in crate::app) fn reset_config_paths(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        paths: &[ConfigPath],
        focus_path: &ConfigPath,
    ) {
        let (changed, blocked) = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            let outcome = apply_reset_paths(
                &mut plugin.draft_config,
                &plugin.default_config,
                plugin.schema.as_ref(),
                paths,
            );
            if outcome.changed {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            (outcome.changed, outcome.blocked)
        } else {
            (false, Vec::new())
        };
        if changed {
            self.focus_config_path(dialog, plugin_id, focus_path.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
        if let Some(message) = reset_paths_warning_message(blocked.as_slice()) {
            self.flash_warning(message);
        }
    }

    pub(in crate::app) fn duplicate_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            if !array_item_action_info(plugin, path).is_some_and(|info| info.can_duplicate) {
                self.flash_warning("cannot duplicate this array item".to_owned());
                return;
            }
            let focus = duplicate_array_item_at_path(&mut plugin.draft_config, path);
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
    }

    pub(in crate::app) fn insert_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
        after: bool,
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            let focus = insert_default_array_item_at_path(
                &mut plugin.draft_config,
                plugin.schema.as_ref(),
                path,
                after,
            );
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
            self.maybe_open_type_selector_for_selected_row(dialog, plugin_id, focus.as_slice());
        } else {
            self.flash_warning("cannot insert an item at this array position".to_owned());
        }
    }

    pub(in crate::app) fn move_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
        direction: isize,
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            let focus = move_array_item_at_path(&mut plugin.draft_config, path, direction);
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
    }

    pub(in crate::app) fn remove_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            if !array_item_action_info(plugin, path).is_some_and(|info| info.can_remove) {
                self.flash_warning("cannot remove this array item".to_owned());
                return;
            }
            let focus = remove_array_item_at_path(&mut plugin.draft_config, path);
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
    }

    pub(in crate::app) fn open_rename_field_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
    ) {
        let Some((_, key)) = path_key_info(path.as_slice()) else {
            self.flash_warning("selected row does not point to an object field".to_owned());
            return;
        };
        dialog.editor = Some(EditorDialogState::new(
            format!("Rename {}", title_from_key(key.as_str())),
            "Enter the new field name.".to_owned(),
            "Type to edit".to_owned(),
            false,
            Editor::from_text(key),
            PluginConfigEditAction::RenameObjectField { plugin_id, path },
        ));
    }

    pub(in crate::app) fn focus_config_path(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        target_path: &[PathSegment],
    ) {
        dialog.drilldown_stack.clear();
        select_config_path(dialog, plugin_id, target_path);
        if dialog
            .selected_plugin()
            .and_then(|plugin| find_row_position(plugin, dialog.config_view, target_path))
            .is_some()
        {
            return;
        }
        let Some((section_index, row_index, row)) = dialog.selected_plugin().and_then(|plugin| {
            find_best_section_row_for_path(plugin, dialog.config_view, target_path)
        }) else {
            return;
        };
        dialog.selected_section = section_index;
        dialog.selected_node = row_index;
        dialog.clamp_selection();
        let ConfigRowEditor::Structured { path } = &row.editor else {
            return;
        };
        self.open_structured_row_drilldown(
            dialog,
            plugin_id.to_owned(),
            path.clone(),
            row.title.clone(),
        );
        self.focus_drilldown_path(dialog, target_path);
    }

    pub(in crate::app) fn focus_drilldown_path(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        target_path: &[PathSegment],
    ) {
        loop {
            let Some(overlay) = dialog.current_drilldown().cloned() else {
                return;
            };
            let Some((row_index, row)) =
                find_best_drilldown_row_for_path(&overlay, dialog.config_view, target_path)
            else {
                return;
            };
            if let Some(current) = dialog.current_drilldown_mut() {
                current.selected_row = row_index;
            }
            if row.primary_path.as_slice() == target_path
                || row
                    .additional_paths
                    .iter()
                    .any(|candidate| candidate.as_slice() == target_path)
            {
                return;
            }
            let ConfigRowEditor::Structured { path } = &row.editor else {
                return;
            };
            self.open_structured_row_drilldown(
                dialog,
                overlay.plugin_id.clone(),
                path.clone(),
                row.title.clone(),
            );
        }
    }
}
use super::{
    App, CompactToolbarAction, ConfigPath, ConfigRowEditor, ConfigRowTypeMode, DiagnosticSeverity,
    Editor, EditorDialogState, JsonValue, PathSegment, PluginConfigAction,
    PluginConfigActionCandidate, PluginConfigActionOverlay, PluginConfigEditAction,
    PluginConfigFocus, PluginConfigView, PluginWorkbenchOverlay, apply_reset_paths,
    array_item_action_info, can_append_array_item, clear_branch_drafts_for_structural_change,
    config_actions_overlay_footer, config_row_primary_action, duplicate_array_item_at_path,
    find_best_drilldown_row_for_path, find_best_section_row_for_path, find_row_position,
    get_value_at_path, insert_default_array_item_at_path, move_array_item_at_path,
    object_add_field_block_reason, path_key_info, persisted_plugin_config_value,
    plugin_all_diagnostics, plugin_config_record_value, plugin_save_block_reason,
    prioritize_config_actions, quote_settings_segment, rebuild_drilldown_stack,
    recompute_plugin_config_state, remove_array_item_at_path, reset_paths_warning_message,
    row_paths, row_rename_action_allowed, select_config_path, selected_config_row_context,
    title_from_key,
};
