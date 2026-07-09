use std::borrow::ToOwned;
use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{
    compare_json_numbers, ordered_schema_properties, resolve_schema_value, string_literals,
    top_level_discriminated_variants, top_level_union_variants,
};

const MAX_VARIANT_EXAMPLES: usize = 6;

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

pub fn schema_example_texts(schema: &serde_json::Value) -> Vec<String> {
    if let Some(variants) = top_level_discriminated_variants(schema) {
        return variants
            .into_iter()
            .take(MAX_VARIANT_EXAMPLES)
            .filter_map(|variant| {
                let label = schema_description_text(&variant.schema)
                    .map(|description| format!(": {description}"))
                    .unwrap_or_default();
                let mut object = match schema_example_object(&variant.schema) {
                    Some(serde_json::Value::Object(object)) => object,
                    _ => serde_json::Map::new(),
                };
                object.insert(
                    variant.field,
                    serde_json::Value::String(variant.value.clone()),
                );
                let value = serde_json::Value::Object(object);
                schema_compact_json_text(schema, &variant.schema, &value)
                    .or_else(|| serde_json::to_string(&value).ok())
                    .map(|text| format!("{}{}: {}", variant.value, label, text))
            })
            .collect();
    }

    if let Some(variants) = top_level_union_variants(schema) {
        let mut examples = Vec::new();
        let mut seen = BTreeSet::new();
        let mut saw_non_null = false;
        for (index, variant) in variants.iter().take(MAX_VARIANT_EXAMPLES).enumerate() {
            if let Some(value) = schema_example_value("value", variant) {
                saw_non_null |= !value.is_null();
                let label = schema_type_label(schema, variant);
                let description = schema_description_text(variant)
                    .map(|description| format!(": {description}"))
                    .unwrap_or_default();
                let Some(text) = schema_compact_json_text(schema, variant, &value)
                    .or_else(|| serde_json::to_string(&value).ok())
                else {
                    continue;
                };
                let labeled_text =
                    format!("Variant {} <{}>{}: {}", index + 1, label, description, text);
                if seen.insert(labeled_text.clone()) {
                    examples.push(labeled_text);
                }
            }
        }
        if saw_non_null {
            examples.retain(|text| !text.ends_with(": null"));
        }
        if !examples.is_empty() {
            return examples;
        }
    }

    if let Some(examples) = schema_nested_example_texts(schema) {
        return examples;
    }

    schema_example_object(schema)
        .and_then(|value| {
            schema_compact_json_text(schema, schema, &value)
                .or_else(|| serde_json::to_string(&value).ok())
        })
        .into_iter()
        .collect()
}

fn schema_nested_example_texts(schema: &serde_json::Value) -> Option<Vec<String>> {
    let schema = resolve_schema_value(schema, schema);
    let object = schema.as_object()?;

    if object.get("type").and_then(serde_json::Value::as_str) == Some("array") {
        let item_schema = object.get("items")?;
        let variants = schema_example_variants(item_schema, "item")?;
        let item_count = object
            .get("minItems")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value.clamp(1, 3) as usize)
            .unwrap_or(1);
        let mut texts = Vec::new();
        for (label, item) in variants {
            let value = serde_json::Value::Array(vec![item; item_count]);
            let text = schema_compact_json_text(schema, item_schema, &value)
                .or_else(|| serde_json::to_string(&value).ok())?;
            texts.push(format!("Item {label}: {text}"));
        }
        return (!texts.is_empty()).then_some(texts);
    }

    let base = schema_example_object(schema)?;
    let mut texts = Vec::new();

    for (name, property) in ordered_schema_properties(schema, schema)? {
        let property = resolve_schema_value(schema, property);
        let Some(variants) = schema_example_variants(property, name) else {
            continue;
        };
        for (label, value) in variants {
            let mut object = match base.clone() {
                serde_json::Value::Object(object) => object,
                _ => return None,
            };
            object.insert(name.clone(), value);
            let value = serde_json::Value::Object(object);
            let text = schema_compact_json_text(schema, property, &value)
                .or_else(|| serde_json::to_string(&value).ok())?;
            texts.push(format!("{name}.{label}: {text}"));
            if texts.len() >= MAX_VARIANT_EXAMPLES {
                return Some(texts);
            }
        }
    }

    (!texts.is_empty()).then_some(texts)
}

fn schema_example_variants(
    schema: &serde_json::Value,
    field_name: &str,
) -> Option<Vec<(String, serde_json::Value)>> {
    if let Some(variants) = top_level_discriminated_variants(schema) {
        return Some(
            variants
                .into_iter()
                .take(MAX_VARIANT_EXAMPLES)
                .filter_map(|variant| {
                    let mut object = match schema_example_object(&variant.schema) {
                        Some(serde_json::Value::Object(object)) => object,
                        _ => serde_json::Map::new(),
                    };
                    object.insert(
                        variant.field,
                        serde_json::Value::String(variant.value.clone()),
                    );
                    Some((variant.value.clone(), serde_json::Value::Object(object)))
                })
                .collect(),
        );
    }

    if let Some(variants) = top_level_union_variants(schema) {
        let mut examples = Vec::new();
        let mut seen = BTreeSet::new();
        let mut saw_non_null = false;
        for (index, variant) in variants.iter().take(MAX_VARIANT_EXAMPLES).enumerate() {
            if let Some(value) = schema_example_value(field_name, variant) {
                saw_non_null |= !value.is_null();
                let label = schema_type_label(schema, variant);
                let description = schema_description_text(variant)
                    .map(|description| format!(": {description}"))
                    .unwrap_or_default();
                let Some(text) = schema_compact_json_text(schema, variant, &value)
                    .or_else(|| serde_json::to_string(&value).ok())
                else {
                    continue;
                };
                let labeled_text =
                    format!("Variant {} <{}>{}: {}", index + 1, label, description, text);
                if seen.insert(labeled_text.clone()) {
                    examples.push((labeled_text, value));
                }
            }
        }
        if saw_non_null {
            examples.retain(|(_, value)| !value.is_null());
        }
        if !examples.is_empty() {
            return Some(examples);
        }
    }

    let object = schema.as_object()?;
    if object.get("type").and_then(serde_json::Value::as_str) == Some("array") {
        let item_schema = object.get("items")?;
        let item_variants = schema_example_variants(item_schema, "item")?;
        let item_count = object
            .get("minItems")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value.clamp(1, 3) as usize)
            .unwrap_or(1);
        return Some(
            item_variants
                .into_iter()
                .map(|(label, item)| (label, serde_json::Value::Array(vec![item; item_count])))
                .collect(),
        );
    }

    None
}

fn schema_compact_json_text(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> Option<String> {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => serde_json::to_string(value).ok(),
        serde_json::Value::Array(items) => {
            let schema = resolve_schema_value(root, schema);
            let item_schema = schema.as_object()?.get("items")?;
            let mut rendered = Vec::with_capacity(items.len());
            for item in items {
                rendered.push(schema_compact_json_text(root, item_schema, item)?);
            }
            Some(format!("[{}]", rendered.join(",")))
        }
        serde_json::Value::Object(object) => {
            let schema = resolve_schema_value(root, schema);
            let mut rendered = Vec::new();
            let mut seen = BTreeSet::new();
            if let Some(discriminant_field) = schema
                .as_object()
                .and_then(|object| object.get("x-agena-discriminant-field"))
                .and_then(serde_json::Value::as_str)
                && let Some(value) = object.get(discriminant_field)
            {
                rendered.push(format!(
                    "{}:{}",
                    serde_json::to_string(discriminant_field).ok()?,
                    serde_json::to_string(value).ok()?
                ));
                seen.insert(discriminant_field.to_string());
            }
            if let Some(ordered_properties) = ordered_schema_properties(root, schema) {
                for (name, property_schema) in ordered_properties {
                    if let Some(value) = object.get(name) {
                        let text = schema_compact_json_text(root, property_schema, value)
                            .or_else(|| serde_json::to_string(value).ok())?;
                        rendered.push(format!("{}:{}", serde_json::to_string(name).ok()?, text));
                        seen.insert(name.clone());
                    }
                }
            }
            for (name, value) in object {
                if seen.contains(name) {
                    continue;
                }
                rendered.push(format!(
                    "{}:{}",
                    serde_json::to_string(name).ok()?,
                    serde_json::to_string(value).ok()?
                ));
            }
            Some(format!("{{{}}}", rendered.join(",")))
        }
    }
}

fn schema_example_object(schema: &serde_json::Value) -> Option<serde_json::Value> {
    let object = schema.as_object()?;
    if let Some(example) = schema_first_example_value(object) {
        if example.is_object() {
            return Some(example);
        }
    }
    if let Some(default) = object.get("default") {
        return Some(default.clone());
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
    let mut rendered = serde_json::Map::new();
    for (name, property) in ordered_schema_properties(schema, schema)? {
        if required.contains(name.as_str())
            || property.get("examples").is_some()
            || property.get("const").is_some()
            || property.get("default").is_some()
        {
            if let Some(value) = schema_example_value(name, property) {
                rendered.insert(name.clone(), value);
            }
        }
    }
    Some(serde_json::Value::Object(rendered))
}

fn schema_example_value(field_name: &str, schema: &serde_json::Value) -> Option<serde_json::Value> {
    let object = schema.as_object()?;
    if let Some(example) = schema_first_example_value(object) {
        return Some(example);
    }
    if let Some(default) = object.get("default") {
        return Some(default.clone());
    }
    if let Some(value) = object.get("const") {
        return Some(value.clone());
    }
    if let Some(value) = object
        .get("enum")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
    {
        return Some(value.clone());
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(items) = object.get(key).and_then(serde_json::Value::as_array) {
            let mut primitive_example = None;
            let mut null_example = None;
            for item in items {
                if let Some(value) = schema_example_value(field_name, item) {
                    if value.is_object() {
                        return Some(value);
                    }
                    if value.is_null() {
                        null_example.get_or_insert(value);
                    } else {
                        primitive_example.get_or_insert(value);
                    }
                }
            }
            if let Some(value) = primitive_example {
                return Some(value);
            }
            if let Some(value) = null_example {
                return Some(value);
            }
        }
    }
    match object.get("type").and_then(serde_json::Value::as_str) {
        Some("string") => Some(schema_string_example_value(field_name, object)),
        Some("integer") => Some(schema_numeric_example_value(object, serde_json::json!(1))?),
        Some("number") => Some(schema_numeric_example_value(
            object,
            serde_json::json!(1.0),
        )?),
        Some("boolean") => Some(serde_json::Value::Bool(false)),
        Some("array") => {
            let item_schema = object.get("items")?;
            let item_example = schema_example_value("item", item_schema)?;
            let item_count = object
                .get("minItems")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value.clamp(1, 3) as usize)
                .unwrap_or(1);
            Some(serde_json::Value::Array(vec![item_example; item_count]))
        }
        Some("object") => schema_example_object(schema),
        _ if object.get("properties").is_some() => schema_example_object(schema),
        _ => None,
    }
}

fn schema_string_example_value(field_name: &str, object: &Map<String, Value>) -> Value {
    let text = object
        .get("format")
        .and_then(serde_json::Value::as_str)
        .and_then(schema_format_example_text)
        .unwrap_or_else(|| format!("<{field_name}>"));
    Value::String(text)
}

fn schema_format_example_text(format: &str) -> Option<String> {
    let text = match format {
        "uri" => "https://example.com",
        "uuid" => "550e8400-e29b-41d4-a716-446655440000",
        "email" => "user@example.com",
        "hostname" => "example.com",
        "ipv4" => "127.0.0.1",
        "ipv6" => "2001:db8::1",
        _ => return None,
    };
    Some(text.to_string())
}

fn schema_numeric_example_value(
    object: &Map<String, Value>,
    default: Value,
) -> Option<serde_json::Value> {
    let default_number = default.as_number()?;
    if number_value_satisfies_schema_bounds(default_number, object) {
        return Some(default);
    }
    if default.as_i64().is_some() || default.as_u64().is_some() {
        if let Some(value) = schema_integer_example_value(object) {
            return Some(value);
        }
    }
    if let Some(value) = schema_float_example_value(object) {
        return Some(value);
    }
    Some(default)
}

#[derive(Clone, Copy)]
struct NumericSchemaBound<'a> {
    value: &'a serde_json::Number,
    exclusive: bool,
}

fn schema_numeric_lower_bound(object: &Map<String, Value>) -> Option<NumericSchemaBound<'_>> {
    choose_stricter_lower_bound(
        object.get("minimum").and_then(Value::as_number),
        object.get("exclusiveMinimum").and_then(Value::as_number),
    )
}

fn schema_numeric_upper_bound(object: &Map<String, Value>) -> Option<NumericSchemaBound<'_>> {
    choose_stricter_upper_bound(
        object.get("maximum").and_then(Value::as_number),
        object.get("exclusiveMaximum").and_then(Value::as_number),
    )
}

fn choose_stricter_lower_bound<'a>(
    inclusive: Option<&'a serde_json::Number>,
    exclusive: Option<&'a serde_json::Number>,
) -> Option<NumericSchemaBound<'a>> {
    match (inclusive, exclusive) {
        (Some(inclusive), Some(exclusive)) => {
            match compare_json_numbers(inclusive, exclusive).unwrap_or(Ordering::Equal) {
                Ordering::Less => Some(NumericSchemaBound {
                    value: exclusive,
                    exclusive: true,
                }),
                Ordering::Greater => Some(NumericSchemaBound {
                    value: inclusive,
                    exclusive: false,
                }),
                Ordering::Equal => Some(NumericSchemaBound {
                    value: exclusive,
                    exclusive: true,
                }),
            }
        }
        (Some(inclusive), None) => Some(NumericSchemaBound {
            value: inclusive,
            exclusive: false,
        }),
        (None, Some(exclusive)) => Some(NumericSchemaBound {
            value: exclusive,
            exclusive: true,
        }),
        (None, None) => None,
    }
}

fn choose_stricter_upper_bound<'a>(
    inclusive: Option<&'a serde_json::Number>,
    exclusive: Option<&'a serde_json::Number>,
) -> Option<NumericSchemaBound<'a>> {
    match (inclusive, exclusive) {
        (Some(inclusive), Some(exclusive)) => {
            match compare_json_numbers(inclusive, exclusive).unwrap_or(Ordering::Equal) {
                Ordering::Less => Some(NumericSchemaBound {
                    value: inclusive,
                    exclusive: false,
                }),
                Ordering::Greater => Some(NumericSchemaBound {
                    value: exclusive,
                    exclusive: true,
                }),
                Ordering::Equal => Some(NumericSchemaBound {
                    value: exclusive,
                    exclusive: true,
                }),
            }
        }
        (Some(inclusive), None) => Some(NumericSchemaBound {
            value: inclusive,
            exclusive: false,
        }),
        (None, Some(exclusive)) => Some(NumericSchemaBound {
            value: exclusive,
            exclusive: true,
        }),
        (None, None) => None,
    }
}

fn number_value_satisfies_schema_bounds(
    number: &serde_json::Number,
    object: &Map<String, Value>,
) -> bool {
    if let Some(lower) = schema_numeric_lower_bound(object) {
        let Some(ordering) = compare_json_numbers(number, lower.value) else {
            return false;
        };
        if ordering == Ordering::Less || (lower.exclusive && ordering == Ordering::Equal) {
            return false;
        }
    }
    if let Some(upper) = schema_numeric_upper_bound(object) {
        let Some(ordering) = compare_json_numbers(number, upper.value) else {
            return false;
        };
        if ordering == Ordering::Greater || (upper.exclusive && ordering == Ordering::Equal) {
            return false;
        }
    }
    true
}

fn schema_integer_example_value(object: &Map<String, Value>) -> Option<Value> {
    let lower = schema_numeric_lower_bound(object).and_then(|bound| {
        integer_candidate_from_lower_bound(bound.value.as_f64()?, bound.exclusive)
    });
    let upper = schema_numeric_upper_bound(object).and_then(|bound| {
        integer_candidate_from_upper_bound(bound.value.as_f64()?, bound.exclusive)
    });

    let mut candidate = lower.unwrap_or(1);
    if let Some(upper) = upper {
        if candidate > upper {
            candidate = upper;
        }
    }
    if let Some(lower) = lower {
        if candidate < lower {
            return None;
        }
    }
    let number = serde_json::Number::from(candidate);
    number_value_satisfies_schema_bounds(&number, object).then_some(Value::Number(number))
}

fn integer_candidate_from_lower_bound(value: f64, exclusive: bool) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let candidate = if exclusive {
        value.floor() + 1.0
    } else {
        value.ceil()
    };
    (candidate >= i64::MIN as f64 && candidate <= i64::MAX as f64).then_some(candidate as i64)
}

fn integer_candidate_from_upper_bound(value: f64, exclusive: bool) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let candidate = if exclusive {
        value.ceil() - 1.0
    } else {
        value.floor()
    };
    (candidate >= i64::MIN as f64 && candidate <= i64::MAX as f64).then_some(candidate as i64)
}

fn schema_float_example_value(object: &Map<String, Value>) -> Option<Value> {
    let lower = schema_numeric_lower_bound(object);
    let upper = schema_numeric_upper_bound(object);
    let candidate = match (lower, upper) {
        (Some(lower), Some(upper)) => {
            let lower = lower.value.as_f64()?;
            let upper = upper.value.as_f64()?;
            if !lower.is_finite() || !upper.is_finite() || lower >= upper {
                return None;
            }
            (lower + upper) / 2.0
        }
        (Some(lower), None) => {
            let lower_value = lower.value.as_f64()?;
            if !lower_value.is_finite() {
                return None;
            }
            lower_value + if lower.exclusive { 1.0 } else { 0.0 }
        }
        (None, Some(upper)) => {
            let upper_value = upper.value.as_f64()?;
            if !upper_value.is_finite() {
                return None;
            }
            upper_value - if upper.exclusive { 1.0 } else { 0.0 }
        }
        (None, None) => return None,
    };
    let number = serde_json::Number::from_f64(candidate)?;
    number_value_satisfies_schema_bounds(&number, object).then_some(Value::Number(number))
}

fn schema_first_example_value(object: &Map<String, Value>) -> Option<Value> {
    object
        .get("examples")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .cloned()
}

fn schema_type_label(root: &serde_json::Value, schema: &serde_json::Value) -> String {
    let schema = resolve_schema_value(root, schema);
    let Some(object) = schema.as_object() else {
        return "value".to_string();
    };
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(labels) = object
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                let mut labels = Vec::new();
                for item in items {
                    let label = schema_type_label(root, item);
                    if label != "null" && !labels.contains(&label) {
                        labels.push(label);
                    }
                }
                labels
            })
        {
            if labels.len() == 1 {
                return labels.into_iter().next().unwrap();
            }
            if !labels.is_empty() {
                return labels.join(" | ");
            }
        }
    }
    if let Some(kind) = object.get("type").and_then(serde_json::Value::as_str) {
        if kind == "array" {
            let item_label = object
                .get("items")
                .map(|item| schema_type_label(root, item))
                .unwrap_or_else(|| "value".to_string());
            return format!("array<{item_label}>");
        }
        if kind == "null" {
            return "null".to_string();
        }
        return kind.to_string();
    }
    if let Some(kinds) = object.get("type").and_then(serde_json::Value::as_array) {
        let mut labels = Vec::new();
        for kind in kinds.iter().filter_map(serde_json::Value::as_str) {
            if kind == "null" {
                continue;
            }
            let label = if kind == "array" {
                let item_label = object
                    .get("items")
                    .map(|item| schema_type_label(root, item))
                    .unwrap_or_else(|| "value".to_string());
                format!("array<{item_label}>")
            } else {
                kind.to_string()
            };
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        if labels.len() == 1 {
            return labels.into_iter().next().unwrap();
        }
        if !labels.is_empty() {
            return labels.join(" | ");
        }
    }
    if object.get("properties").is_some() {
        return "object".to_string();
    }
    if object.get("enum").is_some() || object.get("const").is_some() {
        return "string".to_string();
    }
    "value".to_string()
}

fn compact_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => format!("`{text}`"),
        other => other.to_string(),
    }
}

fn schema_constraint_labels(schema: &serde_json::Value) -> Option<Vec<String>> {
    let object = schema.as_object()?;
    if let Some(labels) = object
        .iter()
        .find_map(|(key, value)| match key.as_str() {
            "oneOf" | "anyOf" | "allOf" => value.as_array().map(|items| {
                let mut labels = Vec::new();
                for item in items {
                    if let Some(branch_labels) =
                        schema_constraint_labels(resolve_schema_value(schema, item))
                    {
                        for label in branch_labels {
                            if !labels.contains(&label) {
                                labels.push(label);
                            }
                        }
                    }
                }
                labels
            }),
            _ => None,
        })
        .filter(|labels| !labels.is_empty())
    {
        return Some(labels);
    }
    let mut labels = Vec::new();
    if let Some(value) = object.get("minimum").filter(|value| value.is_number()) {
        labels.push(format!("minimum={}", compact_json_value(value)));
    }
    if let Some(value) = object.get("maximum").filter(|value| value.is_number()) {
        labels.push(format!("maximum={}", compact_json_value(value)));
    }
    if let Some(value) = object
        .get("exclusiveMinimum")
        .filter(|value| value.is_number())
    {
        labels.push(format!("exclusive_minimum={}", compact_json_value(value)));
    }
    if let Some(value) = object
        .get("exclusiveMaximum")
        .filter(|value| value.is_number())
    {
        labels.push(format!("exclusive_maximum={}", compact_json_value(value)));
    }
    if let Some(value) = object.get("minItems").and_then(serde_json::Value::as_u64) {
        labels.push(format!("min_items={value}"));
    }
    if let Some(value) = object.get("maxItems").and_then(serde_json::Value::as_u64) {
        labels.push(format!("max_items={value}"));
    }
    if let Some(value) = object.get("minLength").and_then(serde_json::Value::as_u64) {
        labels.push(format!("min_length={value}"));
    }
    if let Some(value) = object.get("maxLength").and_then(serde_json::Value::as_u64) {
        labels.push(format!("max_length={value}"));
    }
    if let Some(value) = object
        .get("minProperties")
        .and_then(serde_json::Value::as_u64)
    {
        labels.push(format!("min_properties={value}"));
    }
    if let Some(value) = object
        .get("maxProperties")
        .and_then(serde_json::Value::as_u64)
    {
        labels.push(format!("max_properties={value}"));
    }
    if let Some(value) = object.get("pattern").and_then(serde_json::Value::as_str) {
        labels.push(format!("pattern={value}"));
    }
    if let Some(value) = object.get("format").and_then(serde_json::Value::as_str) {
        labels.push(format!("format={value}"));
    }
    Some(labels)
}

fn schema_array_item_constraint_labels(schema: &serde_json::Value) -> Option<Vec<String>> {
    let object = schema.as_object()?;
    if let Some(labels) = object
        .iter()
        .find_map(|(key, value)| match key.as_str() {
            "oneOf" | "anyOf" | "allOf" => value.as_array().map(|items| {
                let mut labels = Vec::new();
                for item in items {
                    if let Some(branch_labels) =
                        schema_array_item_constraint_labels(resolve_schema_value(schema, item))
                    {
                        for label in branch_labels {
                            if !labels.contains(&label) {
                                labels.push(label);
                            }
                        }
                    }
                }
                labels
            }),
            _ => None,
        })
        .filter(|labels| !labels.is_empty())
    {
        return Some(labels);
    }
    let item_schema = object.get("items")?.as_object()?;
    let mut labels = Vec::new();
    if let Some(value) = item_schema.get("minimum").filter(|value| value.is_number()) {
        labels.push(format!("item_minimum={}", compact_json_value(value)));
    }
    if let Some(value) = item_schema.get("maximum").filter(|value| value.is_number()) {
        labels.push(format!("item_maximum={}", compact_json_value(value)));
    }
    if let Some(value) = item_schema
        .get("exclusiveMinimum")
        .filter(|value| value.is_number())
    {
        labels.push(format!(
            "item_exclusive_minimum={}",
            compact_json_value(value)
        ));
    }
    if let Some(value) = item_schema
        .get("exclusiveMaximum")
        .filter(|value| value.is_number())
    {
        labels.push(format!(
            "item_exclusive_maximum={}",
            compact_json_value(value)
        ));
    }
    if let Some(value) = item_schema
        .get("minLength")
        .and_then(serde_json::Value::as_u64)
    {
        labels.push(format!("item_min_length={value}"));
    }
    if let Some(value) = item_schema
        .get("maxLength")
        .and_then(serde_json::Value::as_u64)
    {
        labels.push(format!("item_max_length={value}"));
    }
    if let Some(value) = item_schema
        .get("minProperties")
        .and_then(serde_json::Value::as_u64)
    {
        labels.push(format!("item_min_properties={value}"));
    }
    if let Some(value) = item_schema
        .get("maxProperties")
        .and_then(serde_json::Value::as_u64)
    {
        labels.push(format!("item_max_properties={value}"));
    }
    if let Some(value) = item_schema
        .get("pattern")
        .and_then(serde_json::Value::as_str)
    {
        labels.push(format!("item_pattern={value}"));
    }
    if let Some(value) = item_schema
        .get("format")
        .and_then(serde_json::Value::as_str)
    {
        labels.push(format!("item_format={value}"));
    }
    if let Some(values) = string_literals(&Value::Object(item_schema.clone())) {
        let joined = values.into_iter().collect::<Vec<_>>().join(" | ");
        labels.push(format!("item_values={joined}"));
    }
    Some(labels)
}

fn schema_description_text(schema: &serde_json::Value) -> Option<&str> {
    schema
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn schema_aliases(schema: &serde_json::Value) -> Option<Vec<String>> {
    schema
        .get("x-agena-aliases")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
}

fn schema_property_input_keys(name: &str, property_schema: &Value) -> Vec<String> {
    let parse_name = property_schema
        .get("x-agena-parse-name")
        .and_then(Value::as_str)
        .unwrap_or(name);
    let mut seen = BTreeSet::new();
    let mut keys = Vec::new();
    for key in std::iter::once(parse_name.to_string())
        .chain(std::iter::once(name.to_string()))
        .chain(schema_aliases(property_schema).unwrap_or_default())
    {
        if seen.insert(key.clone()) {
            keys.push(key);
        }
    }
    keys
}

pub fn flattened_input_keys_for_parse_path(schema: &Value, path: &str) -> Vec<String> {
    let schema = resolve_schema_value(schema, schema);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };

    let head_end = path.find('.').unwrap_or(path.len());
    let (head, tail) = path.split_at(head_end);
    let mut base = head;
    let mut suffix = String::new();
    while let Some(stripped) = base.strip_suffix("[]") {
        base = stripped;
        suffix.push_str("[]");
    }

    for (name, property_schema) in properties {
        let property_schema = resolve_schema_value(schema, property_schema);
        let keys = schema_property_input_keys(name, property_schema);
        if keys.iter().any(|candidate| candidate == base) {
            return keys
                .into_iter()
                .map(|key| format!("{key}{suffix}{tail}"))
                .collect();
        }
    }
    Vec::new()
}

pub fn resolve_input_constraint_path(schema: &Value, path: &str) -> String {
    let schema = resolve_schema_value(schema, schema);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return path.to_string();
    };

    let head_end = path.find('.').unwrap_or(path.len());
    let (head, tail) = path.split_at(head_end);
    let mut base = head;
    let mut suffix = String::new();
    while let Some(stripped) = base.strip_suffix("[]") {
        base = stripped;
        suffix.push_str("[]");
    }

    for (name, property_schema) in properties {
        let property_schema = resolve_schema_value(schema, property_schema);
        let keys = schema_property_input_keys(name, property_schema);
        if let Some(parse_name) = keys.first()
            && keys.iter().any(|candidate| candidate == base)
        {
            return format!("{parse_name}{suffix}{tail}");
        }
    }
    path.to_string()
}

pub fn normalize_flattened_input_object(input: &mut Value, schema: &Value) {
    let Some(object) = input.as_object_mut() else {
        return;
    };
    let schema = resolve_schema_value(schema, schema);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };

    for (name, property_schema) in properties {
        let property_schema = resolve_schema_value(schema, property_schema);
        let keys = schema_property_input_keys(name, property_schema);
        let parse_name = keys.first().cloned().unwrap_or_else(|| name.clone());
        let candidate_keys = keys
            .iter()
            .filter(|candidate| candidate.as_str() != parse_name.as_str())
            .collect::<Vec<_>>();

        if !object.contains_key(parse_name.as_str()) {
            let mut matched_alias = None;
            for candidate in &candidate_keys {
                if object.contains_key(candidate.as_str()) {
                    matched_alias = Some((*candidate).clone());
                    break;
                }
            }
            if let Some(alias) = matched_alias
                && let Some(value) = object.remove(alias.as_str())
            {
                object.insert(parse_name.to_string(), value);
            }
        } else {
            for candidate in &candidate_keys {
                object.remove(candidate.as_str());
            }
        }

        if !object.contains_key(parse_name.as_str())
            && let Some(default) = property_schema.get("default")
        {
            object.insert(parse_name.to_string(), default.clone());
        }
    }
}

fn schema_relations(schema: &serde_json::Value) -> Option<Vec<String>> {
    schema
        .get("x-agena-relations")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
}

fn append_schema_relations(schema: &serde_json::Value, lines: &mut Vec<String>) {
    let Some(relations) = schema_relations(schema) else {
        return;
    };
    if lines.is_empty() {
        lines.push("Relations:".to_string());
    } else {
        lines.push(String::new());
        lines.push("Relations:".to_string());
    }
    lines.extend(
        relations
            .into_iter()
            .map(|relation| format!("- {relation}")),
    );
}
