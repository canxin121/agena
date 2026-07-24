impl App {
    pub(crate) fn open_add_config_value_editor_for_path(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let value = get_value_at_path(&plugin.draft_config, &path).unwrap_or(&JsonValue::Null);
        if value.is_object() {
            if let Some(reason) =
                object_add_field_block_reason(plugin.schema.as_ref(), &plugin.draft_config, &path)
            {
                self.flash_warning(reason);
                return;
            }
            dialog.editor = Some(EditorDialogState::new(
                "Add Field".to_owned(),
                format!(
                    "Enter a field name for {}. If the schema allows multiple value types or shapes, the editor will prompt you after create.",
                    path_display(&path)
                ),
                "Type to edit".to_owned(),
                false,
                Editor::default(),
                PluginConfigEditAction::AddObjectField { plugin_id, path },
            ));
        } else if value.is_array() {
            self.append_config_array_item(dialog, plugin_id.as_str(), path.as_slice());
        } else {
            self.flash_warning("add is available for object and array nodes".to_owned());
        }
    }

    pub(crate) fn append_config_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let Some(plugin_index) = dialog
            .plugins
            .iter()
            .position(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let (plugin_id, focus_path, can_append) = {
            let plugin = &mut dialog.plugins[plugin_index];
            let can_append = can_append_array_item(plugin, path);
            let focus_path = append_default_array_item_at_path(
                &mut plugin.draft_config,
                plugin.schema.as_ref(),
                path,
            );
            if focus_path.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            (plugin.plugin_id.clone(), focus_path, can_append)
        };
        if let Some(focus_path) = focus_path {
            self.focus_config_path(dialog, plugin_id.as_str(), focus_path.as_slice());
            dialog.clamp_selection();
            self.maybe_open_type_selector_for_selected_row(
                dialog,
                plugin_id.as_str(),
                focus_path.as_slice(),
            );
        } else if !can_append {
            self.flash_warning("cannot add another item at this array position".to_owned());
        } else {
            self.flash_warning("failed to append array item".to_owned());
        }
    }

    pub(crate) fn move_selected_main_config_cell(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        delta: isize,
    ) -> bool {
        let Some(section) = dialog.selected_section() else {
            return false;
        };
        let Some(row) = section_row_at(section, dialog.config_view, dialog.selected_node) else {
            return false;
        };
        let layout = section_group_for_row(section, dialog.config_view, dialog.selected_node)
            .map(|group| group.layout)
            .unwrap_or(ConfigGroupLayout::Standard);
        let Some(next) = move_config_row_cell(row, layout, dialog.selected_cell, delta) else {
            return false;
        };
        dialog.selected_cell = next;
        true
    }

    pub(crate) fn move_selected_drilldown_cell(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        delta: isize,
    ) -> bool {
        let Some(overlay_snapshot) = dialog.current_drilldown().cloned() else {
            return false;
        };
        let Some(row) = drilldown_row_at(
            &overlay_snapshot,
            dialog.config_view,
            overlay_snapshot.selected_row,
        ) else {
            return false;
        };
        let layout = drilldown_group_for_row(
            &overlay_snapshot,
            dialog.config_view,
            overlay_snapshot.selected_row,
        )
        .map(|group| group.layout)
        .unwrap_or(ConfigGroupLayout::Standard);
        let Some(next) = move_config_row_cell(row, layout, overlay_snapshot.selected_cell, delta)
        else {
            return false;
        };
        let Some(overlay) = dialog.current_drilldown_mut() else {
            return false;
        };
        overlay.selected_cell = next;
        true
    }

    pub(crate) fn open_config_type_selector(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        if dialog.current_drilldown().is_some() {
            if let Some(overlay) = dialog.current_drilldown_mut() {
                overlay.selected_cell = ConfigRowCell::Type;
            }
        } else {
            dialog.selected_cell = ConfigRowCell::Type;
        }
        self.open_type_selector_for_row(dialog, context.plugin_id, context.row);
    }

    pub(crate) fn maybe_open_type_selector_for_selected_row(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        if context.plugin_id != plugin_id
            || context.row.primary_path.as_slice() != path
            || !context.row.type_mode.is_switchable()
        {
            return;
        }
        self.open_type_selector_for_row(dialog, context.plugin_id, context.row);
    }

    pub(crate) fn open_type_selector_for_row(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
    ) {
        if dialog.current_drilldown().is_some() {
            if let Some(overlay) = dialog.current_drilldown_mut() {
                overlay.selected_cell = ConfigRowCell::Type;
            }
        } else {
            dialog.selected_cell = ConfigRowCell::Type;
        }
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let schema = plugin.schema.as_ref().and_then(|schema| {
            declared_schema_for_path(schema, schema, &plugin.draft_config, &row.primary_path)
        });
        if let Some(branches) = schema.as_ref().and_then(|schema| {
            plugin
                .schema
                .as_ref()
                .and_then(|root| branch_choices(root, schema))
        }) {
            self.open_branch_selection_overlay(
                dialog,
                "Select Branch".to_owned(),
                "Choose schema shape".to_owned(),
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                branches,
                get_value_at_path(&plugin.draft_config, &row.primary_path)
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            return;
        }
        let current_value = get_value_at_path(&plugin.draft_config, &row.primary_path)
            .cloned()
            .unwrap_or(JsonValue::Null);
        let choices = schema_type_selector_choices(schema.as_ref());
        if choices.is_empty() {
            self.flash_warning(format!("{} has no selectable schema type", row.title));
            return;
        }
        if choices.len() == 1 {
            let choice = choices[0].clone();
            if value_matches_type(&current_value, choice.as_str()) {
                self.flash_warning(format!("{} has a fixed schema type", row.title));
                return;
            }
            let value = schema
                .as_ref()
                .map(|schema| {
                    default_value_for_schema(schema, plugin.schema.as_ref().unwrap_or(schema))
                })
                .unwrap_or_else(|| JsonValue::Null);
            self.set_config_value_at(
                dialog,
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                value,
            );
            return;
        }
        self.open_named_selection_overlay(
            dialog,
            "Select Type".to_owned(),
            format!("Choose JSON type for {}", path_display(&row.primary_path)),
            String::new(),
            false,
            choices
                .into_iter()
                .map(|choice| PluginConfigSelectionCandidate {
                    checked: value_matches_type(&current_value, choice.as_str()),
                    label: choice.clone(),
                    description: None,
                    value: PluginConfigSelectionValue::Named(choice),
                })
                .collect(),
            PluginConfigSelectionAction::Type {
                plugin_id: plugin.plugin_id.clone(),
                path: row.primary_path.clone(),
            },
        );
    }

    pub(crate) fn open_selected_config_value_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        self.open_row_cell_editor(
            dialog,
            context.plugin_id,
            context.row,
            context.layout,
            context.cell,
        );
    }

    pub(crate) fn open_drilldown_selected_row_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) {
        self.open_selected_config_value_editor(dialog);
    }

    pub(crate) fn open_row_cell_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
        layout: ConfigGroupLayout,
        cell: ConfigRowCell,
    ) {
        let cell = normalize_config_row_cell(&row, layout, cell);
        match cell {
            ConfigRowCell::Default => {
                if dialog.current_drilldown().is_some() {
                    self.delete_drilldown_selected_row(dialog);
                } else {
                    self.delete_selected_config_node(dialog);
                }
                return;
            }
            ConfigRowCell::State => {
                self.open_selected_config_actions(dialog);
                return;
            }
            ConfigRowCell::Action => {
                let Some(primary_action) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                    .and_then(|plugin| {
                        config_row_primary_action(
                            plugin,
                            &row.editor,
                            row.primary_path.as_slice(),
                            row.additional_paths.as_slice(),
                        )
                    })
                else {
                    self.open_selected_config_actions(dialog);
                    return;
                };
                match primary_action {
                    ConfigRowPrimaryAction::InsertAfter => {
                        self.insert_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                            true,
                        );
                    }
                    ConfigRowPrimaryAction::Duplicate => {
                        self.duplicate_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                        );
                    }
                    ConfigRowPrimaryAction::MoveDown => {
                        self.move_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                            1,
                        );
                    }
                    ConfigRowPrimaryAction::MoveUp => {
                        self.move_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                            -1,
                        );
                    }
                    ConfigRowPrimaryAction::Remove => {
                        self.remove_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                        );
                    }
                    ConfigRowPrimaryAction::AddField | ConfigRowPrimaryAction::AddItem => {
                        if let ConfigRowEditor::Structured { path } = row.editor.clone() {
                            self.open_add_config_value_editor_for_path(dialog, plugin_id, path);
                        }
                    }
                    ConfigRowPrimaryAction::Rename => {
                        self.open_rename_field_editor(dialog, plugin_id, row.primary_path.clone());
                    }
                }
                return;
            }
            _ => {}
        }
        if cell == ConfigRowCell::Type && row.type_mode.is_switchable() {
            self.open_type_selector_for_row(dialog, plugin_id, row);
            return;
        }
        if cell == ConfigRowCell::Value
            && let ConfigRowEditor::NullableString { path } = row.editor.clone()
        {
            self.open_nullable_string_value_editor(dialog, plugin_id, row, path);
            return;
        }
        if let ConfigRowEditor::PairInteger {
            left_path,
            right_path,
        } = &row.editor
        {
            let (left_label, right_label) =
                pair_editor_labels(left_path.as_slice(), right_path.as_slice());
            let (path, label) = if cell == ConfigRowCell::SecondaryValue {
                (right_path.clone(), right_label)
            } else {
                (left_path.clone(), left_label)
            };
            self.open_pair_integer_value_editor(dialog, plugin_id, row, path, label);
            return;
        }
        self.open_row_editor(dialog, plugin_id, row);
    }

    pub(crate) fn open_nullable_string_value_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
        path: ConfigPath,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let current = get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        dialog.editor = Some(EditorDialogState::new(
            format!("Edit {}", row.title),
            field_prompt_for_path(plugin, &path),
            editor_save_footer(&self.i18n, false),
            false,
            Editor::from_text(current),
            PluginConfigEditAction::SetNullableString {
                plugin_id: plugin.plugin_id.clone(),
                path,
            },
        ));
    }

    pub(crate) fn open_pair_integer_value_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
        path: ConfigPath,
        label: &str,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let current = get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_i64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "0".to_owned());
        dialog.editor = Some(EditorDialogState::new(
            format!("Edit {} · {}", row.title, label),
            field_prompt_for_path(plugin, &path),
            "Type to edit".to_owned(),
            false,
            Editor::from_text(current),
            PluginConfigEditAction::SetScalar {
                plugin_id: plugin.plugin_id.clone(),
                path,
                kind: ScalarEditKind::Integer,
            },
        ));
    }

    pub(crate) fn open_row_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        match row.editor.clone() {
            ConfigRowEditor::Bool { path } => {
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                self.set_config_value_at(dialog, plugin.plugin_id.clone(), path, json!(!current));
                return;
            }
            ConfigRowEditor::ReadOnly => {
                self.flash_warning(format!("{} is read-only", row.title));
                return;
            }
            ConfigRowEditor::NullableString { path } => {
                self.open_nullable_string_value_editor(dialog, plugin.plugin_id.clone(), row, path);
                return;
            }
            ConfigRowEditor::PairInteger {
                left_path,
                right_path,
            } => {
                let (left_label, right_label) =
                    pair_editor_labels(left_path.as_slice(), right_path.as_slice());
                let left = get_value_at_path(&plugin.draft_config, &left_path)
                    .and_then(JsonValue::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "0".to_owned());
                let right = get_value_at_path(&plugin.draft_config, &right_path)
                    .and_then(JsonValue::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "0".to_owned());
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", row.title),
                    format!(
                        "Enter the two values for {}.\nFirst line: {}\nSecond line: {}",
                        row.title, left_label, right_label
                    ),
                    editor_save_footer(&self.i18n, true),
                    true,
                    Editor::from_text(format!("{left}\n{right}")),
                    PluginConfigEditAction::SetPairIntegers {
                        plugin_id: plugin.plugin_id.clone(),
                        left_path,
                        right_path,
                    },
                ));
                return;
            }
            ConfigRowEditor::Structured { path } => {
                self.open_structured_row_drilldown(
                    dialog,
                    plugin.plugin_id.clone(),
                    path,
                    row.title.clone(),
                );
                return;
            }
            ConfigRowEditor::MultiEnum { path, variants } => {
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.open_multi_enum_selection_overlay(
                    dialog,
                    row.title.clone(),
                    plugin.plugin_id.clone(),
                    path,
                    variants,
                    current,
                );
                return;
            }
            _ => {}
        }
        let value =
            get_value_at_path(&plugin.draft_config, &row.primary_path).unwrap_or(&JsonValue::Null);
        let schema = plugin.schema.as_ref().and_then(|schema| {
            declared_schema_for_path(schema, schema, &plugin.draft_config, &row.primary_path)
        });
        if let Some(variants) = schema
            .as_ref()
            .and_then(schema_enum_values)
            .filter(|variants| !variants.is_empty())
        {
            self.open_enum_selection_overlay(
                dialog,
                row.title.clone(),
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                variants,
                value.clone(),
            );
            return;
        }
        if let Some(branches) = schema.as_ref().and_then(|schema| {
            plugin
                .schema
                .as_ref()
                .and_then(|root| branch_choices(root, schema))
        }) {
            self.open_branch_selection_overlay(
                dialog,
                "Select Branch".to_owned(),
                row.title.clone(),
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                branches,
                value.clone(),
            );
            return;
        }
        match value {
            JsonValue::Bool(current) => {
                self.set_config_value_at(
                    dialog,
                    plugin.plugin_id.clone(),
                    row.primary_path.clone(),
                    json!(!current),
                );
            }
            JsonValue::String(text) => {
                let multiline = schema.as_ref().is_some_and(schema_string_is_multiline);
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", row.title),
                    field_prompt_for_row(schema.as_ref(), &row),
                    editor_save_footer(&self.i18n, multiline),
                    multiline,
                    Editor::from_text(text.clone()),
                    PluginConfigEditAction::SetScalar {
                        plugin_id: plugin.plugin_id.clone(),
                        path: row.primary_path.clone(),
                        kind: ScalarEditKind::String,
                    },
                ));
            }
            JsonValue::Number(number) => {
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", row.title),
                    field_prompt_for_row(schema.as_ref(), &row),
                    "Type to edit".to_owned(),
                    false,
                    Editor::from_text(number.to_string()),
                    PluginConfigEditAction::SetScalar {
                        plugin_id: plugin.plugin_id.clone(),
                        path: row.primary_path.clone(),
                        kind: if number.as_i64().is_some() || number.as_u64().is_some() {
                            ScalarEditKind::Integer
                        } else {
                            ScalarEditKind::Number
                        },
                    },
                ));
            }
            JsonValue::Null => {
                self.open_type_selector_for_row(dialog, plugin.plugin_id.clone(), row)
            }
            JsonValue::Object(_) | JsonValue::Array(_) => {
                self.open_structured_row_drilldown(
                    dialog,
                    plugin.plugin_id.clone(),
                    row.primary_path.clone(),
                    row.title.clone(),
                );
            }
        }
    }

    pub(crate) fn set_config_value_at(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
        value: JsonValue,
    ) -> bool {
        match self.try_set_config_value_at(dialog, plugin_id, path, value) {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                false
            }
        }
    }

    pub(crate) fn try_set_config_values_at(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        updates: Vec<(ConfigPath, JsonValue)>,
        focus_path: ConfigPath,
    ) -> UiResult<()> {
        let Some(plugin_index) = dialog
            .plugins
            .iter()
            .position(|plugin| plugin.plugin_id == plugin_id)
        else {
            return Ok(());
        };
        let next_config = {
            let plugin = &dialog.plugins[plugin_index];
            apply_staged_config_value_updates(
                plugin.schema.as_ref(),
                &plugin.draft_config,
                updates.as_slice(),
            )?
        };
        {
            let plugin = &mut dialog.plugins[plugin_index];
            plugin.draft_config = next_config;
            recompute_plugin_config_state(plugin);
        }
        self.focus_config_path(dialog, plugin_id.as_str(), focus_path.as_slice());
        dialog.clamp_selection();
        Ok(())
    }

    pub(crate) fn try_set_config_value_at(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
        value: JsonValue,
    ) -> UiResult<()> {
        self.try_set_config_values_at(dialog, plugin_id, vec![(path.clone(), value)], path)
    }

    pub(crate) fn open_named_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        prompt: String,
        footer: String,
        multi: bool,
        items: Vec<PluginConfigSelectionCandidate>,
        action: PluginConfigSelectionAction,
    ) {
        let rows = items
            .iter()
            .enumerate()
            .map(
                |(index, item)| agena_tui_plugin_workbench::PluginConfigPickerItem {
                    key: format!("plugin-config-selection:{index:08}"),
                    label: item.label.clone(),
                    detail: item.description.clone(),
                    initially_selected: item.checked,
                },
            )
            .collect::<Vec<_>>();
        let values = items
            .into_iter()
            .enumerate()
            .map(|(index, item)| (format!("plugin-config-selection:{index:08}"), item.value))
            .collect();
        let presentation = agena_tui_plugin_workbench::new_plugin_config_picker(
            title,
            prompt,
            footer,
            "No choices available".to_owned(),
            multi,
            rows,
        );
        dialog.selection = Some(PluginConfigSelectionOverlay {
            presentation,
            action,
            values,
        });
        dialog.clamp_selection();
    }

    pub(crate) fn open_branch_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        prompt: String,
        plugin_id: String,
        path: ConfigPath,
        branches: Vec<BranchChoice>,
        current: JsonValue,
    ) {
        let active = active_branch_id(branches.as_slice(), &current).to_owned();
        let items = branches
            .into_iter()
            .map(|branch| PluginConfigSelectionCandidate {
                label: branch.label.clone(),
                description: Some(schema_kind_label(&branch.schema)),
                checked: branch.id == active,
                value: PluginConfigSelectionValue::Branch(branch),
            })
            .collect::<Vec<_>>();
        self.open_named_selection_overlay(
            dialog,
            title,
            prompt,
            String::new(),
            false,
            items,
            PluginConfigSelectionAction::Branch { plugin_id, path },
        );
    }

    pub(crate) fn open_enum_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        plugin_id: String,
        path: ConfigPath,
        variants: Vec<JsonValue>,
        current: JsonValue,
    ) {
        let items = variants
            .into_iter()
            .map(|variant| {
                let checked = variant == current;
                PluginConfigSelectionCandidate {
                    label: preview_value(&variant),
                    description: None,
                    checked,
                    value: PluginConfigSelectionValue::Json(variant),
                }
            })
            .collect::<Vec<_>>();
        self.open_named_selection_overlay(
            dialog,
            format!("Select {title}"),
            "Choose one value".to_owned(),
            String::new(),
            false,
            items,
            PluginConfigSelectionAction::Enum { plugin_id, path },
        );
    }

    pub(crate) fn open_multi_enum_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        plugin_id: String,
        path: ConfigPath,
        variants: Vec<JsonValue>,
        current: Vec<JsonValue>,
    ) {
        let items = variants
            .into_iter()
            .map(|variant| PluginConfigSelectionCandidate {
                label: preview_value(&variant),
                description: None,
                checked: current.iter().any(|item| item == &variant),
                value: PluginConfigSelectionValue::Json(variant),
            })
            .collect::<Vec<_>>();
        self.open_named_selection_overlay(
            dialog,
            format!("Select {title}"),
            "Choose one or more values".to_owned(),
            "Space toggle".to_owned(),
            true,
            items,
            PluginConfigSelectionAction::MultiEnum { plugin_id, path },
        );
    }

    pub(crate) fn open_structured_row_drilldown(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
        title: String,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let groups = build_drilldown_groups(plugin, &path, title.as_str());
        dialog.drilldown_stack.push(PluginConfigDrilldownOverlay {
            plugin_id,
            path,
            title,
            groups,
            selected_row: 0,
            selected_cell: ConfigRowCell::Value,
        });
    }

    pub(crate) fn delete_drilldown_selected_row(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(overlay) = dialog.current_drilldown() else {
            return;
        };
        let Some(row) =
            drilldown_row_at(overlay, dialog.config_view, overlay.selected_row).cloned()
        else {
            return;
        };
        if row.primary_path.is_empty() {
            self.flash_warning("root config cannot be deleted".to_owned());
            return;
        }
        let plugin_id = overlay.plugin_id.clone();
        let (changed, blocked) = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
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
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
        if let Some(message) = reset_paths_warning_message(blocked.as_slice()) {
            self.flash_warning(message);
        }
    }
}
use super::{
    App, BranchChoice, ConfigGroupLayout, ConfigPath, ConfigRowCell, ConfigRowEditor,
    ConfigRowPrimaryAction, ConfigRowView, Editor, EditorDialogState, JsonValue, PathSegment,
    PluginConfigDrilldownOverlay, PluginConfigEditAction, PluginConfigSelectionAction,
    PluginConfigSelectionCandidate, PluginConfigSelectionOverlay, PluginConfigSelectionValue,
    PluginWorkbenchOverlay, ScalarEditKind, UiResult, active_branch_id,
    append_default_array_item_at_path, apply_reset_paths, apply_staged_config_value_updates,
    branch_choices, build_drilldown_groups, can_append_array_item,
    clear_branch_drafts_for_structural_change, config_row_primary_action, declared_schema_for_path,
    default_value_for_schema, drilldown_group_for_row, drilldown_row_at, editor_save_footer,
    field_prompt_for_path, field_prompt_for_row, get_value_at_path, json, move_config_row_cell,
    normalize_config_row_cell, object_add_field_block_reason, pair_editor_labels, path_display,
    preview_value, rebuild_drilldown_stack, recompute_plugin_config_state,
    reset_paths_warning_message, row_paths, schema_enum_values, schema_kind_label,
    schema_string_is_multiline, schema_type_selector_choices, section_group_for_row,
    section_row_at, selected_config_row_context, value_matches_type,
};
