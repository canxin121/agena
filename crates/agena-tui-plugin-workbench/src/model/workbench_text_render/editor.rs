use super::super::{
    JsonMap, JsonNumber, JsonValue, Line, Modifier, Span, Style, active_branch_id,
    active_branch_label, active_schema_for_value, array_item_schema, branch_choices, clean,
    fixed_columns, json_kind_label, number_constraint_summary, object_array_columns,
    object_field_state, object_property_schema, ordered_object_keys, preview_value,
    schema_const_value, schema_enum_values, schema_examples, schema_first_string_keyword,
    schema_is_map_like, schema_kind_label, schema_matches, schema_prefix_item_count,
    schema_string_is_multiline, schema_type_choices, title_from_key, truncate_text,
};
use agena_tui::i18n::I18n;

fn localized_kind_label(i18n: &I18n, label: &str) -> String {
    let key = match label {
        "null" => "plugin-workbench-kind-null",
        "boolean" => "plugin-workbench-kind-boolean",
        "integer" => "plugin-workbench-kind-integer",
        "number" => "plugin-workbench-kind-number",
        "string" => "plugin-workbench-kind-string",
        "array" => "plugin-workbench-kind-array",
        "object" => "plugin-workbench-kind-object",
        "value" => "plugin-workbench-kind-value",
        "oneOf" => "plugin-workbench-kind-one-of",
        "anyOf" => "plugin-workbench-kind-any-of",
        "allOf" => "plugin-workbench-kind-all-of",
        _ => return label.to_owned(),
    };
    i18n.text(key)
}

fn localized_object_field_state(i18n: &I18n, state: &str) -> String {
    let key = match state {
        "custom" => "plugin-workbench-field-state-custom",
        "required" => "plugin-workbench-field-state-required",
        "missing" => "plugin-workbench-field-state-missing",
        "optional" => "plugin-workbench-field-state-optional",
        "available" => "plugin-workbench-field-state-available",
        "map key" => "plugin-workbench-field-state-map-key",
        _ => return state.to_owned(),
    };
    i18n.text(key)
}

fn localized_structured_preview(i18n: &I18n, value: &JsonValue) -> String {
    match value {
        JsonValue::Object(object) => i18n.text_args(
            "plugin-workbench-editor-object-preview",
            &agena_tui::fl_args!("count" => object.len()),
        ),
        JsonValue::Array(items) => i18n.text_args(
            "plugin-workbench-editor-array-preview",
            &agena_tui::fl_args!("count" => items.len()),
        ),
        _ => preview_value(value),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_schema_editor_lines(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    value: &JsonValue,
    title: &str,
    depth: usize,
    width: u16,
    remaining: usize,
) {
    if remaining == 0 {
        lines.push(Line::from(format!(
            "{}[ {} ]",
            "  ".repeat(depth),
            i18n.text("plugin-workbench-editor-configure")
        )));
        return;
    }
    let active_schema = schema.map(|schema| {
        root_schema
            .map(|root| active_schema_for_value(root, schema, value))
            .unwrap_or_else(|| schema.clone())
    });
    let declared_schema = schema;
    let render_schema = active_schema.as_ref().or(declared_schema);
    let title = clean(title);
    let indent = "  ".repeat(depth);
    if depth == 0 {
        lines.push(Line::from(Span::styled(
            title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let kind = render_schema
            .map(schema_kind_label)
            .unwrap_or_else(|| json_kind_label(value).to_owned());
        lines.push(Line::from(i18n.text_args(
            "plugin-workbench-editor-type-summary",
            &agena_tui::fl_args!("type" => localized_kind_label(i18n, kind.as_str())),
        )));
        lines.push(Line::from(""));
    }

    if let Some(schema) = declared_schema {
        append_branch_selector_lines(
            lines,
            i18n,
            root_schema.unwrap_or(schema),
            schema,
            value,
            depth,
            width,
        );
        append_type_selector_line(lines, i18n, schema, value, depth);
    }
    if let Some(schema) = render_schema {
        if let Some(constant) = schema_const_value(schema) {
            lines.push(Line::from(format!(
                "{}{}        [ {} ] {}",
                indent,
                title,
                preview_value(&constant),
                i18n.text("plugin-workbench-editor-readonly")
            )));
            return;
        }
        if let Some(variants) = schema_enum_values(schema)
            && !variants.is_empty()
        {
            lines.push(Line::from(format!(
                "{}{}        [ {} v ]",
                indent,
                title,
                preview_value(value)
            )));
            return;
        }
    } else if depth == 0 {
        lines.push(Line::from(
            i18n.text("plugin-workbench-editor-schema-missing"),
        ));
    }

    match value {
        JsonValue::Object(object) => append_object_editor_lines(
            lines,
            i18n,
            root_schema,
            render_schema,
            object,
            depth,
            width,
            remaining,
        ),
        JsonValue::Array(items) => append_array_editor_lines(
            lines,
            i18n,
            root_schema,
            render_schema,
            items,
            depth,
            width,
            remaining,
        ),
        JsonValue::String(text) => {
            append_string_editor_lines(lines, i18n, render_schema, title.as_str(), text, depth)
        }
        JsonValue::Number(number) => {
            append_number_editor_lines(lines, i18n, render_schema, title.as_str(), number, depth)
        }
        JsonValue::Bool(value) => lines.push(Line::from(format!(
            "{}{}        [{}]",
            indent,
            title,
            if *value { "x" } else { " " }
        ))),
        JsonValue::Null => append_null_editor_lines(
            lines,
            i18n,
            declared_schema.or(render_schema),
            title.as_str(),
            depth,
        ),
    }
}

pub(crate) fn append_branch_selector_lines(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    root_schema: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    depth: usize,
    width: u16,
) {
    let Some(branches) = branch_choices(root_schema, schema) else {
        return;
    };
    let active = active_branch_label(branches.as_slice(), value);
    let active_id = active_branch_id(branches.as_slice(), value);
    let also_matches = branches
        .iter()
        .filter(|branch| {
            branch.id != active_id && schema_matches(root_schema, &branch.schema, value)
        })
        .map(|branch| branch.label.as_str())
        .collect::<Vec<_>>();
    let suffix = if also_matches.is_empty() {
        String::new()
    } else {
        format!(
            "   {}",
            i18n.text_args(
                "plugin-workbench-editor-also-matches",
                &agena_tui::fl_args!("matches" => clean(also_matches.join(", "))),
            )
        )
    };
    lines.push(Line::from(fixed_columns(
        &[
            (
                format!(
                    "{}{}",
                    "  ".repeat(depth),
                    i18n.text("plugin-workbench-editor-shape")
                )
                .as_str(),
                18,
            ),
            (format!("[ {active} v ]{suffix}").as_str(), 72),
        ],
        width,
    )));
}

pub(crate) fn append_type_selector_line(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    schema: &JsonValue,
    value: &JsonValue,
    depth: usize,
) {
    let choices = schema_type_choices(schema);
    if choices.len() <= 1 {
        return;
    }
    let active = json_kind_label(value);
    lines.push(Line::from(format!(
        "{}{}        [ {} v ]",
        "  ".repeat(depth),
        i18n.text("plugin-workbench-config-type"),
        localized_kind_label(i18n, active)
    )));
}

pub(crate) fn append_object_editor_lines(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    object: &JsonMap<String, JsonValue>,
    depth: usize,
    width: u16,
    remaining: usize,
) {
    let indent = "  ".repeat(depth);
    let editor_label = match schema {
        None => i18n.text("plugin-workbench-editor-generic-object"),
        Some(schema) if schema_is_map_like(root_schema.unwrap_or(schema), schema) => {
            i18n.text("plugin-workbench-editor-map")
        }
        Some(_) => i18n.text("plugin-workbench-editor-object"),
    };
    lines.push(Line::from(format!("{indent}{editor_label}")));
    lines.push(Line::from(Span::styled(
        fixed_columns(
            &[
                (
                    format!("{indent}{}", i18n.text("plugin-workbench-column-field")).as_str(),
                    28,
                ),
                (i18n.text("plugin-workbench-config-type").as_str(), 14),
                (i18n.text("plugin-workbench-config-value").as_str(), 46),
                (i18n.text("plugin-workbench-config-state").as_str(), 14),
            ],
            width,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    let keys = ordered_object_keys(schema, object);
    if keys.is_empty() {
        lines.push(Line::from(format!(
            "{indent}{}",
            i18n.text("plugin-workbench-editor-no-fields")
        )));
    }
    for key in keys {
        let child = object.get(key.as_str()).unwrap_or(&JsonValue::Null);
        let child_schema = schema.and_then(|schema| {
            root_schema.and_then(|root| object_property_schema(root, schema, key.as_str()))
        });
        let kind = child_schema
            .as_ref()
            .map(schema_kind_label)
            .unwrap_or_else(|| json_kind_label(child).to_owned());
        let kind = localized_kind_label(i18n, kind.as_str());
        let state = object_field_state(
            root_schema,
            schema,
            key.as_str(),
            object.contains_key(key.as_str()),
        );
        let state = localized_object_field_state(i18n, state.as_str());
        lines.push(Line::from(fixed_columns(
            &[
                (
                    format!("{indent}{}", title_from_key(key.as_str())).as_str(),
                    28,
                ),
                (kind.as_str(), 14),
                (localized_structured_preview(i18n, child).as_str(), 46),
                (state.as_str(), 14),
            ],
            width,
        )));
        if depth < 2 && matches!(child, JsonValue::Object(_) | JsonValue::Array(_)) && remaining > 1
        {
            append_schema_editor_lines(
                lines,
                i18n,
                root_schema,
                child_schema.as_ref(),
                child,
                title_from_key(key.as_str()).as_str(),
                depth + 1,
                width,
                remaining.saturating_sub(1),
            );
        }
    }
    lines.push(Line::from(format!(
        "{indent}{}",
        i18n.text("plugin-workbench-editor-object-action-help")
    )));
}

pub(crate) fn append_array_editor_lines(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    items: &[JsonValue],
    depth: usize,
    width: u16,
    remaining: usize,
) {
    let indent = "  ".repeat(depth);
    let tuple = schema.is_some_and(|schema| schema_prefix_item_count(schema) > 0);
    let object_items = items.iter().any(JsonValue::is_object);
    let title = if tuple {
        i18n.text("plugin-workbench-editor-tuple")
    } else if object_items {
        i18n.text("plugin-workbench-editor-object-array")
    } else {
        i18n.text("plugin-workbench-editor-primitive-array")
    };
    lines.push(Line::from(format!("{indent}{title}")));
    if object_items {
        append_object_array_table(lines, i18n, root_schema, schema, items, depth, width);
    } else {
        lines.push(Line::from(Span::styled(
            fixed_columns(
                &[
                    (
                        format!("{indent}{}", i18n.text("plugin-workbench-editor-index")).as_str(),
                        10,
                    ),
                    (i18n.text("plugin-workbench-config-type").as_str(), 14),
                    (i18n.text("plugin-workbench-editor-preview").as_str(), 56),
                ],
                width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, item) in items.iter().enumerate() {
            let item_schema = schema.and_then(|schema| {
                root_schema.and_then(|root| array_item_schema(root, schema, index))
            });
            let item_kind = item_schema
                .as_ref()
                .map(schema_kind_label)
                .unwrap_or_else(|| json_kind_label(item).to_owned());
            let item_kind = localized_kind_label(i18n, item_kind.as_str());
            let item_preview = localized_structured_preview(i18n, item);
            lines.push(Line::from(fixed_columns(
                &[
                    (format!("{indent}{index}").as_str(), 10),
                    (item_kind.as_str(), 14),
                    (item_preview.as_str(), 56),
                ],
                width,
            )));
            if depth < 2
                && matches!(item, JsonValue::Object(_) | JsonValue::Array(_))
                && remaining > 1
            {
                append_schema_editor_lines(
                    lines,
                    i18n,
                    root_schema,
                    item_schema.as_ref(),
                    item,
                    i18n.text_args(
                        "plugin-workbench-editor-item",
                        &agena_tui::fl_args!("index" => index),
                    )
                    .as_str(),
                    depth + 1,
                    width,
                    remaining.saturating_sub(1),
                );
            }
        }
    }
    if items.is_empty() {
        lines.push(Line::from(format!(
            "{indent}{}",
            i18n.text("plugin-workbench-editor-no-items")
        )));
    }
    lines.push(Line::from(format!(
        "{indent}{}",
        i18n.text("plugin-workbench-editor-array-action-help")
    )));
}

pub(crate) fn append_object_array_table(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    items: &[JsonValue],
    depth: usize,
    width: u16,
) {
    let indent = "  ".repeat(depth);
    let item_schema =
        schema.and_then(|schema| root_schema.and_then(|root| array_item_schema(root, schema, 0)));
    let columns = object_array_columns(item_schema.as_ref(), items);
    let mut header = vec![(
        format!("{indent}{}", i18n.text("plugin-workbench-editor-index")),
        8,
    )];
    for column in &columns {
        header.push((title_from_key(column), 18));
    }
    let header_refs = header
        .iter()
        .map(|(label, size)| (label.as_str(), *size))
        .collect::<Vec<_>>();
    lines.push(Line::from(Span::styled(
        fixed_columns(header_refs.as_slice(), width),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (index, item) in items.iter().enumerate() {
        let mut row = vec![(format!("{indent}{index}"), 8)];
        if let Some(object) = item.as_object() {
            for column in &columns {
                row.push((
                    object
                        .get(column)
                        .map(|value| localized_structured_preview(i18n, value))
                        .unwrap_or_else(|| i18n.text("plugin-workbench-field-state-missing")),
                    18,
                ));
            }
        } else {
            row.push((localized_structured_preview(i18n, item), 18));
        }
        let row_refs = row
            .iter()
            .map(|(label, size)| (label.as_str(), *size))
            .collect::<Vec<_>>();
        lines.push(Line::from(fixed_columns(row_refs.as_slice(), width)));
    }
    if root_schema.is_some() && item_schema.is_some() {
        lines.push(Line::from(format!(
            "{indent}{}",
            i18n.text("plugin-workbench-editor-object-array-help")
        )));
    }
}

pub(crate) fn append_string_editor_lines(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    schema: Option<&JsonValue>,
    title: &str,
    text: &str,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let format_suffix = schema
        .and_then(|schema| schema_first_string_keyword(schema, "format"))
        .map(|format| {
            format!(
                "   {}",
                i18n.text_args(
                    "plugin-workbench-editor-format",
                    &agena_tui::fl_args!("format" => format.to_owned()),
                )
            )
        })
        .unwrap_or_default();
    if schema.is_some_and(schema_string_is_multiline) || text.contains('\n') {
        lines.push(Line::from(format!("{indent}{title}")));
        lines.push(Line::from(format!("{indent}+{}", "-".repeat(44))));
        for line in text.lines().take(6) {
            lines.push(Line::from(format!("{indent}| {}", clean(line))));
        }
        if text.is_empty() {
            lines.push(Line::from(format!("{indent}| ")));
        }
        lines.push(Line::from(format!("{indent}+{}", "-".repeat(44))));
    } else {
        lines.push(Line::from(format!(
            "{indent}{title}        [ {} ]{}",
            clean(truncate_text(text, 48)),
            format_suffix
        )));
    }
    if let Some(examples) = schema.and_then(schema_examples) {
        let suggestions = examples
            .iter()
            .take(3)
            .map(preview_value)
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::from(format!(
            "{indent}{}        [ {} v ]",
            i18n.text("plugin-workbench-editor-suggestions"),
            clean(suggestions)
        )));
    }
}

pub(crate) fn append_number_editor_lines(
    lines: &mut Vec<Line<'static>>,
    _i18n: &I18n,
    schema: Option<&JsonValue>,
    title: &str,
    number: &JsonNumber,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let constraints = schema
        .map(number_constraint_summary)
        .filter(|summary| !summary.is_empty())
        .unwrap_or_default();
    lines.push(Line::from(format!(
        "{indent}{title}        [ {} ]{}",
        number, constraints
    )));
}

pub(crate) fn append_null_editor_lines(
    lines: &mut Vec<Line<'static>>,
    i18n: &I18n,
    schema: Option<&JsonValue>,
    title: &str,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let choices = schema.map(schema_type_choices).unwrap_or_default();
    if choices.len() > 1 {
        lines.push(Line::from(format!("{indent}{title}")));
        lines.push(Line::from(format!(
            "{indent}{}        [ null v ]",
            i18n.text("plugin-workbench-config-type")
        )));
    } else {
        lines.push(Line::from(format!("{indent}{title}        [ null ]")));
    }
}
