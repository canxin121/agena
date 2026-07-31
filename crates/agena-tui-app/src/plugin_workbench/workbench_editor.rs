impl App {
    pub(crate) fn commit_plugin_config_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        action: PluginConfigEditAction,
        input: &str,
    ) -> UiResult<()> {
        match action {
            PluginConfigEditAction::SetScalar {
                plugin_id,
                path,
                kind,
            } => {
                let value =
                    parse_scalar_editor_value(kind, input).map_err(crate::UiFailure::message)?;
                self.try_set_config_value_at(dialog, plugin_id, path, value)?;
            }
            PluginConfigEditAction::SetNullableString { plugin_id, path } => {
                let trimmed = input.trim();
                let value = if trimmed.is_empty() {
                    JsonValue::Null
                } else {
                    JsonValue::String(trimmed.to_owned())
                };
                self.try_set_config_value_at(dialog, plugin_id, path, value)?;
            }
            PluginConfigEditAction::SetPairIntegers {
                plugin_id,
                left_path,
                right_path,
            } => {
                let (left_value, right_value) =
                    parse_pair_integer_editor_values(input).map_err(crate::UiFailure::message)?;
                let selection_plugin_id = plugin_id.clone();
                self.try_set_config_values_at(
                    dialog,
                    plugin_id,
                    vec![
                        (
                            left_path.clone(),
                            JsonValue::Number(JsonNumber::from(left_value)),
                        ),
                        (right_path, JsonValue::Number(JsonNumber::from(right_value))),
                    ],
                    left_path.clone(),
                )?;
                self.focus_config_path(dialog, selection_plugin_id.as_str(), left_path.as_slice());
            }
            PluginConfigEditAction::AddObjectField { plugin_id, path } => {
                let key = input.trim();
                if key.is_empty() {
                    return Err(crate::UiFailure::message("field name cannot be empty"));
                }
                let Some(plugin_index) = dialog
                    .plugins
                    .iter()
                    .position(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(key.to_owned()));
                {
                    let plugin = &mut dialog.plugins[plugin_index];
                    if get_value_at_path(&plugin.draft_config, &path)
                        .and_then(JsonValue::as_object)
                        .is_some_and(|object| object.contains_key(key))
                    {
                        return Err(crate::UiFailure::message(format!(
                            "field `{key}` already exists"
                        )));
                    }
                    if let Some(reason) = object_add_field_block_reason(
                        plugin.schema.as_ref(),
                        &plugin.draft_config,
                        &path,
                    ) {
                        return Err(crate::UiFailure::message(reason));
                    }
                    let child_schema = validate_new_object_field_key(
                        plugin.schema.as_ref(),
                        &plugin.draft_config,
                        &path,
                        key,
                    )
                    .map_err(crate::UiFailure::message)?;
                    let default = child_schema
                        .as_ref()
                        .map(|schema| {
                            default_value_for_schema(
                                schema,
                                plugin.schema.as_ref().unwrap_or(schema),
                            )
                        })
                        .unwrap_or(JsonValue::Null);
                    set_value_at_path(&mut plugin.draft_config, &child_path, default);
                    clear_branch_drafts_for_structural_change(plugin);
                    recompute_plugin_config_state(plugin);
                }
                self.focus_config_path(dialog, plugin_id.as_str(), child_path.as_slice());
                self.maybe_open_type_selector_for_selected_row(
                    dialog,
                    plugin_id.as_str(),
                    child_path.as_slice(),
                );
            }
            PluginConfigEditAction::RenameObjectField { plugin_id, path } => {
                let new_key = input.trim();
                if new_key.is_empty() {
                    return Err(crate::UiFailure::message("field name cannot be empty"));
                }
                let Some(plugin_index) = dialog
                    .plugins
                    .iter()
                    .position(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let Some((parent_path, current_key)) = path_key_info(path.as_slice()) else {
                    return Err(crate::UiFailure::message(
                        "selected row does not point to an object field",
                    ));
                };
                if new_key == current_key {
                    return Ok(());
                }
                let parent_path = parent_path.to_vec();
                let new_path = {
                    let plugin = &mut dialog.plugins[plugin_index];
                    if get_value_at_path(&plugin.draft_config, &parent_path)
                        .and_then(JsonValue::as_object)
                        .is_some_and(|object| object.contains_key(new_key))
                    {
                        return Err(crate::UiFailure::message(format!(
                            "field `{new_key}` already exists"
                        )));
                    }
                    let child_schema = validate_new_object_field_key(
                        plugin.schema.as_ref(),
                        &plugin.draft_config,
                        &parent_path,
                        new_key,
                    )
                    .map_err(crate::UiFailure::message)?;
                    let current_value = get_value_at_path(&plugin.draft_config, &path)
                        .cloned()
                        .unwrap_or(JsonValue::Null);
                    let mut preview_path = parent_path.clone();
                    preview_path.push(PathSegment::Key(new_key.to_owned()));
                    if let Some(schema) = child_schema.as_ref()
                        && let Some(root_schema) = plugin.schema.as_ref()
                    {
                        validate_schema_value_for_path(
                            root_schema,
                            schema,
                            &current_value,
                            &preview_path,
                            title_for_schema_or_key(schema, new_key).as_str(),
                        )
                        .map_err(|error| {
                            crate::UiFailure::invalid_with_diagnostic(
                                "The plugin configuration value does not match its schema.",
                                error,
                            )
                        })?;
                    }
                    let Some(new_path) = rename_object_field_at_path(
                        &mut plugin.draft_config,
                        path.as_slice(),
                        new_key,
                    ) else {
                        return Err(crate::UiFailure::message("failed to rename field"));
                    };
                    clear_branch_drafts_for_structural_change(plugin);
                    recompute_plugin_config_state(plugin);
                    new_path
                };
                self.focus_config_path(dialog, plugin_id.as_str(), new_path.as_slice());
            }
        }
        dialog.clamp_selection();
        Ok(())
    }

    pub(crate) fn commit_plugin_config_selection(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        key: String,
    ) -> UiResult<()> {
        let Some(overlay) = dialog.selection.clone() else {
            return Ok(());
        };
        let selected_value = overlay
            .values
            .get(key.as_str())
            .cloned()
            .ok_or_else(|| crate::UiFailure::message("no selection available"))?;
        match overlay.action {
            PluginConfigSelectionAction::Type { plugin_id, path } => {
                let PluginConfigSelectionValue::Named(selected) = selected_value else {
                    return Err(crate::UiFailure::message("invalid type selection"));
                };
                let Some(plugin) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let schema = plugin
                    .schema
                    .as_ref()
                    .and_then(|root| schema_for_path(root, root, &plugin.draft_config, &path));
                let value = default_value_for_type(selected.as_str(), schema.as_ref());
                self.try_set_config_value_at(dialog, plugin_id, path, value)?;
            }
            PluginConfigSelectionAction::Branch { plugin_id, path } => {
                let PluginConfigSelectionValue::Branch(branch) = selected_value else {
                    return Err(crate::UiFailure::message("invalid branch selection"));
                };
                let Some(plugin) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let all_branches = overlay
                    .values
                    .values()
                    .filter_map(|value| match value {
                        PluginConfigSelectionValue::Branch(branch) => Some(branch.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .cloned()
                    .unwrap_or(JsonValue::Null);
                let active_key = plugin_branch_draft_key(
                    plugin.plugin_id.as_str(),
                    &path,
                    active_branch_id(all_branches.as_slice(), &current),
                );
                let target_key =
                    plugin_branch_draft_key(plugin.plugin_id.as_str(), &path, branch.id.as_str());
                let value = plugin
                    .branch_drafts
                    .get(target_key.as_str())
                    .cloned()
                    .unwrap_or_else(|| {
                        default_value_for_schema(
                            &branch.schema,
                            plugin.schema.as_ref().unwrap_or(&branch.schema),
                        )
                    });
                self.try_set_config_value_at(dialog, plugin_id.clone(), path.clone(), value)?;
                if let Some(plugin) = dialog
                    .plugins
                    .iter_mut()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                {
                    plugin.branch_drafts.insert(active_key, current);
                }
            }
            PluginConfigSelectionAction::Enum { plugin_id, path } => {
                let PluginConfigSelectionValue::Json(selected) = selected_value else {
                    return Err(crate::UiFailure::message("invalid enum selection"));
                };
                self.try_set_config_value_at(dialog, plugin_id, path, selected)?;
            }
            PluginConfigSelectionAction::MultiEnum { plugin_id, path } => {
                let selected_values = overlay
                    .presentation
                    .checked_keys
                    .iter()
                    .filter_map(|key| match overlay.values.get(key) {
                        Some(PluginConfigSelectionValue::Json(value)) => Some(value.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let Some(plugin) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default();
                let values =
                    merge_multi_enum_selection(current.as_slice(), selected_values.as_slice());
                self.try_set_config_value_at(dialog, plugin_id, path, JsonValue::Array(values))?;
            }
        }
        dialog.clamp_selection();
        Ok(())
    }
}
use super::{
    App, JsonNumber, JsonValue, PathSegment, PluginConfigEditAction, PluginConfigSelectionAction,
    PluginConfigSelectionValue, PluginWorkbenchOverlay, UiResult, active_branch_id,
    clear_branch_drafts_for_structural_change, default_value_for_schema, default_value_for_type,
    get_value_at_path, merge_multi_enum_selection, object_add_field_block_reason,
    parse_pair_integer_editor_values, parse_scalar_editor_value, path_key_info,
    plugin_branch_draft_key, recompute_plugin_config_state, rename_object_field_at_path,
    schema_for_path, set_value_at_path, title_for_schema_or_key, validate_new_object_field_key,
    validate_schema_value_for_path,
};
