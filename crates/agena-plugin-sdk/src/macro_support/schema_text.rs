use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::schema_examples::{
    append_schema_relations, compact_json_value, schema_aliases,
    schema_array_item_constraint_labels, schema_compact_json_text, schema_constraint_labels,
    schema_description_text, schema_example_value, schema_type_label,
};
use super::{
    ordered_schema_properties, resolve_schema_value, string_literals,
    top_level_discriminated_variants, top_level_union_variants,
};

pub fn schema_usage_text(schema: &serde_json::Value) -> Option<String> {
    if let Some(text) = append_schema_discriminated_variants(schema, schema, "Actions:", "") {
        return Some(text);
    }

    if let Some(text) = append_schema_union_variants(schema, schema, "Variants:", "") {
        return Some(text);
    }

    let mut lines = Vec::new();
    render_schema_arguments(schema, schema, "", &mut lines);
    append_schema_relations(schema, &mut lines);
    (!lines.is_empty()).then(|| format!("Arguments:\n{}", lines.join("\n")))
}

fn append_schema_discriminated_variants(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    heading: &str,
    args_prefix: &str,
) -> Option<String> {
    let variants = top_level_discriminated_variants(schema)?;
    let mut lines = vec![heading.to_string()];
    for variant in &variants {
        let alias_label = schema_aliases(&variant.schema)
            .filter(|aliases| !aliases.is_empty())
            .map(|aliases| format!(" (aliases: {})", aliases.join(", ")))
            .unwrap_or_default();
        let summary = schema_description_text(&variant.schema)
            .map(|description| format!(": {description}"))
            .unwrap_or_default();
        lines.push(format!("- {}{}{}", variant.value, alias_label, summary));
    }
    for variant in variants {
        let mut argument_lines = Vec::new();
        render_schema_arguments(root, &variant.schema, args_prefix, &mut argument_lines);
        append_schema_relations(&variant.schema, &mut argument_lines);
        if !argument_lines.is_empty() {
            lines.push(String::new());
            lines.push(format!("Arguments for `{}`:", variant.value));
            lines.extend(argument_lines);
        }
    }
    append_schema_relations(schema, &mut lines);
    Some(lines.join("\n"))
}

fn append_schema_union_variants(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    heading: &str,
    args_prefix: &str,
) -> Option<String> {
    let variants = top_level_union_variants(schema)?;
    let mut lines = vec![heading.to_string()];
    for (index, variant) in variants.iter().enumerate() {
        let label = schema_type_label(root, variant);
        let summary = schema_description_text(variant)
            .map(|description| format!(": {description}"))
            .unwrap_or_default();
        lines.push(format!("- Variant {} <{}>{}", index + 1, label, summary));
    }
    for (index, variant) in variants.iter().enumerate() {
        let mut argument_lines = Vec::new();
        render_schema_arguments(root, variant, args_prefix, &mut argument_lines);
        append_schema_relations(variant, &mut argument_lines);
        if !argument_lines.is_empty() {
            lines.push(String::new());
            let label = if args_prefix.is_empty() {
                format!("variant {}", index + 1)
            } else {
                format!("{args_prefix} variant {}", index + 1)
            };
            lines.push(format!("Arguments for `{label}`:"));
            lines.extend(argument_lines);
        }
    }
    append_schema_relations(schema, &mut lines);
    Some(lines.join("\n"))
}

fn render_schema_arguments(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    prefix: &str,
    lines: &mut Vec<String>,
) {
    let schema = resolve_schema_value(root, schema);
    let Some(object) = schema.as_object() else {
        return;
    };
    if object
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .is_none()
    {
        return;
    }
    let required = object
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();

    let mut ordered_properties = ordered_schema_properties(root, schema).unwrap_or_default();
    ordered_properties.sort_by(|(left_name, left_property), (right_name, right_property)| {
        let left_required = required.contains(left_name.as_str());
        let right_required = required.contains(right_name.as_str());
        right_required
            .cmp(&left_required)
            .then_with(|| {
                let left_order = super::schema_order_key(left_property);
                let right_order = super::schema_order_key(right_property);
                match (left_order, right_order) {
                    (Some(left_order), Some(right_order)) => left_order.cmp(&right_order),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => left_name.cmp(right_name),
                }
            })
            .then_with(|| left_name.cmp(right_name))
    });

    for (name, property) in ordered_properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let required_label = if required.contains(name.as_str()) {
            "required"
        } else {
            "optional"
        };
        let type_label = schema_type_label(root, property);
        let constraint_label = schema_constraint_labels(property)
            .filter(|labels| !labels.is_empty())
            .map(|labels| format!(", {}", labels.join(", ")))
            .unwrap_or_default();
        let item_constraint_label = schema_array_item_constraint_labels(property)
            .filter(|labels| !labels.is_empty())
            .map(|labels| format!(", {}", labels.join(", ")))
            .unwrap_or_default();
        let default_label = property
            .get("default")
            .map(compact_json_value)
            .map(|value| format!(", default={value}"))
            .unwrap_or_default();
        let alias_label = schema_aliases(property)
            .filter(|aliases| !aliases.is_empty())
            .map(|aliases| format!(", aliases={}", aliases.join(" | ")))
            .unwrap_or_default();
        let enum_label = string_literals(property)
            .map(|values| {
                let joined = values.into_iter().collect::<Vec<_>>().join(" | ");
                format!(", values={joined}")
            })
            .unwrap_or_default();
        let description = schema_description_text(property)
            .map(|text| format!(": {text}"))
            .unwrap_or_default();

        lines.push(format!(
            "- `{path}` <{type_label}, {required_label}{constraint_label}{item_constraint_label}{default_label}{alias_label}{enum_label}>{description}"
        ));

        if let Some(item_schema) = property.get("items") {
            let item_schema = resolve_schema_value(root, item_schema);
            if item_schema
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "object")
                || item_schema.get("properties").is_some()
            {
                render_schema_arguments(root, item_schema, format!("{path}[]").as_str(), lines);
                append_schema_relations(item_schema, lines);
            } else if let Some(text) = append_schema_discriminated_variants(
                root,
                item_schema,
                format!("Item variants for `{path}`:").as_str(),
                format!("{path}[]").as_str(),
            ) {
                lines.push(text);
            } else if let Some(text) = append_schema_union_variants(
                root,
                item_schema,
                format!("Item variants for `{path}`:").as_str(),
                format!("{path}[]").as_str(),
            ) {
                lines.push(text);
            }
        } else if property
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind == "object")
            || property.get("properties").is_some()
        {
            render_schema_arguments(root, property, path.as_str(), lines);
            append_schema_relations(property, lines);
        }

        if let Some(text) = append_schema_discriminated_variants(
            root,
            property,
            format!("Variants for `{path}`:").as_str(),
            path.as_str(),
        ) {
            lines.push(text);
        } else if let Some(text) = append_schema_union_variants(
            root,
            property,
            format!("Variants for `{path}`:").as_str(),
            path.as_str(),
        ) {
            lines.push(text);
        }
    }
}

pub fn command_usage_text(value: &serde_json::Value) -> Option<String> {
    command_usage_shorthand_text(value).or_else(|| serde_json::to_string(value).ok())
}

pub fn command_usage_text_for_schema(
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> Option<String> {
    command_usage_shorthand_text_for_schema(schema, value)
        .or_else(|| schema_compact_json_text(schema, schema, value))
        .or_else(|| command_usage_text(value))
}

pub fn command_usage_text_from_schema(schema: &serde_json::Value) -> Option<String> {
    let example = example_value_from_schema(schema)?;
    let merged = merge_example_with_schema(schema, &example);
    command_usage_text_for_schema(schema, &merged)
}

pub fn example_value_from_schema(schema: &serde_json::Value) -> Option<serde_json::Value> {
    schema_example_value("value", schema)
}

pub fn merge_example_with_schema(
    schema: &serde_json::Value,
    example: &serde_json::Value,
) -> serde_json::Value {
    merge_required_example_values(schema, example)
}

fn command_usage_shorthand_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Some(value.to_string())
        }
        serde_json::Value::String(text) => command_bare_string_usage_text(text),
        serde_json::Value::Object(object) => command_object_usage_text(object),
        serde_json::Value::Array(_) => None,
    }
}

fn command_usage_shorthand_text_for_schema(
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> Option<String> {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Some(value.to_string())
        }
        serde_json::Value::String(text) => command_bare_string_usage_text(text),
        serde_json::Value::Object(object) => command_object_usage_text_for_schema(schema, object),
        serde_json::Value::Array(_) => None,
    }
}

fn command_object_usage_text(object: &Map<String, Value>) -> Option<String> {
    if object.is_empty() {
        return None;
    }

    if object.len() == 1 {
        let (_, value) = object.iter().next()?;
        return command_single_field_usage_text(value);
    }

    let mut rendered = Vec::with_capacity(object.len());
    for (name, value) in object {
        rendered.push(format!("{name}={}", command_key_value_usage_text(value)?));
    }
    Some(rendered.join(" "))
}

fn command_object_usage_text_for_schema(
    schema: &serde_json::Value,
    object: &Map<String, Value>,
) -> Option<String> {
    if object.is_empty() {
        return None;
    }

    if object.len() == 1 {
        let (name, value) = object.iter().next()?;
        let property_schema = ordered_schema_properties(schema, schema)
            .and_then(|properties| {
                properties
                    .into_iter()
                    .find_map(|(property_name, property_schema)| {
                        (property_name == name).then_some(property_schema)
                    })
            })
            .unwrap_or(schema);
        return command_single_field_usage_text_for_schema(schema, property_schema, value);
    }

    let mut rendered = Vec::with_capacity(object.len());
    let mut seen = BTreeSet::new();
    if let Some(ordered_properties) = ordered_schema_properties(schema, schema) {
        for (name, property_schema) in ordered_properties {
            if let Some(value) = object.get(name) {
                rendered.push(format!(
                    "{name}={}",
                    command_key_value_usage_text_for_schema(schema, property_schema, value)?
                ));
                seen.insert(name.clone());
            }
        }
    }
    for (name, value) in object {
        if seen.contains(name) {
            continue;
        }
        rendered.push(format!(
            "{name}={}",
            command_key_value_usage_text_for_schema(schema, schema, value)?
        ));
    }
    Some(rendered.join(" "))
}

fn command_single_field_usage_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Some(value.to_string())
        }
        serde_json::Value::String(text) => command_bare_string_usage_text(text),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => None,
    }
}

fn command_single_field_usage_text_for_schema(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> Option<String> {
    command_single_field_usage_text(value)
        .or_else(|| compact_command_value_text(root, schema, value))
}

fn command_key_value_usage_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Some(value.to_string())
        }
        serde_json::Value::String(text) if command_key_value_string_is_safe(text) => {
            Some(text.to_string())
        }
        serde_json::Value::String(_)
        | serde_json::Value::Object(_)
        | serde_json::Value::Array(_) => None,
    }
}

fn command_key_value_usage_text_for_schema(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> Option<String> {
    command_key_value_usage_text(value).or_else(|| compact_command_value_text(root, schema, value))
}

fn command_bare_string_usage_text(text: &str) -> Option<String> {
    command_string_is_shorthand_safe(text).then(|| text.to_string())
}

fn command_key_value_string_is_safe(text: &str) -> bool {
    command_string_is_shorthand_safe(text) && !text.chars().any(char::is_whitespace)
}

fn command_string_is_shorthand_safe(text: &str) -> bool {
    !text.is_empty()
        && text.trim() == text
        && !text.chars().any(|c| matches!(c, '\n' | '\r' | '\t'))
}

fn compact_command_value_text(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> Option<String> {
    match value {
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            schema_compact_json_text(root, schema, value)
                .or_else(|| serde_json::to_string(value).ok())
                .filter(|text| !text.chars().any(char::is_whitespace))
        }
        _ => None,
    }
}

fn merge_required_example_values(schema: &Value, example: &Value) -> Value {
    let Some(mut merged) = example.as_object().cloned() else {
        return example.clone();
    };
    let required = resolve_schema_value(schema, schema)
        .as_object()
        .and_then(|object| object.get("required"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let Some(ordered_properties) = ordered_schema_properties(schema, schema) else {
        return example.clone();
    };
    for (name, property) in ordered_properties {
        if required.contains(name.as_str()) && !merged.contains_key(name.as_str()) {
            if let Some(value) = schema_example_value(&name, property) {
                merged.insert(name.clone(), value);
            }
        }
    }
    Value::Object(merged)
}
