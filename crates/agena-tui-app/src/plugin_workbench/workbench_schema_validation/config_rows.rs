use super::super::{
    ConfigDiagnostic, ConfigPath, ConfigRowEditor, ConfigRowState, ConfigRowTypeMode,
    ConfigRowView, DiagnosticSeverity, JsonNumber, JsonValue, PathSegment, PluginConfigFocus,
    PluginWorkbenchPlugin, active_branch_label, branch_choices, clean, config_row_action_display,
    declared_schema_for_path, effective_schema_kind, get_value_at_path, json_kind_label,
    path_is_prefix_of, path_present_in_value, preview_value, schema_constraints, schema_for_path,
    schema_type_selector_choices, structured_preview, title_for_schema_or_key, title_from_key,
    truncate_text,
};
use super::schema_description_text;

pub(crate) fn config_row_type_meta(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    editor: &ConfigRowEditor,
) -> (String, ConfigRowTypeMode) {
    let value = get_value_at_path(&plugin.draft_config, path).unwrap_or(&JsonValue::Null);
    let declared_schema = plugin
        .schema
        .as_ref()
        .and_then(|root| declared_schema_for_path(root, root, &plugin.draft_config, path));
    if !matches!(
        editor,
        ConfigRowEditor::ReadOnly
            | ConfigRowEditor::Enum
            | ConfigRowEditor::MultiEnum { .. }
            | ConfigRowEditor::PairInteger { .. }
    ) {
        if let Some(schema) = declared_schema.as_ref()
            && let Some(root) = plugin.schema.as_ref()
            && let Some(branches) = branch_choices(root, schema)
        {
            return (
                format!("[ {} ▾ ]", active_branch_label(branches.as_slice(), value)),
                ConfigRowTypeMode::SelectShape,
            );
        }
        let choices = schema_type_selector_choices(declared_schema.as_ref());
        if choices.len() > 1 {
            return (
                format!("[ {} ▾ ]", json_kind_label(value)),
                ConfigRowTypeMode::SelectType,
            );
        }
    }
    let display = declared_schema
        .as_ref()
        .and_then(effective_schema_kind)
        .unwrap_or_else(|| json_kind_label(value).to_owned());
    (display, ConfigRowTypeMode::Fixed)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_config_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    primary_path: ConfigPath,
    additional_paths: Vec<ConfigPath>,
    editor: ConfigRowEditor,
    value_display: String,
    default_display: String,
    secondary_value_display: Option<String>,
    action_display: Option<String>,
    _secondary_default_display: Option<String>,
    inactive_reason: Option<String>,
    description: Option<String>,
    constraints: Vec<String>,
) -> ConfigRowView {
    let all_paths = std::iter::once(&primary_path)
        .chain(additional_paths.iter())
        .cloned()
        .collect::<Vec<_>>();
    let diagnostics = plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            all_paths
                .iter()
                .any(|path| path_is_prefix_of(path.as_slice(), diagnostic.path.as_slice()))
        })
        .cloned()
        .collect::<Vec<_>>();
    let dirty = all_paths
        .iter()
        .any(|path| value_changed_at_path(&plugin.saved_config, &plugin.draft_config, path));
    let override_count = all_paths
        .iter()
        .filter(|path| path_present_in_value(&plugin.draft_override, path.as_slice()))
        .count();
    let state = if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        ConfigRowState::Error
    } else if dirty {
        ConfigRowState::Dirty
    } else if inactive_reason.is_some() {
        ConfigRowState::Inactive
    } else if override_count > 0 {
        ConfigRowState::Override
    } else {
        ConfigRowState::Default
    };
    let (type_display, type_mode) = config_row_type_meta(plugin, &primary_path, &editor);
    let action_display = action_display.or_else(|| {
        config_row_action_display(
            plugin,
            &editor,
            primary_path.as_slice(),
            additional_paths.as_slice(),
        )
    });
    ConfigRowView {
        title: title.to_owned(),
        primary_path,
        additional_paths,
        editor,
        description,
        constraints,
        type_display,
        type_mode,
        value_display,
        default_display,
        secondary_value_display,
        action_display,
        state,
    }
}

pub(crate) fn value_changed_at_path(
    before: &JsonValue,
    after: &JsonValue,
    path: &ConfigPath,
) -> bool {
    get_value_at_path(before, path) != get_value_at_path(after, path)
}

pub(crate) fn override_leaf_count(value: &JsonValue) -> usize {
    match value {
        JsonValue::Null => 0,
        JsonValue::Object(object) => object.values().map(override_leaf_count).sum(),
        JsonValue::Array(items) => {
            if items.is_empty() {
                1
            } else {
                items.iter().map(override_leaf_count).sum()
            }
        }
        _ => 1,
    }
}

pub(crate) fn title_for_config_path(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    fallback: &str,
) -> String {
    plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path))
        .as_ref()
        .map(|schema| title_for_schema_or_key(schema, fallback))
        .unwrap_or_else(|| title_from_key(fallback))
}

pub(crate) fn path_description(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
) -> Option<String> {
    plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path))
        .and_then(|schema| schema_description_text(&schema))
}

pub(crate) fn path_constraints(plugin: &PluginWorkbenchPlugin, path: &ConfigPath) -> Vec<String> {
    plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path))
        .map(|schema| schema_constraints(&schema))
        .unwrap_or_default()
}

pub(crate) fn pair_constraints(
    plugin: &PluginWorkbenchPlugin,
    left_path: &ConfigPath,
    right_path: &ConfigPath,
) -> Vec<String> {
    let mut constraints = path_constraints(plugin, left_path);
    constraints.extend(path_constraints(plugin, right_path));
    constraints.sort();
    constraints.dedup();
    constraints
}

pub(crate) fn format_bool_checkbox(value: bool) -> String {
    if value {
        "[x]".to_owned()
    } else {
        "[ ]".to_owned()
    }
}

pub(crate) fn format_value_with_brackets(path: &ConfigPath, value: &JsonValue) -> String {
    match value {
        JsonValue::Bool(value) => format_bool_checkbox(*value),
        JsonValue::String(text) => format!("[ {} ]", clean(truncate_text(text, 28))),
        JsonValue::Number(number) => format!("[ {} ]", format_number_with_unit(path, number)),
        JsonValue::Null => "[ null ]".to_owned(),
        JsonValue::Array(_) | JsonValue::Object(_) => format!("[ {} ]", structured_preview(value)),
    }
}

pub(crate) fn format_default_value(path: &ConfigPath, value: &JsonValue) -> String {
    match value {
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::String(text) => clean(truncate_text(text, 28)),
        JsonValue::Number(number) => format_number_with_unit(path, number),
        JsonValue::Null => "Not set".to_owned(),
        JsonValue::Array(_) | JsonValue::Object(_) => structured_preview(value),
    }
}

pub(crate) fn format_nullable_value_for_cell(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "[ Not set ]".to_owned(),
        JsonValue::String(text) => format!("[ {} ]", clean(truncate_text(text, 24))),
        _ => format!("[ {} ]", preview_value(value)),
    }
}

pub(crate) fn format_default_nullable_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "Not set".to_owned(),
        JsonValue::String(text) => clean(truncate_text(text, 28)),
        _ => preview_value(value),
    }
}

pub(crate) fn format_number_with_unit(path: &ConfigPath, number: &JsonNumber) -> String {
    let Some(last) = path.last() else {
        return number.to_string();
    };
    let PathSegment::Key(key) = last else {
        return number.to_string();
    };
    if key.ends_with("_ms") {
        return format!("{} ms", number);
    }
    if key.ends_with("_secs") {
        return format!("{} sec", number);
    }
    if key.ends_with("_chars") {
        return format!("{} ch", number);
    }
    if key.ends_with("_bytes") {
        return format_bytes_summary(number.as_u64().unwrap_or_default());
    }
    number.to_string()
}

pub(crate) fn format_bytes_summary(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

pub(crate) fn compact_duration_summary(value: u64, suffix: &str, label: &str) -> String {
    if label.is_empty() {
        format!("{value}{suffix}")
    } else {
        format!("{value}{suffix} {label}")
    }
}

pub(crate) fn runtime_diagnostics(
    status: &agena_plugin_host::status::PluginStatus,
) -> Vec<ConfigDiagnostic> {
    if status.state == agena_plugin_host::status::PluginRunState::Failed {
        vec![ConfigDiagnostic {
            severity: DiagnosticSeverity::Error,
            source: "runtime".to_owned(),
            path: Vec::new(),
            field: "Process".to_owned(),
            message: status
                .last_error
                .clone()
                .unwrap_or_else(|| "plugin failed".to_owned()),
        }]
    } else {
        Vec::new()
    }
}

pub(crate) fn next_config_focus(focus: PluginConfigFocus, compact: bool) -> PluginConfigFocus {
    let _ = compact;
    match focus {
        PluginConfigFocus::Structure => PluginConfigFocus::Editor,
        _ => PluginConfigFocus::Structure,
    }
}

pub(crate) fn previous_config_focus(focus: PluginConfigFocus, compact: bool) -> PluginConfigFocus {
    let _ = compact;
    match focus {
        PluginConfigFocus::Editor => PluginConfigFocus::Structure,
        PluginConfigFocus::Structure => PluginConfigFocus::Editor,
        _ => PluginConfigFocus::Editor,
    }
}
