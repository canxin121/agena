use super::super::{
    ConfigGroupLayout, ConfigGroupView, ConfigOverviewCard, ConfigPath, ConfigRowEditor,
    ConfigRowView, ConfigSectionBody, ConfigSectionView, DiagnosticSeverity, JsonNumber, JsonValue,
    PathSegment, PluginWorkbenchPlugin, array_enum_variants, build_config_row,
    declared_schema_for_path, diff_config_values, effective_schema_kind, format_bool_checkbox,
    format_default_nullable_value, format_default_value, format_multi_enum_default_value,
    format_multi_enum_value_with_selector, format_nullable_value_for_cell,
    format_value_with_brackets, get_value_at_path, ordered_object_keys, override_leaf_count,
    pair_constraints, path_constraints, path_description, path_is_prefix_of, path_present_in_value,
    preview_value, schema_bool_keyword_any, schema_const_value, schema_enum_values,
    schema_for_path, schema_is_map_like, schema_uses_nullable_string_editor, structured_preview,
    title_for_config_path, title_for_schema_or_key,
};

pub(in crate::app) fn config_path<const N: usize>(segments: [&str; N]) -> ConfigPath {
    segments
        .into_iter()
        .map(|segment| PathSegment::Key(segment.to_owned()))
        .collect()
}

pub(in crate::app) fn section_issue_label(
    plugin: &PluginWorkbenchPlugin,
    path: &[PathSegment],
) -> Option<String> {
    let count = plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && path_is_prefix_of(path, diagnostic.path.as_slice())
        })
        .count();
    (count > 0).then(|| format!("Error {count}"))
}

pub(in crate::app) fn section_issue_count(
    plugin: &PluginWorkbenchPlugin,
    path: &[PathSegment],
) -> usize {
    plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && path_is_prefix_of(path, diagnostic.path.as_slice())
        })
        .count()
}

pub(in crate::app) fn section_dirty(plugin: &PluginWorkbenchPlugin, path: &[PathSegment]) -> bool {
    if path.is_empty() {
        return plugin.dirty;
    }
    path_present_in_value(&plugin.draft_override, path)
        || !diff_config_values(
            get_value_at_path(&plugin.saved_config, &path.to_vec()).unwrap_or(&JsonValue::Null),
            get_value_at_path(&plugin.draft_config, &path.to_vec()).unwrap_or(&JsonValue::Null),
        )
        .is_empty()
}

pub(in crate::app) fn web_form_section(
    plugin: &PluginWorkbenchPlugin,
    key: &str,
    title: &str,
    path: ConfigPath,
    notice: Option<String>,
    groups: Vec<ConfigGroupView>,
) -> ConfigSectionView {
    ConfigSectionView {
        key: key.to_owned(),
        title: title.to_owned(),
        issue_count: section_issue_count(plugin, &path),
        dirty: section_dirty(plugin, &path),
        body: ConfigSectionBody::Form { notice, groups },
    }
}

pub(in crate::app) fn build_generic_overview_section(
    plugin: &PluginWorkbenchPlugin,
) -> ConfigSectionView {
    let mut cards = Vec::new();
    if let Some(root) = plugin.draft_config.as_object() {
        for key in ordered_object_keys(plugin.schema.as_ref(), root) {
            let path = vec![PathSegment::Key(key.clone())];
            let summary = get_value_at_path(&plugin.draft_config, &path)
                .map(preview_value)
                .unwrap_or_else(|| "missing".to_owned());
            cards.push(ConfigOverviewCard {
                title: title_for_config_path(plugin, &path, key.as_str()),
                summary,
                issue_label: section_issue_label(plugin, &path),
            });
        }
    }
    ConfigSectionView {
        key: "overview".to_owned(),
        title: "Overview".to_owned(),
        issue_count: plugin
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .count(),
        dirty: plugin.dirty,
        body: ConfigSectionBody::Overview {
            cards,
            lines: vec![
                format!(
                    "Schema             {}",
                    if plugin.schema_missing {
                        "Missing"
                    } else {
                        "Available"
                    }
                ),
                "Effective mode     Full config values".to_owned(),
                format!(
                    "Changed            {} field(s)",
                    override_leaf_count(&plugin.draft_override)
                ),
                format!(
                    "Diagnostics        {}",
                    if plugin.diagnostics.is_empty() {
                        "No issues".to_owned()
                    } else {
                        format!("{} issue(s)", plugin.diagnostics.len())
                    }
                ),
            ],
        },
    }
}

pub(in crate::app) fn build_generic_section(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    title: String,
) -> ConfigSectionView {
    let value = get_value_at_path(&plugin.draft_config, path).unwrap_or(&JsonValue::Null);
    let groups = if value.is_object() {
        build_generic_object_groups(plugin, path, title.as_str())
    } else {
        vec![ConfigGroupView {
            title: title.clone(),
            layout: ConfigGroupLayout::Standard,
            rows: vec![build_row_for_path(
                plugin,
                path.clone(),
                title.as_str(),
                None,
            )],
        }]
    };
    ConfigSectionView {
        key: path
            .last()
            .and_then(|segment| match segment {
                PathSegment::Key(key) => Some(key.clone()),
                PathSegment::Index(_) => None,
            })
            .unwrap_or_else(|| "config".to_owned()),
        title,
        issue_count: section_issue_count(plugin, path),
        dirty: section_dirty(plugin, path),
        body: ConfigSectionBody::Form {
            notice: None,
            groups,
        },
    }
}

pub(in crate::app) fn build_generic_object_groups(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    title: &str,
) -> Vec<ConfigGroupView> {
    let value = get_value_at_path(&plugin.draft_config, path)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path));
    let mut primitive_rows = Vec::new();
    let mut groups = Vec::new();
    for key in ordered_object_keys(schema.as_ref(), &value) {
        let mut child_path = path.clone();
        child_path.push(PathSegment::Key(key.clone()));
        let child_value = value.get(key.as_str()).unwrap_or(&JsonValue::Null);
        let child_schema = plugin.schema.as_ref().and_then(|root_schema| {
            declared_schema_for_path(root_schema, root_schema, &plugin.draft_config, &child_path)
        });
        if should_expand_object_child(plugin, child_schema.as_ref(), child_value) {
            groups.push(ConfigGroupView {
                title: title_for_config_path(plugin, &child_path, key.as_str()),
                layout: ConfigGroupLayout::Standard,
                rows: flatten_generic_object_rows(plugin, &child_path),
            });
        } else {
            primitive_rows.push(build_row_for_path(
                plugin,
                child_path,
                title_for_schema_or_key(
                    child_schema.as_ref().unwrap_or(&JsonValue::Null),
                    key.as_str(),
                )
                .as_str(),
                None,
            ));
        }
    }
    if !primitive_rows.is_empty() {
        groups.insert(
            0,
            ConfigGroupView {
                title: title.to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: primitive_rows,
            },
        );
    }
    if groups.is_empty() {
        groups.push(ConfigGroupView {
            title: title.to_owned(),
            layout: ConfigGroupLayout::Standard,
            rows: vec![build_structured_row(plugin, title, path.clone(), None)],
        });
    }
    groups
}

pub(in crate::app) fn should_expand_object_child(
    plugin: &PluginWorkbenchPlugin,
    child_schema: Option<&JsonValue>,
    child_value: &JsonValue,
) -> bool {
    if !child_value.is_object() {
        return false;
    }
    let Some(child_schema) = child_schema else {
        return false;
    };
    let root = plugin.schema.as_ref().unwrap_or(child_schema);
    effective_schema_kind(child_schema).as_deref() == Some("object")
        && !schema_is_map_like(root, child_schema)
}

pub(in crate::app) fn flatten_generic_object_rows(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
) -> Vec<ConfigRowView> {
    let value = get_value_at_path(&plugin.draft_config, path)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path));
    let mut rows = Vec::new();
    for key in ordered_object_keys(schema.as_ref(), &value) {
        let mut child_path = path.clone();
        child_path.push(PathSegment::Key(key.clone()));
        rows.push(build_row_for_path(
            plugin,
            child_path.clone(),
            title_for_config_path(plugin, &child_path, key.as_str()).as_str(),
            None,
        ));
    }
    rows
}

pub(in crate::app) fn build_row_for_path(
    plugin: &PluginWorkbenchPlugin,
    path: ConfigPath,
    title: &str,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path).unwrap_or(&JsonValue::Null);
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, &path));
    if schema.as_ref().is_some_and(|schema| {
        schema_const_value(schema).is_some() || schema_bool_keyword_any(schema, "readOnly")
    }) {
        return build_read_only_row(plugin, title, path, inactive_reason);
    }
    if let Some(variants) = schema.as_ref().and_then(|schema| {
        plugin
            .schema
            .as_ref()
            .and_then(|root| array_enum_variants(root, schema))
    }) {
        return build_multi_enum_row(plugin, title, path, variants, inactive_reason);
    }
    if schema.as_ref().and_then(schema_enum_values).is_some() {
        return build_enum_row(plugin, title, path, inactive_reason);
    }
    if schema
        .as_ref()
        .is_some_and(schema_uses_nullable_string_editor)
    {
        return build_nullable_string_row(plugin, title, path, inactive_reason);
    }
    match value {
        JsonValue::Bool(_) => build_bool_row(plugin, title, path, inactive_reason),
        JsonValue::Number(number) if number.as_i64().is_some() || number.as_u64().is_some() => {
            build_integer_row(plugin, title, path, inactive_reason)
        }
        JsonValue::Number(_) => build_number_row(plugin, title, path, inactive_reason),
        JsonValue::String(_) => build_string_row(plugin, title, path, inactive_reason),
        JsonValue::Null => build_null_row(plugin, title, path, inactive_reason),
        JsonValue::Object(_) | JsonValue::Array(_) => {
            build_structured_row(plugin, title, path, inactive_reason)
        }
    }
}

pub(in crate::app) fn build_read_only_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let mut constraints = path_constraints(plugin, &path);
    constraints.push("read-only".to_owned());
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::ReadOnly,
        preview_value(&value),
        preview_value(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        constraints,
    )
}

pub(in crate::app) fn build_bool_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Bool(false));
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Bool(false));
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Bool { path: path.clone() },
        format_bool_checkbox(value.as_bool().unwrap_or(false)),
        default
            .as_bool()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "false".to_owned()),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

pub(in crate::app) fn build_integer_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    build_numeric_row(plugin, title, path, inactive_reason)
}

pub(in crate::app) fn build_number_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    build_numeric_row(plugin, title, path, inactive_reason)
}

pub(in crate::app) fn build_numeric_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Scalar,
        format_value_with_brackets(&path, &value),
        format_default_value(&path, &default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

pub(in crate::app) fn build_string_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::String(String::new()));
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::String(String::new()));
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Scalar,
        format_value_with_brackets(&path, &value),
        format_default_value(&path, &default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

pub(in crate::app) fn build_nullable_string_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::NullableString { path: path.clone() },
        format_nullable_value_for_cell(&value),
        format_default_nullable_value(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

pub(in crate::app) fn build_null_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Null,
        format_value_with_brackets(&path, &value),
        format_default_value(&path, &default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

pub(in crate::app) fn build_enum_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Enum,
        format!("[ {} ▾ ]", preview_value(&value)),
        preview_value(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

pub(in crate::app) fn build_multi_enum_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    variants: Vec<JsonValue>,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let default = get_value_at_path(&plugin.default_config, &path)
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::MultiEnum {
            path: path.clone(),
            variants,
        },
        format_multi_enum_value_with_selector(value.as_slice()),
        format_multi_enum_default_value(default.as_slice()),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

pub(in crate::app) fn build_pair_integer_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    left_path: ConfigPath,
    right_path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let left_value = get_value_at_path(&plugin.draft_config, &left_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let right_value = get_value_at_path(&plugin.draft_config, &right_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let left_default = get_value_at_path(&plugin.default_config, &left_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let right_default = get_value_at_path(&plugin.default_config, &right_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    build_config_row(
        plugin,
        title,
        left_path.clone(),
        vec![right_path.clone()],
        ConfigRowEditor::PairInteger {
            left_path: left_path.clone(),
            right_path: right_path.clone(),
        },
        format_value_with_brackets(&left_path, &left_value),
        format_default_value(&left_path, &left_default),
        Some(format_value_with_brackets(&right_path, &right_value)),
        None,
        Some(format_default_value(&right_path, &right_default)),
        inactive_reason,
        path_description(plugin, &left_path),
        pair_constraints(plugin, &left_path, &right_path),
    )
}

pub(in crate::app) fn build_structured_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Structured { path: path.clone() },
        structured_preview(&value),
        structured_preview(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}
