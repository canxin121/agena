use std::borrow::ToOwned;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::{OnceLock, RwLock};

use schemars::{JsonSchema, schema_for};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::{PluginError, PluginErrorCode, Result, ToolTag};

mod schema_text;

mod validation_paths;

use validation_paths::{
    compare_json_numbers, invalid_json_data_error, invalid_json_syntax_error, json_error_data,
    normalized_name_distance, reject_unknown_object_fields, schema_field_candidates,
    unknown_field_from_error_detail,
};

pub use validation_paths::{
    json_path_present, normalize_nested_input_path, normalize_trim_paths,
    normalize_trim_suffix_path, prefix_input_jsonpath, remove_json_path,
    validate_allowed_values_path, validate_at_least_one_of_paths, validate_conflicts_with_path,
    validate_distinct_trimmed_path, validate_distinct_trimmed_within_path,
    validate_exactly_one_of_paths, validate_exclusive_maximum_path,
    validate_exclusive_minimum_path, validate_forbid_substrings_path, validate_format_path,
    validate_max_chars_path, validate_max_items_path, validate_max_properties_path,
    validate_maximum_path, validate_min_chars_path, validate_min_items_path,
    validate_min_properties_path, validate_minimum_path, validate_non_empty_if_present_paths,
    validate_non_empty_paths, validate_pattern_path, validate_required_unless_present_path,
    validate_requires_path,
};

pub use schema_text::{
    command_usage_text, command_usage_text_for_schema, command_usage_text_from_schema,
    example_value_from_schema, flattened_input_keys_for_parse_path, merge_example_with_schema,
    normalize_flattened_input_object, resolve_input_constraint_path, schema_example_texts,
    schema_usage_text,
};

pub fn json_schema_for<T>() -> Value
where
    T: JsonSchema,
{
    let mut value = serde_json::to_value(schema_for!(T))
        .expect("schemars should always serialize generated schema");
    if let Some(object) = value.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    normalize_schema_json(value)
}

pub fn json_schema_for_default<T>(default: T) -> Value
where
    T: JsonSchema + Serialize,
{
    let mut value = json_schema_for::<T>();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "default".to_string(),
            serde_json::to_value(default).expect("schema default should serialize"),
        );
    }
    value
}

pub fn typed_tool_output<T>(value: T) -> Result<crate::ToolInvokeOutput>
where
    T: Serialize,
{
    let payload =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let output_text = match &payload {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        _ => payload.to_string(),
    };
    Ok(crate::ToolInvokeOutput::from_parts(
        String::new(),
        output_text,
        Some(payload),
        Default::default(),
        Vec::new(),
    ))
}

pub fn dedupe_tool_tags(tags: &mut Vec<ToolTag>) {
    let mut deduped = Vec::with_capacity(tags.len());
    for tag in tags.drain(..) {
        if !deduped.iter().any(|existing| existing == &tag) {
            deduped.push(tag);
        }
    }
    *tags = deduped;
}

pub fn empty_config_schema() -> Value {
    serde_json::json!({
        "title": "Plugin Config",
        "description": "This plugin does not expose plugin-specific runtime configuration.",
        "type": "object",
        "properties": {},
        "additionalProperties": false,
        "default": {}
    })
}

pub fn set_schema_metadata(
    schema: &mut Value,
    pointer: &str,
    title: Option<&str>,
    description: Option<&str>,
) {
    if title.is_none() && description.is_none() {
        return;
    }
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(object) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    if let Some(title) = title {
        object.insert("title".to_string(), Value::String(title.to_owned()));
    }
    if let Some(description) = description {
        object.insert(
            "description".to_string(),
            Value::String(description.to_owned()),
        );
    }
}

pub fn merge_flattened_schema_at_pointer(schema: &mut Value, pointer: &str, overlay: &Value) {
    let Some(root) = schema.as_object_mut() else {
        return;
    };
    let Some(overlay_object) = overlay.as_object() else {
        return;
    };
    if let Some(defs) = overlay_object.get("$defs") {
        let root_defs = root
            .entry("$defs".to_string())
            .or_insert_with(|| Value::Object(Default::default()));
        merge_schema_overlay(root_defs, defs);
    }

    let mut overlay_without_root_meta = overlay.clone();
    if let Some(object) = overlay_without_root_meta.as_object_mut() {
        object.remove("$defs");
        object.remove("title");
        object.remove("description");
    }

    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let target = if resolved_pointer.is_empty() {
        schema
    } else {
        match schema.pointer_mut(resolved_pointer.as_str()) {
            Some(target) => target,
            None => return,
        }
    };
    let overlay_parse_names = flattened_overlay_parse_name_mappings(&overlay_without_root_meta);
    let mut required = schema_required_names(target);
    remove_schema_parse_name_properties(target, &overlay_parse_names);
    required.retain(|name| {
        !overlay_parse_names
            .iter()
            .any(|(_, parse_name)| parse_name == name)
    });
    for name in schema_required_names(&overlay_without_root_meta) {
        if !required.contains(&name) {
            required.push(name);
        }
    }
    merge_schema_overlay(target, &overlay_without_root_meta);
    set_schema_required_names(target, &required);
    promote_nested_composite_properties(target);
}

pub fn merge_schema_overlay_at_pointer(schema: &mut Value, pointer: &str, overlay: &Value) {
    let Some(root) = schema.as_object_mut() else {
        return;
    };
    let Some(overlay_object) = overlay.as_object() else {
        return;
    };
    if let Some(defs) = overlay_object.get("$defs") {
        let root_defs = root
            .entry("$defs".to_string())
            .or_insert_with(|| Value::Object(Default::default()));
        merge_schema_overlay(root_defs, defs);
    }

    let mut overlay_without_root_meta = overlay.clone();
    if let Some(object) = overlay_without_root_meta.as_object_mut() {
        object.remove("$defs");
        object.remove("title");
        object.remove("description");
    }

    let Some(target) = ensure_schema_object_at_pointer(schema, pointer) else {
        return;
    };
    let target = &mut Value::Object(target.clone());
    merge_schema_overlay(target, &overlay_without_root_meta);
    promote_nested_composite_properties(target);
    if let Some(merged) = target.as_object() {
        if let Some(destination) = ensure_schema_object_at_pointer(schema, pointer) {
            *destination = merged.clone();
        }
    }
}

pub fn rename_schema_property(schema: &mut Value, pointer: &str, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let target = if resolved_pointer.is_empty() {
        schema
    } else {
        match schema.pointer_mut(resolved_pointer.as_str()) {
            Some(target) => target,
            None => return,
        }
    };
    let Some(object) = target.as_object_mut() else {
        return;
    };
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut)
        && let Some(property) = properties.remove(from)
    {
        properties.insert(to.to_string(), property);
    }
    if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
        for item in required {
            if item.as_str() == Some(from) {
                *item = Value::String(to.to_string());
            }
        }
    }
}

pub fn set_schema_string_list_metadata(
    schema: &mut Value,
    pointer: &str,
    key: &str,
    values: &[&str],
) {
    if values.is_empty() {
        return;
    }
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(object) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    object.insert(
        key.to_string(),
        Value::Array(
            values
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        ),
    );
}

pub fn set_schema_value_list_metadata(
    schema: &mut Value,
    pointer: &str,
    key: &str,
    values: &[Value],
) {
    if values.is_empty() {
        return;
    }
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(object) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    object.insert(key.to_string(), Value::Array(values.to_vec()));
}

pub fn set_schema_value_metadata(schema: &mut Value, pointer: &str, key: &str, value: Value) {
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(object) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    object.insert(key.to_string(), value);
}

pub fn set_schema_string_metadata(schema: &mut Value, pointer: &str, key: &str, value: &str) {
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(target) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    set_schema_string_metadata_on_object(target, key, value);
}

pub fn set_schema_u64_metadata(schema: &mut Value, pointer: &str, key: &str, value: u64) {
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(target) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    set_schema_u64_metadata_on_object(target, key, value);
}

pub fn set_schema_number_metadata(schema: &mut Value, pointer: &str, key: &str, value: Value) {
    if !value.is_number() {
        return;
    }
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(target) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    set_schema_number_metadata_on_object(target, key, &value);
}

pub fn set_schema_bool_metadata(schema: &mut Value, pointer: &str, key: &str, value: bool) {
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(target) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    target.insert(key.to_string(), Value::Bool(value));
}

pub fn suggest_name_candidates<I, T>(requested: &str, candidates: I, limit: usize) -> Vec<String>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let requested = requested.trim();
    let requested_lower = requested.to_ascii_lowercase();
    let mut ranked: Vec<(usize, String)> = Vec::new();

    for candidate in candidates {
        let name = candidate.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        let score = normalized_name_distance(requested, name);
        if score == 0 {
            continue;
        }
        let name_lower = name.to_ascii_lowercase();
        if score <= 4
            || name_lower.contains(requested_lower.as_str())
            || requested_lower.contains(name_lower.as_str())
        {
            ranked.push((score, name.to_string()));
        }
    }

    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let mut suggestions = Vec::new();
    for (_, name) in ranked {
        if !suggestions.contains(&name) {
            suggestions.push(name);
        }
        if suggestions.len() >= limit {
            break;
        }
    }
    suggestions
}

pub fn unknown_name_message(kind: &str, requested: &str, suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        return format!("unknown {kind} '{requested}'");
    }
    format!(
        "unknown {kind} '{requested}'. Did you mean {}?",
        suggestions
            .iter()
            .map(|item| format!("`{item}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub fn set_schema_minimum_u64_metadata(schema: &mut Value, pointer: &str, key: &str, value: u64) {
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(target) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    set_schema_minimum_u64_metadata_on_object(target, key, value);
}

pub fn set_schema_non_empty_metadata(schema: &mut Value, pointer: &str) {
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let Some(target) = ensure_schema_object_at_pointer(schema, resolved_pointer.as_str()) else {
        return;
    };
    set_schema_non_empty_metadata_on_object(target);
}

pub fn prefix_schema_order_metadata(schema: &mut Value, prefix: &str) {
    if prefix.trim().is_empty() {
        return;
    }
    prefix_schema_order_metadata_on_value(schema, prefix);
}

pub fn remap_invalid_params_paths<T>(result: Result<T>, mappings: &[(&str, &str)]) -> Result<T> {
    result.map_err(|mut err| {
        if err.code != PluginErrorCode::InvalidParams || mappings.is_empty() {
            return err;
        }
        let mut mappings = mappings.to_vec();
        mappings.sort_by(|left, right| {
            right
                .0
                .len()
                .cmp(&left.0.len())
                .then_with(|| left.0.cmp(right.0))
        });
        for (from, to) in mappings {
            err.message = remap_message_path_prefixes(err.message.as_str(), from, to);
            if let Some(data) = err.data.as_mut() {
                remap_error_data_paths(data, from, to);
            }
        }
        err
    })
}

pub fn remap_invalid_params_paths_owned<T>(
    result: Result<T>,
    mappings: &[(String, String)],
) -> Result<T> {
    let borrowed = mappings
        .iter()
        .map(|(from, to)| (from.as_str(), to.as_str()))
        .collect::<Vec<_>>();
    remap_invalid_params_paths(result, &borrowed)
}

pub fn prefixed_input_error_path_mappings(schema: &Value, prefix: &str) -> Vec<(String, String)> {
    let schema = resolve_schema_value(schema, schema);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    properties
        .keys()
        .map(|name| (name.clone(), format!("{prefix}.{name}")))
        .collect()
}

fn set_schema_u64_metadata_on_object(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: u64,
) {
    for composite_key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get_mut(composite_key) {
            for item in items {
                set_schema_u64_metadata_on_value(item, key, value);
            }
            return;
        }
    }
    object.insert(
        key.to_string(),
        Value::Number(serde_json::Number::from(value)),
    );
}

fn set_schema_string_metadata_on_object(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: &str,
) {
    object.insert(key.to_string(), Value::String(value.to_string()));
}

fn set_schema_u64_metadata_on_value(target: &mut Value, key: &str, value: u64) {
    if let Some(object) = target.as_object_mut() {
        set_schema_u64_metadata_on_object(object, key, value);
    }
}

fn set_schema_number_metadata_on_object(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: &Value,
) {
    for composite_key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get_mut(composite_key) {
            for item in items {
                set_schema_number_metadata_on_value(item, key, value);
            }
            return;
        }
    }
    object.insert(key.to_string(), value.clone());
}

fn set_schema_number_metadata_on_value(target: &mut Value, key: &str, value: &Value) {
    if let Some(object) = target.as_object_mut() {
        set_schema_number_metadata_on_object(object, key, value);
    }
}

fn set_schema_minimum_u64_metadata_on_object(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: u64,
) {
    for composite_key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get_mut(composite_key) {
            for item in items {
                set_schema_minimum_u64_metadata_on_value(item, key, value);
            }
            return;
        }
    }
    let should_update = object
        .get(key)
        .and_then(Value::as_u64)
        .is_none_or(|current| current < value);
    if should_update {
        object.insert(
            key.to_string(),
            Value::Number(serde_json::Number::from(value)),
        );
    }
}

fn set_schema_minimum_u64_metadata_on_value(target: &mut Value, key: &str, value: u64) {
    if let Some(object) = target.as_object_mut() {
        set_schema_minimum_u64_metadata_on_object(object, key, value);
    }
}

fn set_schema_non_empty_metadata_on_object(object: &mut serde_json::Map<String, Value>) {
    for composite_key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get_mut(composite_key) {
            for item in items {
                set_schema_non_empty_metadata_on_value(item);
            }
            return;
        }
    }
    if schema_kind_matches(object, "string") {
        set_schema_minimum_u64_metadata_on_object(object, "minLength", 1);
    } else if schema_kind_matches(object, "array") {
        set_schema_minimum_u64_metadata_on_object(object, "minItems", 1);
    } else if schema_kind_matches(object, "object") || object.contains_key("properties") {
        set_schema_minimum_u64_metadata_on_object(object, "minProperties", 1);
    }
}

fn set_schema_non_empty_metadata_on_value(target: &mut Value) {
    if let Some(object) = target.as_object_mut() {
        set_schema_non_empty_metadata_on_object(object);
    }
}

fn prefix_schema_order_metadata_on_value(target: &mut Value, prefix: &str) {
    match target {
        Value::Object(object) => prefix_schema_order_metadata_on_object(object, prefix),
        Value::Array(items) => {
            for item in items {
                prefix_schema_order_metadata_on_value(item, prefix);
            }
        }
        _ => {}
    }
}

fn remap_message_path_prefixes(message: &str, from: &str, to: &str) -> String {
    message
        .replace(format!("`{from}`").as_str(), format!("`{to}`").as_str())
        .replace(format!("`{from}.").as_str(), format!("`{to}.").as_str())
        .replace(format!("`{from}[").as_str(), format!("`{to}[").as_str())
}

fn remap_error_data_paths(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(path)) = object.get_mut("path") {
                *path = remap_prefixed_path(path.as_str(), from, to);
            }
            for nested in object.values_mut() {
                remap_error_data_paths(nested, from, to);
            }
        }
        Value::Array(items) => {
            for item in items {
                remap_error_data_paths(item, from, to);
            }
        }
        _ => {}
    }
}

fn remap_prefixed_path(path: &str, from: &str, to: &str) -> String {
    if path == from {
        return to.to_string();
    }
    if let Some(rest) = path.strip_prefix(from)
        && matches!(rest.chars().next(), Some('.') | Some('['))
    {
        return format!("{to}{rest}");
    }
    path.to_string()
}

fn prefix_schema_order_metadata_on_object(
    object: &mut serde_json::Map<String, Value>,
    prefix: &str,
) {
    if let Some(Value::String(value)) = object.get_mut("x-agena-order") {
        if value.is_empty() {
            *value = prefix.to_string();
        } else {
            *value = format!("{prefix}.{value}");
        }
    }
    for (key, value) in object.iter_mut() {
        if key == "x-agena-order" {
            continue;
        }
        prefix_schema_order_metadata_on_value(value, prefix);
    }
}

fn schema_kind_matches(object: &serde_json::Map<String, Value>, kind: &str) -> bool {
    match object.get("type") {
        Some(Value::String(value)) => value == kind,
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(kind)),
        _ => false,
    }
}

pub fn parse_json_value_str(input: &str) -> Result<Value> {
    serde_json::from_str(input).map_err(|err| invalid_json_syntax_error(err, "string"))
}

pub fn parse_typed_json_value<T>(input: Value) -> Result<T>
where
    T: DeserializeOwned,
{
    match serde_path_to_error::deserialize(input) {
        Ok(parsed) => Ok(parsed),
        Err(err) => {
            let path = err.path().to_string();
            let inner = err.into_inner();
            Err(invalid_json_data_error(inner, "value", Some(path)))
        }
    }
}

pub fn parse_typed_json_value_with_field_suggestions<T>(
    input: Value,
    schema: &Value,
    kind: &str,
) -> Result<T>
where
    T: DeserializeOwned,
{
    reject_unknown_object_fields(&input, schema, kind)?;
    match serde_path_to_error::deserialize(input) {
        Ok(parsed) => Ok(parsed),
        Err(err) => {
            let path = err.path().to_string();
            let inner = err.into_inner();
            if let Some(field) = unknown_field_from_error_detail(&inner.to_string()) {
                let candidates = schema_field_candidates(schema);
                let suggestions = suggest_name_candidates(&field, candidates.iter(), 1);
                if !suggestions.is_empty() {
                    let message = unknown_name_message(kind, &field, &suggestions);
                    return Err(PluginError::invalid_params_with_data(
                        message,
                        json_error_data(inner, "value", Some(path)),
                    ));
                }
            }
            Err(invalid_json_data_error(inner, "value", Some(path)))
        }
    }
}

pub fn parse_typed_json_str<T>(input: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = parse_json_value_str(input)?;
    parse_typed_json_value(value)
}

pub fn parse_defaulted_config<T>(input: Value, invalid: &str) -> Result<T>
where
    T: Default + DeserializeOwned,
{
    if input.is_null() {
        Ok(T::default())
    } else {
        parse_typed_json_value(input).map_err(|err| PluginError::new(format!("{invalid}: {err}")))
    }
}

pub fn store_once<T>(cell: &OnceLock<T>, value: T, already: &str) -> Result<()> {
    cell.set(value)
        .map_err(|_| PluginError::new(already.to_string()))
}

pub fn store_rwlock_option<T>(cell: &RwLock<Option<T>>, value: T, poisoned: &str) -> Result<()> {
    *cell
        .write()
        .map_err(|_| PluginError::new(poisoned.to_string()))? = Some(value);
    Ok(())
}

fn ensure_schema_object_at_pointer<'a>(
    schema: &'a mut Value,
    pointer: &str,
) -> Option<&'a mut serde_json::Map<String, Value>> {
    let target = if pointer.is_empty() {
        schema
    } else {
        schema.pointer_mut(pointer)?
    };
    match target {
        Value::Object(object) => Some(object),
        Value::Bool(true) => {
            *target = Value::Object(serde_json::Map::new());
            target.as_object_mut()
        }
        Value::Bool(false) => {
            *target = serde_json::json!({ "not": {} });
            target.as_object_mut()
        }
        _ => None,
    }
}

fn resolve_schema_pointer(schema: &Value, pointer: &str) -> Option<String> {
    if pointer.is_empty() {
        return Some(String::new());
    }
    let mut current = schema;
    let mut resolved_pointer = String::new();
    for segment in pointer
        .split('/')
        .skip(1)
        .map(unescape_json_pointer_segment)
    {
        while let Some((target, target_pointer)) = resolve_schema_ref(schema, current) {
            current = target;
            resolved_pointer = target_pointer.to_owned();
        }
        current = match current {
            Value::Object(object) => object.get(segment.as_str())?,
            Value::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                items.get(index)?
            }
            _ => return None,
        };
        resolved_pointer.push('/');
        resolved_pointer.push_str(escape_json_pointer_segment(segment.as_str()).as_str());
    }
    while let Some((target, target_pointer)) = resolve_schema_ref(schema, current) {
        current = target;
        resolved_pointer = target_pointer.to_owned();
    }
    Some(resolved_pointer)
}

fn resolve_schema_ref<'a>(root: &'a Value, current: &'a Value) -> Option<(&'a Value, &'a str)> {
    let target_pointer = current
        .get("$ref")
        .and_then(Value::as_str)?
        .strip_prefix('#')?;
    let target = root.pointer(target_pointer)?;
    Some((target, target_pointer))
}

#[derive(Debug, Clone)]
struct DiscriminatedSchemaVariant {
    field: String,
    value: String,
    schema: Value,
}

fn top_level_discriminated_variants(schema: &Value) -> Option<Vec<DiscriminatedSchemaVariant>> {
    let object = schema.as_object()?;
    let variants = ["oneOf", "anyOf", "allOf"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_array))?;
    if variants.len() <= 1 {
        return None;
    }

    let variant_objects = variants
        .iter()
        .map(Value::as_object)
        .collect::<Option<Vec<_>>>()?;
    let discriminant = variant_objects
        .iter()
        .fold(None::<BTreeSet<String>>, |candidates, variant| {
            let fields = variant
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| {
                    properties
                        .iter()
                        .filter_map(|(name, property)| {
                            let literals = string_literals(property)?;
                            (literals.len() == 1).then_some(name.clone())
                        })
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            Some(match candidates {
                Some(existing) => existing
                    .intersection(&fields)
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                None => fields,
            })
        })
        .and_then(|candidates| {
            ["action", "target"]
                .into_iter()
                .find_map(|preferred| candidates.contains(preferred).then_some(preferred))
                .map(ToOwned::to_owned)
                .or_else(|| candidates.into_iter().next())
        })?;

    let mut seen_values = BTreeSet::new();
    let mut expanded = Vec::with_capacity(variant_objects.len());
    for variant in variant_objects {
        let value = variant
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(discriminant.as_str()))
            .and_then(string_literals)
            .and_then(|literals| literals.into_iter().next())?;
        if !seen_values.insert(value.clone()) {
            return None;
        }
        expanded.push(DiscriminatedSchemaVariant {
            field: discriminant.clone(),
            value,
            schema: strip_discriminant_from_variant(variant, discriminant.as_str()),
        });
    }

    Some(expanded)
}

pub fn schema_for_discriminated_variant(schema: &Value, field: &str, value: &str) -> Option<Value> {
    top_level_discriminated_variants(schema)?
        .into_iter()
        .find(|variant| variant.field == field && variant.value == value)
        .map(|variant| variant.schema)
}

fn strip_discriminant_from_variant(variant: &serde_json::Map<String, Value>, field: &str) -> Value {
    let mut stripped = variant.clone();
    if let Some(properties) = stripped
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.remove(field);
    }
    if let Some(required) = stripped.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|item| item.as_str() != Some(field));
        if required.is_empty() {
            stripped.remove("required");
        }
    }
    stripped
        .entry("type".to_string())
        .or_insert_with(|| Value::String("object".to_string()));
    stripped
        .entry("properties".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    stripped.insert(
        "x-agena-discriminant-field".to_string(),
        Value::String(field.to_string()),
    );
    Value::Object(stripped)
}

fn top_level_union_variants(schema: &Value) -> Option<&[Value]> {
    let object = schema.as_object()?;
    ["oneOf", "anyOf", "allOf"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_array))
        .map(Vec::as_slice)
        .filter(|items| !items.is_empty())
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn unescape_json_pointer_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

pub fn normalize_schema_json(value: Value) -> Value {
    normalize_schema_json_value(value, true)
}

fn normalize_schema_json_value(value: Value, remove_schema_metadata: bool) -> Value {
    match value {
        Value::Object(mut object) => {
            if remove_schema_metadata {
                object.remove("$schema");
                object.remove("title");
            }
            let mut cleaned = serde_json::Map::new();
            for (key, value) in object {
                let normalized = match key.as_str() {
                    "properties" => match value {
                        Value::Object(map) => Value::Object(
                            map.into_iter()
                                .map(|(nested_key, nested_value)| {
                                    (nested_key, normalize_schema_json_value(nested_value, true))
                                })
                                .collect(),
                        ),
                        other => normalize_schema_json_value(other, true),
                    },
                    "required" => match value {
                        Value::Array(items) => Value::Array(items),
                        other => normalize_schema_json_value(other, true),
                    },
                    "$defs" | "definitions" | "patternProperties" | "dependentSchemas" => {
                        match value {
                            Value::Object(map) => Value::Object(
                                map.into_iter()
                                    .map(|(nested_key, nested_value)| {
                                        (
                                            nested_key,
                                            normalize_schema_json_value(nested_value, true),
                                        )
                                    })
                                    .collect(),
                            ),
                            other => normalize_schema_json_value(other, true),
                        }
                    }
                    _ => normalize_schema_json_value(value, true),
                };
                cleaned.insert(key, normalized);
            }
            if !cleaned.contains_key("type") && schema_map_is_object_like(&cleaned) {
                cleaned.insert("type".to_string(), Value::String("object".to_string()));
            }
            if cleaned
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "object")
                && !cleaned.contains_key("properties")
            {
                cleaned.insert(
                    "properties".to_string(),
                    Value::Object(serde_json::Map::new()),
                );
            }
            Value::Object(cleaned)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| normalize_schema_json_value(item, true))
                .collect(),
        ),
        other => other,
    }
}

fn merge_schema_overlay(target: &mut Value, overlay: &Value) {
    match (target, overlay) {
        (Value::Object(target_object), Value::Object(overlay_object)) => {
            for (key, overlay_value) in overlay_object {
                match target_object.get_mut(key) {
                    Some(existing) => merge_schema_overlay(existing, overlay_value),
                    None => {
                        target_object.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (target_slot, overlay_value) => {
            *target_slot = overlay_value.clone();
        }
    }
}

fn flattened_overlay_parse_name_mappings(overlay: &Value) -> Vec<(String, String)> {
    overlay
        .as_object()
        .and_then(|object| object.get("properties"))
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter_map(|(name, property)| {
                    let parse_name = property.get("x-agena-parse-name").and_then(Value::as_str)?;
                    (parse_name != name).then(|| (name.clone(), parse_name.to_string()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn schema_required_names(schema: &Value) -> Vec<String> {
    schema
        .as_object()
        .and_then(|object| object.get("required"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn remove_schema_parse_name_properties(target: &mut Value, mappings: &[(String, String)]) {
    let Some(object) = target.as_object_mut() else {
        return;
    };
    let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    for (_, parse_name) in mappings {
        properties.remove(parse_name);
    }
}

fn set_schema_required_names(target: &mut Value, required: &[String]) {
    let Some(object) = target.as_object_mut() else {
        return;
    };
    let mut required = required
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();
    if required.is_empty() {
        object.remove("required");
    } else {
        object.insert(
            "required".to_string(),
            Value::Array(std::mem::take(&mut required)),
        );
    }
}

fn promote_nested_composite_properties(target: &mut Value) {
    let Some(object) = target.as_object_mut() else {
        return;
    };
    let mut collected = Map::new();
    for key in ["oneOf", "anyOf", "allOf"] {
        let Some(Value::Array(items)) = object.get(key) else {
            continue;
        };
        for item in items {
            collect_nested_object_properties(item, &mut collected);
        }
    }
    if collected.is_empty() {
        return;
    }
    let properties = object
        .entry("properties".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let Some(properties) = properties.as_object_mut() else {
        return;
    };
    for (name, property) in collected {
        properties
            .entry(name)
            .and_modify(|existing| *existing = merge_property_schema(existing, &property))
            .or_insert(property);
    }
}

fn collect_nested_object_properties(value: &Value, collected: &mut Map<String, Value>) {
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            collected
                .entry(name.clone())
                .and_modify(|existing| *existing = merge_property_schema(existing, property))
                .or_insert_with(|| property.clone());
        }
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get(key) {
            for item in items {
                collect_nested_object_properties(item, collected);
            }
        }
    }
}

fn merge_property_schema(existing: &Value, next: &Value) -> Value {
    let Some(mut literals) = string_literals(existing) else {
        return existing.clone();
    };
    let Some(next_literals) = string_literals(next) else {
        return existing.clone();
    };
    literals.extend(next_literals);
    serde_json::json!({
        "type": "string",
        "enum": literals.into_iter().collect::<Vec<_>>()
    })
}

fn schema_map_is_object_like(map: &serde_json::Map<String, Value>) -> bool {
    if map
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "object")
    {
        return true;
    }
    if map.contains_key("properties") || map.contains_key("required") {
        return true;
    }
    ["oneOf", "anyOf", "allOf"].into_iter().any(|key| {
        map.get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty() && items.iter().all(schema_value_is_object_like))
    })
}

fn schema_value_is_object_like(value: &Value) -> bool {
    value.as_object().is_some_and(schema_map_is_object_like)
}

pub fn normalize_typed_json_value<T, F>(value: &T, normalize: F) -> Result<T>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce(&mut Value),
{
    let mut json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    normalize(&mut json);
    parse_typed_json_value(json)
}

fn schema_order_key(schema: &serde_json::Value) -> Option<String> {
    schema
        .get("x-agena-order")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .map(ToOwned::to_owned)
        .filter(|value| !value.is_empty())
}

fn ordered_schema_properties<'a>(
    root: &'a serde_json::Value,
    schema: &'a serde_json::Value,
) -> Option<Vec<(&'a String, &'a serde_json::Value)>> {
    let schema = resolve_schema_value(root, schema);
    let properties = schema
        .as_object()?
        .get("properties")
        .and_then(serde_json::Value::as_object)?;
    let mut ordered = properties.iter().collect::<Vec<_>>();
    ordered.sort_by(|(left_name, left_property), (right_name, right_property)| {
        let left_order = schema_order_key(resolve_schema_value(root, left_property));
        let right_order = schema_order_key(resolve_schema_value(root, right_property));
        match (left_order, right_order) {
            (Some(left_order), Some(right_order)) => left_order
                .cmp(&right_order)
                .then_with(|| left_name.cmp(right_name)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left_name.cmp(right_name),
        }
    });
    Some(ordered)
}

fn resolve_schema_value<'a>(
    root: &'a serde_json::Value,
    current: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let mut current = current;
    while let Some((target, _)) = resolve_schema_ref(root, current) {
        current = target;
    }
    current
}

fn string_literals(value: &serde_json::Value) -> Option<BTreeSet<String>> {
    let object = value.as_object()?;
    if let Some(value) = object.get("const").and_then(serde_json::Value::as_str) {
        return Some(BTreeSet::from([value.to_owned()]));
    }
    object
        .get("enum")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
}
