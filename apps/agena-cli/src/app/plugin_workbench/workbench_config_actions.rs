pub(in crate::app) fn action_matches_primary(
    action: &PluginConfigAction,
    primary: ConfigRowPrimaryAction,
) -> bool {
    matches!(
        (action, primary),
        (
            PluginConfigAction::AppendArrayItem { .. },
            ConfigRowPrimaryAction::AddItem
        ) | (
            PluginConfigAction::PromptAddObjectField { .. },
            ConfigRowPrimaryAction::AddField
        ) | (
            PluginConfigAction::InsertArrayItemAfter { .. },
            ConfigRowPrimaryAction::InsertAfter
        ) | (
            PluginConfigAction::DuplicateArrayItem { .. },
            ConfigRowPrimaryAction::Duplicate
        ) | (
            PluginConfigAction::MoveArrayItem { direction: 1, .. },
            ConfigRowPrimaryAction::MoveDown
        ) | (
            PluginConfigAction::MoveArrayItem { direction: -1, .. },
            ConfigRowPrimaryAction::MoveUp
        ) | (
            PluginConfigAction::RemoveArrayItem { .. },
            ConfigRowPrimaryAction::Remove
        ) | (
            PluginConfigAction::RenameField { .. },
            ConfigRowPrimaryAction::Rename
        )
    )
}

pub(in crate::app) fn action_is_select_type(action: &PluginConfigAction) -> bool {
    matches!(action, PluginConfigAction::SelectType { .. })
}

pub(in crate::app) fn action_is_reset_field(action: &PluginConfigAction) -> bool {
    matches!(action, PluginConfigAction::ResetField { .. })
}

pub(in crate::app) fn action_priority_for_focus(
    action: &PluginConfigAction,
    focused_cell: ConfigRowCell,
    primary_action: Option<ConfigRowPrimaryAction>,
) -> (u8, u8) {
    let base = match action {
        PluginConfigAction::SelectType { .. } => 0,
        PluginConfigAction::AppendArrayItem { .. } => 1,
        PluginConfigAction::PromptAddObjectField { .. } => 2,
        PluginConfigAction::InsertArrayItemBefore { .. } => 3,
        PluginConfigAction::InsertArrayItemAfter { .. } => 4,
        PluginConfigAction::DuplicateArrayItem { .. } => 5,
        PluginConfigAction::MoveArrayItem { direction: -1, .. } => 6,
        PluginConfigAction::MoveArrayItem { direction: 1, .. } => 7,
        PluginConfigAction::RemoveArrayItem { .. } => 8,
        PluginConfigAction::RenameField { .. } => 9,
        PluginConfigAction::ResetField { .. } => 10,
        PluginConfigAction::ResetGroup { .. } => 11,
        PluginConfigAction::MoveArrayItem { .. } => 12,
    };
    let rank = if focused_cell == ConfigRowCell::Type && action_is_select_type(action) {
        0
    } else if focused_cell == ConfigRowCell::Default && action_is_reset_field(action) {
        0
    } else if primary_action.is_some_and(|primary| action_matches_primary(action, primary)) {
        if focused_cell == ConfigRowCell::Action || focused_cell == ConfigRowCell::State {
            0
        } else {
            1
        }
    } else if action_is_select_type(action) {
        2
    } else if matches!(
        action,
        PluginConfigAction::AppendArrayItem { .. }
            | PluginConfigAction::PromptAddObjectField { .. }
            | PluginConfigAction::InsertArrayItemBefore { .. }
            | PluginConfigAction::InsertArrayItemAfter { .. }
            | PluginConfigAction::DuplicateArrayItem { .. }
            | PluginConfigAction::MoveArrayItem { .. }
    ) {
        3
    } else if matches!(action, PluginConfigAction::RenameField { .. }) {
        4
    } else if action_is_reset_field(action) {
        5
    } else {
        6
    };
    (rank, base)
}

pub(in crate::app) fn prioritize_config_actions(
    actions: &mut [PluginConfigActionItem],
    focused_cell: ConfigRowCell,
    primary_action: Option<ConfigRowPrimaryAction>,
) -> usize {
    actions
        .sort_by_key(|item| action_priority_for_focus(&item.action, focused_cell, primary_action));
    0
}

pub(in crate::app) fn config_actions_overlay_footer(
    primary_action: Option<ConfigRowPrimaryAction>,
) -> String {
    primary_action
        .map(|action| format!("Primary: {}", action.plain_label()))
        .unwrap_or_default()
}

#[derive(Debug, Default)]
pub(in crate::app) struct ResetPathsOutcome {
    pub(super) changed: bool,
    pub(super) blocked: Vec<String>,
}

pub(in crate::app) fn schema_unique_items(schema: &JsonValue) -> bool {
    schema_bool_keyword_any(schema, "uniqueItems")
}

pub(in crate::app) fn clear_branch_drafts_for_structural_change(
    plugin: &mut PluginWorkbenchPlugin,
) {
    plugin.branch_drafts.clear();
}

pub(in crate::app) fn schema_allows_dynamic_object_keys(schema: &JsonValue) -> bool {
    let direct = match schema.get("additionalProperties") {
        Some(JsonValue::Bool(false)) => schema
            .get("patternProperties")
            .and_then(JsonValue::as_object)
            .is_some_and(|patterns| !patterns.is_empty()),
        _ => true,
    };
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        direct && all_of.iter().all(schema_allows_dynamic_object_keys)
    } else {
        direct
    }
}

pub(in crate::app) fn object_add_field_block_reason(
    root_schema: Option<&JsonValue>,
    config: &JsonValue,
    object_path: &ConfigPath,
) -> Option<String> {
    let object = get_value_at_path(config, object_path)?.as_object()?;
    let Some(root_schema) = root_schema else {
        return None;
    };
    let Some(parent_schema) = schema_for_path(root_schema, root_schema, config, object_path) else {
        return None;
    };
    let max_properties =
        schema_max_u64_constraint(&parent_schema, "maxProperties").map(|value| value as usize);
    max_properties
        .filter(|max_properties| object.len() >= *max_properties)
        .map(|max_properties| {
            format!("object already has the maximum of {max_properties} field(s)")
        })
        .or_else(|| {
            (!schema_allows_dynamic_object_keys(&parent_schema))
                .then_some("schema does not allow adding custom field names".to_owned())
        })
}

pub(in crate::app) fn write_path_block_reason(
    root_schema: Option<&JsonValue>,
    config: &JsonValue,
    path: &[PathSegment],
) -> Option<String> {
    let Some(root_schema) = root_schema else {
        return None;
    };
    let mut current_value = config;
    let mut prefix = Vec::new();
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                let object = current_value.as_object();
                let present = object.is_some_and(|object| object.contains_key(key));
                if !present {
                    let object_len = object.map(|object| object.len()).unwrap_or_default();
                    if let Some(parent_schema) =
                        schema_for_path(root_schema, root_schema, config, &prefix)
                        && let Some(max_properties) =
                            schema_max_u64_constraint(&parent_schema, "maxProperties")
                                .map(|value| value as usize)
                        && object_len >= max_properties
                    {
                        return Some(if prefix.is_empty() {
                            format!("object already has the maximum of {max_properties} field(s)")
                        } else {
                            format!(
                                "{} already has the maximum of {max_properties} field(s)",
                                path_display(&prefix)
                            )
                        });
                    }
                }
                current_value = object
                    .and_then(|object| object.get(key))
                    .unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_value = current_value
                    .as_array()
                    .and_then(|items| items.get(*index))
                    .unwrap_or(&JsonValue::Null);
            }
        }
        prefix.push(segment.clone());
    }
    None
}

pub(in crate::app) fn apply_staged_config_value_updates(
    root_schema: Option<&JsonValue>,
    current_config: &JsonValue,
    updates: &[(ConfigPath, JsonValue)],
) -> UiResult<JsonValue> {
    let mut next_config = current_config.clone();
    for (path, value) in updates {
        if let Some(reason) = write_path_block_reason(root_schema, &next_config, path.as_slice()) {
            return Err(reason);
        }
        set_value_at_path(&mut next_config, path, value.clone());
    }
    Ok(next_config)
}

pub(in crate::app) fn validate_container_candidate(
    root_schema: &JsonValue,
    current_root: &JsonValue,
    container_path: &ConfigPath,
    candidate_container: JsonValue,
) -> Option<String> {
    let mut candidate_root = current_root.clone();
    set_value_at_path(&mut candidate_root, container_path, candidate_container);
    let candidate_schema =
        schema_for_path(root_schema, root_schema, &candidate_root, container_path)
            .or_else(|| schema_for_path(root_schema, root_schema, current_root, container_path))?;
    let candidate_value = get_value_at_path(&candidate_root, container_path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let title = title_from_path(container_path.as_slice());
    validate_schema_value_for_path(
        root_schema,
        &candidate_schema,
        &candidate_value,
        container_path,
        title.as_str(),
    )
    .err()
}

pub(in crate::app) fn validate_reset_candidate(
    root_schema: Option<&JsonValue>,
    current_root: &JsonValue,
    path: &ConfigPath,
) -> Option<String> {
    let Some(root_schema) = root_schema else {
        return None;
    };
    let (_, parent_path) = path.split_last()?;
    let container_path = parent_path.to_vec();
    let mut candidate_root = current_root.clone();
    remove_value_at_path(&mut candidate_root, path)?;
    let candidate_container = get_value_at_path(&candidate_root, &container_path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    validate_container_candidate(
        root_schema,
        current_root,
        &container_path,
        candidate_container,
    )
}

pub(in crate::app) fn prepared_default_array_item_value(
    current_root: &JsonValue,
    root_schema: Option<&JsonValue>,
    parent_path: &ConfigPath,
    insert_index: usize,
) -> Option<JsonValue> {
    let array = get_value_at_path(current_root, parent_path)?.as_array()?;
    if insert_index > array.len() {
        return None;
    }
    let value = if let Some(root_schema) = root_schema {
        let Some(parent_schema) =
            schema_for_path(root_schema, root_schema, current_root, parent_path)
        else {
            return Some(JsonValue::Null);
        };
        let max_items =
            schema_max_u64_constraint(&parent_schema, "maxItems").map(|value| value as usize);
        if max_items.is_some_and(|max_items| array.len() >= max_items) {
            return None;
        }
        let item_schema = array_item_schema(root_schema, &parent_schema, insert_index)?;
        if matches!(item_schema, JsonValue::Bool(false)) {
            return None;
        }
        let value = default_value_for_schema(&item_schema, root_schema);
        let mut candidate_items = array.clone();
        candidate_items.insert(insert_index, value.clone());
        if validate_container_candidate(
            root_schema,
            current_root,
            parent_path,
            JsonValue::Array(candidate_items),
        )
        .is_some()
        {
            return None;
        }
        value
    } else {
        JsonValue::Null
    };
    Some(value)
}

pub(in crate::app) fn can_duplicate_array_item(
    root_schema: Option<&JsonValue>,
    current_root: &JsonValue,
    path: &[PathSegment],
) -> bool {
    let Some((parent_path, index, len)) = array_item_path_info(current_root, path) else {
        return false;
    };
    let Some(array) = get_value_at_path(current_root, &parent_path).and_then(JsonValue::as_array)
    else {
        return false;
    };
    if index >= len {
        return false;
    }
    if let Some(root_schema) = root_schema {
        let mut candidate_items = array.clone();
        let Some(clone) = candidate_items.get(index).cloned() else {
            return false;
        };
        candidate_items.insert(index + 1, clone);
        validate_container_candidate(
            root_schema,
            current_root,
            &parent_path,
            JsonValue::Array(candidate_items),
        )
        .is_none()
    } else {
        true
    }
}

pub(in crate::app) fn can_remove_array_item(
    root_schema: Option<&JsonValue>,
    current_root: &JsonValue,
    path: &[PathSegment],
) -> bool {
    let Some((parent_path, index, len)) = array_item_path_info(current_root, path) else {
        return false;
    };
    let Some(array) = get_value_at_path(current_root, &parent_path).and_then(JsonValue::as_array)
    else {
        return false;
    };
    if index >= len {
        return false;
    }
    if let Some(root_schema) = root_schema {
        let mut candidate_items = array.clone();
        candidate_items.remove(index);
        validate_container_candidate(
            root_schema,
            current_root,
            &parent_path,
            JsonValue::Array(candidate_items),
        )
        .is_none()
    } else {
        true
    }
}

pub(in crate::app) fn validate_schema_value_for_path(
    root_schema: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
    title: &str,
) -> UiResult<()> {
    let mut diagnostics = Vec::new();
    validate_schema_at(&mut diagnostics, root_schema, schema, value, path, title);
    if let Some(error) = diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Err(error.message);
    }
    Ok(())
}

pub(in crate::app) fn reset_path_block_reason(
    root_schema: Option<&JsonValue>,
    default_root: &JsonValue,
    current_root: &JsonValue,
    path: &ConfigPath,
) -> Option<String> {
    if get_value_at_path(default_root, path).is_some() {
        return None;
    }
    if get_value_at_path(current_root, path).is_none() {
        return None;
    }
    if let Some((parent_path, key)) = path_key_info(path.as_slice()) {
        let object = get_value_at_path(current_root, &parent_path)?.as_object()?;
        let Some(root_schema) = root_schema else {
            return None;
        };
        let Some(parent_schema) =
            schema_for_path(root_schema, root_schema, current_root, &parent_path)
        else {
            return None;
        };
        if schema_required_fields(&parent_schema).contains(key.as_str()) {
            return Some("field is required and has no default".to_owned());
        }
        let min_properties =
            schema_min_u64_constraint(&parent_schema, "minProperties").unwrap_or(0) as usize;
        if object.len() <= min_properties {
            return Some(format!(
                "object requires at least {min_properties} field(s)"
            ));
        }
        return validate_reset_candidate(Some(root_schema), current_root, path);
    }
    let (parent_path, index, len) = array_item_path_info(current_root, path.as_slice())?;
    let Some(root_schema) = root_schema else {
        return None;
    };
    let Some(parent_schema) = schema_for_path(root_schema, root_schema, current_root, &parent_path)
    else {
        return None;
    };
    let tuple_prefix_len = schema_prefix_item_count(&parent_schema);
    if index < tuple_prefix_len {
        return Some("tuple item has no removable default slot".to_owned());
    }
    let min_items = schema_min_u64_constraint(&parent_schema, "minItems").unwrap_or(0) as usize;
    if len <= min_items {
        return Some(format!("array requires at least {min_items} item(s)"));
    }
    validate_reset_candidate(Some(root_schema), current_root, path)
}

pub(in crate::app) fn compare_reset_paths(
    left: &ConfigPath,
    right: &ConfigPath,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    for (left_segment, right_segment) in left.iter().zip(right.iter()) {
        match (left_segment, right_segment) {
            (PathSegment::Index(left_index), PathSegment::Index(right_index))
                if left_index != right_index =>
            {
                return right_index.cmp(left_index);
            }
            (PathSegment::Key(left_key), PathSegment::Key(right_key)) if left_key != right_key => {
                return left_key.cmp(right_key);
            }
            (PathSegment::Key(_), PathSegment::Index(_)) => return Ordering::Less,
            (PathSegment::Index(_), PathSegment::Key(_)) => return Ordering::Greater,
            _ => {}
        }
    }
    right.len().cmp(&left.len())
}

pub(in crate::app) fn normalized_reset_paths(paths: &[ConfigPath]) -> Vec<ConfigPath> {
    let mut paths = paths.to_vec();
    paths.sort_by(compare_reset_paths);
    paths.dedup();
    paths
}

pub(in crate::app) fn apply_reset_paths(
    value: &mut JsonValue,
    default_root: &JsonValue,
    root_schema: Option<&JsonValue>,
    paths: &[ConfigPath],
) -> ResetPathsOutcome {
    let mut changed = false;
    let mut blocked = Vec::new();
    for path in normalized_reset_paths(paths) {
        if let Some(reason) = reset_path_block_reason(root_schema, default_root, value, &path) {
            blocked.push(format!("{}: {reason}", path_display(&path)));
            continue;
        }
        changed |= reset_effective_value_at_path(value, default_root, path.as_slice());
    }
    ResetPathsOutcome { changed, blocked }
}

pub(in crate::app) fn reset_paths_warning_message(blocked: &[String]) -> Option<String> {
    let first = blocked.first()?;
    Some(if blocked.len() == 1 {
        format!("cannot reset {first}")
    } else {
        format!("some fields could not be reset (showing first): {first}")
    })
}

pub(in crate::app) fn generic_array_item_action_info(
    index: usize,
    len: usize,
) -> ArrayItemActionInfo {
    ArrayItemActionInfo {
        can_insert_before: true,
        can_insert_after: true,
        can_duplicate: true,
        can_move_up: index > 0,
        can_move_down: index + 1 < len,
        can_remove: len > 0,
    }
}

pub(in crate::app) fn duplicate_array_item_at_path(
    root: &mut JsonValue,
    path: &[PathSegment],
) -> Option<ConfigPath> {
    let (parent_path, index, len) = array_item_path_info(root, path)?;
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    if index >= len {
        return None;
    }
    let clone = array.get(index)?.clone();
    let next_index = index + 1;
    array.insert(next_index, clone);
    Some(replace_last_index(path, next_index))
}

pub(in crate::app) fn move_array_item_at_path(
    root: &mut JsonValue,
    path: &[PathSegment],
    direction: isize,
) -> Option<ConfigPath> {
    let (parent_path, index, len) = array_item_path_info(root, path)?;
    let target_index =
        (index as isize + direction).clamp(0, len.saturating_sub(1) as isize) as usize;
    if target_index == index {
        return Some(path.to_vec());
    }
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    array.swap(index, target_index);
    Some(replace_last_index(path, target_index))
}

pub(in crate::app) fn remove_array_item_at_path(
    root: &mut JsonValue,
    path: &[PathSegment],
) -> Option<ConfigPath> {
    let (parent_path, index, len) = array_item_path_info(root, path)?;
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    if index >= len {
        return None;
    }
    array.remove(index);
    if array.is_empty() {
        Some(parent_path)
    } else if index >= array.len() {
        Some(replace_last_index(path, array.len().saturating_sub(1)))
    } else {
        Some(replace_last_index(path, index))
    }
}

pub(in crate::app) fn rename_object_field_at_path(
    root: &mut JsonValue,
    path: &[PathSegment],
    new_key: &str,
) -> Option<ConfigPath> {
    let (parent_path, current_key) = path_key_info(path)?;
    let object = get_value_mut_at_path(root, &parent_path)?.as_object_mut()?;
    let value = object.remove(current_key.as_str())?;
    object.insert(new_key.to_owned(), value);
    let mut next_path = parent_path;
    next_path.push(PathSegment::Key(new_key.to_owned()));
    Some(next_path)
}

pub(in crate::app) fn validate_new_object_field_key(
    root_schema: Option<&JsonValue>,
    config: &JsonValue,
    object_path: &ConfigPath,
    key: &str,
) -> UiResult<Option<JsonValue>> {
    let Some(root_schema) = root_schema else {
        return Ok(None);
    };
    let Some(parent_schema) = schema_for_path(root_schema, root_schema, config, object_path) else {
        return Ok(None);
    };
    for property_names_schema in schema_property_name_schemas(&parent_schema) {
        let mut diagnostics = Vec::new();
        let mut key_path = object_path.clone();
        key_path.push(PathSegment::Key(key.to_owned()));
        validate_schema_at(
            &mut diagnostics,
            root_schema,
            &property_names_schema,
            &JsonValue::String(key.to_owned()),
            &key_path,
            format!("{key} name").as_str(),
        );
        if let Some(error) = diagnostics
            .into_iter()
            .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            return Err(error.message);
        }
    }
    let child_schema = object_property_schema(root_schema, &parent_schema, key);
    if matches!(child_schema, Some(JsonValue::Bool(false)))
        || (child_schema.is_none() && schema_prohibits_additional_properties(&parent_schema))
    {
        return Err(format!("field `{key}` is not allowed by this schema"));
    }
    Ok(child_schema)
}

pub(in crate::app) fn array_item_action_info(
    plugin: &PluginWorkbenchPlugin,
    path: &[PathSegment],
) -> Option<ArrayItemActionInfo> {
    let (parent_path, index, len) = array_item_path_info(&plugin.draft_config, path)?;
    let Some(root_schema) = plugin.schema.as_ref() else {
        return Some(generic_array_item_action_info(index, len));
    };
    let Some(parent_schema) =
        schema_for_path(root_schema, root_schema, &plugin.draft_config, &parent_path)
    else {
        return Some(generic_array_item_action_info(index, len));
    };
    let tuple_prefix_len = schema_prefix_item_count(&parent_schema);
    let tuple_slot = index < tuple_prefix_len;
    let min_items = schema_min_u64_constraint(&parent_schema, "minItems").unwrap_or(0) as usize;
    let can_insert_before = !tuple_slot
        && prepared_default_array_item_value(
            &plugin.draft_config,
            plugin.schema.as_ref(),
            &parent_path,
            index,
        )
        .is_some();
    let can_insert_after = !tuple_slot
        && prepared_default_array_item_value(
            &plugin.draft_config,
            plugin.schema.as_ref(),
            &parent_path,
            index + 1,
        )
        .is_some();
    Some(ArrayItemActionInfo {
        can_insert_before,
        can_insert_after,
        can_duplicate: can_insert_after
            && !schema_unique_items(&parent_schema)
            && can_duplicate_array_item(plugin.schema.as_ref(), &plugin.draft_config, path),
        can_move_up: !tuple_slot && index > tuple_prefix_len,
        can_move_down: !tuple_slot && index + 1 < len,
        can_remove: !tuple_slot
            && len > min_items
            && can_remove_array_item(plugin.schema.as_ref(), &plugin.draft_config, path),
    })
}

pub(in crate::app) fn can_append_array_item(
    plugin: &PluginWorkbenchPlugin,
    path: &[PathSegment],
) -> bool {
    let path = path.to_vec();
    let Some(len) = get_value_at_path(&plugin.draft_config, &path)
        .and_then(JsonValue::as_array)
        .map(|items| items.len())
    else {
        return false;
    };
    prepared_default_array_item_value(&plugin.draft_config, plugin.schema.as_ref(), &path, len)
        .is_some()
}

pub(in crate::app) fn append_default_array_item_at_path(
    root: &mut JsonValue,
    root_schema: Option<&JsonValue>,
    path: &[PathSegment],
) -> Option<ConfigPath> {
    let path = path.to_vec();
    let len = get_value_at_path(root, &path)?.as_array()?.len();
    let value = prepared_default_array_item_value(root, root_schema, &path, len)?;
    let array = get_value_mut_at_path(root, &path)?.as_array_mut()?;
    array.push(value);
    let mut focus_path = path;
    focus_path.push(PathSegment::Index(len));
    Some(focus_path)
}

pub(in crate::app) fn insert_default_array_item_at_path(
    root: &mut JsonValue,
    root_schema: Option<&JsonValue>,
    path: &[PathSegment],
    after: bool,
) -> Option<ConfigPath> {
    let (parent_path, index, _len) = array_item_path_info(root, path)?;
    let insert_index = if after { index + 1 } else { index };
    if let Some(root_schema) = root_schema
        && let Some(parent_schema) = schema_for_path(root_schema, root_schema, root, &parent_path)
        && index < schema_prefix_item_count(&parent_schema)
    {
        return None;
    }
    let value = prepared_default_array_item_value(root, root_schema, &parent_path, insert_index)?;
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    array.insert(insert_index, value);
    let mut focus_path = parent_path;
    focus_path.push(PathSegment::Index(insert_index));
    Some(focus_path)
}

pub(in crate::app) fn path_display(path: &ConfigPath) -> String {
    if path.is_empty() {
        return "/".to_owned();
    }
    let mut out = String::new();
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                out.push('/');
                out.push_str(key);
            }
            PathSegment::Index(index) => {
                out.push('[');
                out.push_str(index.to_string().as_str());
                out.push(']');
            }
        }
    }
    out
}

pub(in crate::app) fn title_for_property(
    root: &JsonValue,
    schema: &JsonValue,
    key: &str,
) -> String {
    object_property_schema(root, schema, key)
        .map(|schema| title_for_schema_or_key(&schema, key))
        .unwrap_or_else(|| title_from_key(key))
}

pub(in crate::app) fn title_for_schema_or_key(schema: &JsonValue, key: &str) -> String {
    schema
        .get("title")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| title_from_key(key))
}

pub(in crate::app) fn title_from_key(key: &str) -> String {
    let mut out = String::new();
    for (index, part) in key
        .split(['_', '-', '.'])
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if index > 0 {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() { key.to_owned() } else { out }
}

pub(in crate::app) fn title_from_path(path: &[PathSegment]) -> String {
    path.iter()
        .rev()
        .find_map(path_segment_key_name)
        .map(title_from_key)
        .unwrap_or_else(|| "Value".to_owned())
}

pub(in crate::app) fn preview_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "null".to_owned(),
        JsonValue::Bool(value) => {
            if *value {
                "yes".to_owned()
            } else {
                "no".to_owned()
            }
        }
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(text) => truncate_text(text, 40),
        JsonValue::Array(items) => format!("{} item(s)", items.len()),
        JsonValue::Object(object) => format!("{} field(s)", object.len()),
    }
}

pub(in crate::app) fn diff_preview(value: &JsonValue) -> String {
    if value.is_null() {
        "missing".to_owned()
    } else {
        preview_value(value)
    }
}

pub(in crate::app) fn diff_summary(before: &JsonValue, after: &JsonValue) -> String {
    match (before, after) {
        (JsonValue::Null, _) => "added".to_owned(),
        (_, JsonValue::Null) => "removed".to_owned(),
        (JsonValue::Object(_), JsonValue::Object(_)) => "modified object".to_owned(),
        (JsonValue::Array(_), JsonValue::Array(_)) => "modified array".to_owned(),
        _ => "changed".to_owned(),
    }
}

pub(in crate::app) fn parse_scalar_editor_value(
    kind: ScalarEditKind,
    input: &str,
) -> UiResult<JsonValue> {
    match kind {
        ScalarEditKind::String => Ok(JsonValue::String(input.to_owned())),
        ScalarEditKind::Number => {
            let parsed = input
                .trim()
                .parse::<f64>()
                .map_err(|error| format!("invalid number: {error}"))?;
            let Some(number) = JsonNumber::from_f64(parsed) else {
                return Err("number cannot be NaN or infinite".to_owned());
            };
            Ok(JsonValue::Number(number))
        }
        ScalarEditKind::Integer => {
            let parsed = input
                .trim()
                .parse::<i64>()
                .map_err(|error| format!("invalid integer: {error}"))?;
            Ok(JsonValue::Number(JsonNumber::from(parsed)))
        }
    }
}

pub(in crate::app) fn parse_pair_integer_editor_values(input: &str) -> UiResult<(i64, i64)> {
    let parts = input
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err("expected two integer values".to_owned());
    }
    let left = parts[0]
        .parse::<i64>()
        .map_err(|error| format!("invalid first integer: {error}"))?;
    let right = parts[1]
        .parse::<i64>()
        .map_err(|error| format!("invalid second integer: {error}"))?;
    Ok((left, right))
}

pub(in crate::app) fn field_prompt_for_row(
    schema: Option<&JsonValue>,
    row: &ConfigRowView,
) -> String {
    let mut parts = vec![format!("Path: {}", path_display(&row.primary_path))];
    if let Some(description) = row.description.as_deref() {
        parts.push(description.to_owned());
    } else if let Some(schema) = schema {
        if let Some(description) = schema_description_text(schema) {
            parts.push(description.to_owned());
        }
    }
    if let Some(schema) = schema {
        if let Some(format) = schema_first_string_keyword(schema, "format") {
            parts.push(format!("format: {format}"));
        }
    }
    parts.extend(row.constraints.iter().cloned());
    parts.join("\n")
}

pub(in crate::app) fn field_prompt_for_path(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
) -> String {
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|root| declared_schema_for_path(root, root, &plugin.draft_config, path));
    let mut parts = vec![format!("Path: {}", path_display(path))];
    if let Some(description) = path_description(plugin, path) {
        parts.push(description);
    } else if let Some(schema) = schema.as_ref()
        && let Some(description) = schema_description_text(schema)
    {
        parts.push(description);
    }
    if let Some(schema) = schema.as_ref()
        && let Some(format) = schema_first_string_keyword(schema, "format")
    {
        parts.push(format!("format: {format}"));
    }
    parts.extend(path_constraints(plugin, path));
    parts.join("\n")
}

pub(in crate::app) fn schema_string_is_multiline(schema: &JsonValue) -> bool {
    schema_first_string_keyword(schema, "format")
        .is_some_and(|format| matches!(format, "markdown" | "multiline" | "textarea"))
        || schema
            .get("maxLength")
            .and_then(JsonValue::as_u64)
            .is_some_and(|max| max > 240)
}

pub(in crate::app) fn pattern_key_matches(pattern: &str, key: &str) -> bool {
    pattern_matches(pattern, key).unwrap_or(false)
}
use super::{
    ArrayItemActionInfo, ConfigPath, ConfigRowCell, ConfigRowPrimaryAction, ConfigRowView,
    DiagnosticSeverity, JsonNumber, JsonValue, PathSegment, PluginConfigAction,
    PluginConfigActionItem, PluginWorkbenchPlugin, ScalarEditKind, UiResult, array_item_path_info,
    array_item_schema, declared_schema_for_path, default_value_for_schema, get_value_at_path,
    get_value_mut_at_path, object_property_schema, path_constraints, path_description,
    path_key_info, path_segment_key_name, pattern_matches, remove_value_at_path,
    replace_last_index, reset_effective_value_at_path, schema_bool_keyword_any,
    schema_description_text, schema_first_string_keyword, schema_for_path,
    schema_max_u64_constraint, schema_min_u64_constraint, schema_prefix_item_count,
    schema_prohibits_additional_properties, schema_property_name_schemas, schema_required_fields,
    set_value_at_path, truncate_text, validate_schema_at,
};
