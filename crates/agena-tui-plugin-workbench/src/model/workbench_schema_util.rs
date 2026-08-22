pub(crate) fn schema_property_count(schema: &JsonValue) -> usize {
    let property_count = schema_declared_property_keys(schema).len();
    if property_count > 0 {
        return property_count;
    }
    let prefix_count = schema_prefix_item_count(schema);
    if prefix_count > 0 {
        return prefix_count;
    }
    if schema_has_array_shape(schema) || schema_has_map_keywords(schema) {
        1
    } else {
        0
    }
}

pub(crate) fn operation_argument_count(
    _plugin: &PluginWorkbenchPlugin,
    operation: &agena_plugin_host::PluginOperationDefinition,
) -> usize {
    use agena_plugin_host::sdk::SettingsNodeKind;
    match &operation.input.root.kind {
        SettingsNodeKind::Object { fields } => fields.len(),
        _ => 1,
    }
}

pub(crate) fn operation_schema_and_value(
    _plugin: &PluginWorkbenchPlugin,
    operation: &agena_plugin_host::PluginOperationDefinition,
) -> Option<(JsonValue, JsonValue)> {
    let schema = settings_contract_editor_schema(&operation.input);
    let value = operation.input.default_value().ok()?;
    Some((schema, value))
}

pub(crate) fn schema_is_map_like(root: &JsonValue, schema: &JsonValue) -> bool {
    let schema = active_schema_for_value(root, schema, &JsonValue::Object(JsonMap::new()));
    schema_has_object_shape(&schema) && schema_has_map_keywords(&schema)
}

pub(crate) fn schema_has_map_keywords(schema: &JsonValue) -> bool {
    if schema.as_object().is_some_and(|object| {
        object.contains_key("additionalProperties")
            || object.contains_key("patternProperties")
            || object.contains_key("propertyNames")
    }) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_has_map_keywords))
}

pub(crate) fn schema_prohibits_additional_properties(schema: &JsonValue) -> bool {
    if schema.get("additionalProperties") == Some(&JsonValue::Bool(false)) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_prohibits_additional_properties))
}

pub(crate) fn schema_property_name_schemas(schema: &JsonValue) -> Vec<JsonValue> {
    let mut schemas = schema
        .get("propertyNames")
        .cloned()
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            schemas.extend(schema_property_name_schemas(branch));
        }
    }
    schemas
}

pub(crate) fn schema_prefix_item_count(schema: &JsonValue) -> usize {
    let direct = schema
        .get("prefixItems")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let nested = schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .map(|branches| {
            branches
                .iter()
                .map(schema_prefix_item_count)
                .max()
                .unwrap_or_default()
        })
        .unwrap_or_default();
    direct.max(nested)
}

pub(crate) fn schema_min_u64_constraint(schema: &JsonValue, key: &str) -> Option<u64> {
    let direct = schema.get(key).and_then(JsonValue::as_u64);
    let nested = schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| {
            branches
                .iter()
                .filter_map(|branch| schema_min_u64_constraint(branch, key))
                .max()
        });
    match (direct, nested) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

pub(crate) fn schema_max_u64_constraint(schema: &JsonValue, key: &str) -> Option<u64> {
    let direct = schema.get(key).and_then(JsonValue::as_u64);
    let nested = schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| {
            branches
                .iter()
                .filter_map(|branch| schema_max_u64_constraint(branch, key))
                .min()
        });
    match (direct, nested) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

pub(crate) fn schema_declared_property_keys(schema: &JsonValue) -> BTreeSet<String> {
    let mut keys = schema
        .get("properties")
        .and_then(JsonValue::as_object)
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            keys.extend(schema_declared_property_keys(branch));
        }
    }
    keys
}

pub(crate) fn schema_matches_pattern_property(schema: &JsonValue, key: &str) -> bool {
    if schema
        .get("patternProperties")
        .and_then(JsonValue::as_object)
        .is_some_and(|patterns| {
            patterns
                .keys()
                .any(|pattern| pattern_key_matches(pattern, key))
        })
    {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_matches_pattern_property(branch, key))
        })
}

pub(crate) fn ordered_object_keys(
    schema: Option<&JsonValue>,
    object: &JsonMap<String, JsonValue>,
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(schema) = schema {
        let required = schema_required_fields(schema);
        for key in &required {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
        for key in schema_declared_property_keys(schema) {
            if seen.insert(key.clone()) {
                keys.push(key);
            }
        }
    }
    for key in object.keys() {
        if seen.insert(key.clone()) {
            keys.push(key.clone());
        }
    }
    keys
}

pub(crate) fn schema_required_fields(schema: &JsonValue) -> BTreeSet<String> {
    let mut required = schema
        .get("required")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            required.extend(schema_required_fields(branch));
        }
    }
    required
}

pub(crate) fn object_field_state(
    root: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    key: &str,
    present: bool,
) -> String {
    let Some(schema) = schema else {
        return "custom".to_owned();
    };
    if schema_required_fields(schema).contains(key) {
        if present {
            "required".to_owned()
        } else {
            "missing".to_owned()
        }
    } else if schema_declared_property_keys(schema).contains(key)
        || (present
            && root.is_some_and(|root| {
                object_property_schema(root, schema, key).is_some()
                    || schema_matches_pattern_property(schema, key)
            }))
    {
        if present {
            "optional".to_owned()
        } else {
            "available".to_owned()
        }
    } else {
        "map key".to_owned()
    }
}

pub(crate) fn object_array_columns(schema: Option<&JsonValue>, items: &[JsonValue]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(schema) = schema {
        for key in schema_declared_property_keys(schema).into_iter().take(4) {
            if seen.insert(key.clone()) {
                keys.push(key);
            }
        }
    }
    for item in items {
        if let Some(object) = item.as_object() {
            for key in object.keys().take(4) {
                if seen.insert(key.clone()) {
                    keys.push(key.clone());
                }
                if keys.len() >= 4 {
                    return keys;
                }
            }
        }
    }
    if keys.is_empty() {
        keys.push("value".to_owned());
    }
    keys
}

pub(crate) fn structured_preview(value: &JsonValue) -> String {
    match value {
        JsonValue::Object(object) => format!("Configure... ({} field(s))", object.len()),
        JsonValue::Array(items) => format!("Configure... ({} item(s))", items.len()),
        _ => preview_value(value),
    }
}

pub(crate) fn number_constraint_summary(schema: &JsonValue) -> String {
    let mut parts = BTreeSet::new();
    for key in [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
    ] {
        if let Some(value) = schema.get(key) {
            parts.insert(format!("{key}: {}", preview_value(value)));
        }
    }
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            for part in schema_constraints(branch).into_iter().filter(|part| {
                part.starts_with("minimum:")
                    || part.starts_with("maximum:")
                    || part.starts_with("exclusiveMinimum:")
                    || part.starts_with("exclusiveMaximum:")
                    || part.starts_with("multipleOf:")
            }) {
                parts.insert(part);
            }
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("     {}", parts.into_iter().collect::<Vec<_>>().join("   "))
    }
}

pub(crate) fn schema_constraints(schema: &JsonValue) -> Vec<String> {
    let mut constraints = BTreeSet::new();
    for key in [
        "format",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
    ] {
        if let Some(value) = schema.get(key) {
            constraints.insert(format!("{key}: {}", preview_value(value)));
        }
    }
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            constraints.extend(schema_constraints(branch));
        }
    }
    constraints.into_iter().collect()
}

pub fn plugin_workbench_summary(dialog: &PluginWorkbenchOverlay) -> String {
    let query = if dialog.list.query.text().is_empty() {
        dialog.i18n.text("plugin-workbench-filter-all")
    } else {
        dialog.list.query.text().to_owned()
    };
    dialog.i18n.text_args(
        "plugin-workbench-summary",
        &agena_tui::fl_args![
            "query" => query,
            "transport" => dialog.list.transport_filter.label(&dialog.i18n),
            "config" => dialog.list.config_filter.label(&dialog.i18n),
            "shown" => dialog.list.visible_len(),
            "total" => dialog.plugins.len(),
        ],
    )
}

pub(crate) fn fixed_columns(columns: &[(&str, usize)], width: u16) -> String {
    agena_tui_components::format_fixed_columns(columns, width, |text| clean(text))
}

pub(crate) fn pad_to_width(text: &str, width: usize) -> String {
    let clipped = truncate_text(text, width);
    let padding = width.saturating_sub(clipped.width());
    format!("{clipped}{}", " ".repeat(padding))
}

pub(crate) fn wrap_prefixed_text(
    text: &str,
    first_prefix: &str,
    rest_prefix: &str,
    width: usize,
) -> Vec<String> {
    let available_first = width.saturating_sub(first_prefix.width()).max(1);
    let available_rest = width.saturating_sub(rest_prefix.width()).max(1);
    let mut lines = Vec::new();
    let mut prefix = first_prefix;
    let mut available = available_first;
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in text.split_whitespace() {
        let mut remaining = word.to_owned();
        loop {
            let room = if current.is_empty() {
                available
            } else {
                available.saturating_sub(current_width + 1)
            };
            if room == 0 {
                lines.push(format!("{prefix}{current}"));
                prefix = rest_prefix;
                available = available_rest;
                current.clear();
                current_width = 0;
                continue;
            }
            if remaining.width() <= room {
                if !current.is_empty() {
                    current.push(' ');
                    current_width += 1;
                }
                current.push_str(remaining.as_str());
                current_width += remaining.width();
                break;
            }

            let chunk = take_width_prefix(remaining.as_str(), room);
            if chunk.is_empty() {
                break;
            }
            if !current.is_empty() {
                lines.push(format!("{prefix}{current}"));
                prefix = rest_prefix;
                current.clear();
                current_width = 0;
            }
            lines.push(format!("{prefix}{chunk}"));
            let consumed = chunk.len();
            remaining = remaining[consumed..].to_owned();
            prefix = rest_prefix;
            available = available_rest;
        }
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(format!("{prefix}{current}"));
    }

    lines
}

pub(crate) fn take_width_prefix(text: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or_default();
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

pub(crate) fn plugin_package_preview(value: &JsonValue) -> String {
    value
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| preview_value(value))
}

pub(crate) fn plugin_workbench_selection_highlight_style() -> Style {
    agena_tui_components::theme::selection_style()
}

pub fn quote_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn plugin_get_json_path(
    value: &JsonValue,
    path: Option<&str>,
) -> Result<JsonValue, String> {
    agena_domain::get_json_path(value, path)
        .map_err(|error| agena_failure::diagnostic::format_error_chain(&error))
}

pub fn plugin_config_record_value(plugin: &PluginWorkbenchPlugin) -> JsonValue {
    plugin.configured_plugin_value.clone().unwrap_or_else(|| {
        json!({
            "enabled": true,
            "package": {
                "kind": "static"
            },
            "settings": JsonValue::Null
        })
    })
}

pub fn move_selected_config_node(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    let item_count = dialog
        .selected_section()
        .map(|section| section_row_count(section, dialog.config_view))
        .unwrap_or_default();
    move_index(&mut dialog.selected_node, item_count, delta);
    dialog.clamp_selection();
}

pub fn move_detail_scroll(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    if dialog.navigation.detail_tab == PluginDetailTab::Diagnostics {
        move_index(&mut dialog.diagnostics_scroll, usize::MAX / 2, delta);
    } else {
        move_index(&mut dialog.config_scroll, usize::MAX / 2, delta);
    }
}

pub fn move_index(index: &mut usize, item_count: usize, delta: isize) {
    if item_count == 0 {
        *index = 0;
        return;
    }
    let last = item_count.saturating_sub(1) as isize;
    *index = (*index as isize + delta).clamp(0, last) as usize;
}

pub(crate) fn truncate_text(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_owned();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut out = String::new();
    let mut width = 0;
    let suffix_width = 3;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or_default();
        if width + ch_width + suffix_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push_str("...");
    out
}

pub fn clean(text: impl AsRef<str>) -> String {
    text.as_ref()
        .chars()
        .map(|ch| {
            if ch.is_control() && ch != '\n' && ch != '\t' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}
use super::{
    BTreeSet, JsonMap, JsonValue, PluginDetailTab, PluginWorkbenchOverlay, PluginWorkbenchPlugin,
    Style, active_schema_for_value, json, object_property_schema, pattern_key_matches,
    preview_value, schema_has_array_shape, schema_has_object_shape, section_row_count,
    settings_contract_editor_schema,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
