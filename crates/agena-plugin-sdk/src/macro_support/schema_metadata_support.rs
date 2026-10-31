use std::collections::BTreeSet;

use serde_json::Value;

use crate::{PluginErrorKind, Result};

use super::schema_support::{
    escape_json_pointer_segment, resolve_schema_ref, resolve_schema_value,
    unescape_json_pointer_segment,
};
use super::validation_paths::{
    JsonPathSegment, json_path_matches_segments, parse_json_path, value_present,
};
use crate::identity::normalized_name_distance;

// Keep the independent non-empty rule when later metadata batches replace
// ordinary count bounds (for example, a ToolSpec applied to a typed input).
const NON_EMPTY_METADATA: &str = "x-agena-non-empty";

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
    if let Some(merged) = target.as_object()
        && let Some(destination) = ensure_schema_object_at_pointer(schema, pointer)
    {
        *destination = merged.clone();
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

/// Record a field default that the input parser actually injects. The field
/// may be omitted; its type and value constraints still apply when supplied.
pub fn set_schema_input_default_metadata(
    schema: &mut Value,
    pointer: &str,
    field: &str,
    value: Value,
) {
    let Some(parent_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    let field_pointer = format!(
        "{parent_pointer}/properties/{}",
        escape_json_pointer_segment(field)
    );
    let Some(property) = ensure_schema_object_at_pointer(schema, &field_pointer) else {
        return;
    };
    // Keep a field's default beside its $ref, not in the shared definition.
    property.insert("default".to_owned(), value);
    if let Some(parent) = schema.pointer_mut(&parent_pointer) {
        let mut required = schema_required_names(parent);
        required.retain(|name| name != field);
        set_schema_required_names(parent, &required);
    }
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

// Rust's str::trim uses Unicode White_Space. Keep an explicit BMP class
// instead of ECMA-262 \s, whose character set differs (notably U+0085/U+FEFF).
const EMPTY_STRING_PATTERN: &str =
    r"^[\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]*$";

/// Require at least one match of a strict input path and non-empty values for
/// every match. A wildcard is existential for presence: an empty sibling array
/// must not invalidate another sibling that has a match.
///
/// The complement of `items: { not: child }` expresses this without `contains`,
/// preserving the structural schema consumed by the closed settings compiler.
/// Typed command validation continues to enforce this relational constraint.
pub fn set_schema_strict_non_empty_path_metadata(schema: &mut Value, pointer: &str, path: &str) {
    let segments = parse_json_path(path);
    if segments.is_empty() {
        return;
    }
    let Some(resolved_pointer) = resolve_schema_pointer(schema, pointer) else {
        return;
    };
    // A missing property can be supplied by serde/arg defaults. Evaluate the
    // serialized default with the runtime path grammar and value predicate;
    // a mixed default such as ["a", ""] is not a valid substitute.
    let mut context = schema.pointer(&resolved_pointer);
    let mut defaults = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        context = match context.map(|current| path_schema_child(schema, current, segment)) {
            Some(Ok(child)) => child,
            None => None,
            // Multiple composed shapes can have different serde defaults or
            // custom branch selection. Do not invent a stricter raw-input
            // contract where that runtime choice is not represented.
            Some(Err(())) => return,
        };
        let matches = context
            .and_then(|child| {
                child
                    .get("default")
                    .or_else(|| resolve_schema_value(schema, child).get("default"))
            })
            .map(|value| json_path_matches_segments(value, &segments[index + 1..]))
            .unwrap_or_default();
        let invalid = matches.iter().any(|value| !value_present(value));
        defaults.push((!matches.is_empty() && !invalid, invalid));
    }
    let mut present = serde_json::json!({});
    let mut invalid = serde_json::json!({"anyOf": [
        {"type": "null"},
        {"type": "string", "pattern": EMPTY_STRING_PATTERN},
        {"type": "array", "maxItems": 0},
        {"type": "object", "maxProperties": 0},
    ]});
    for (index, segment) in segments.iter().enumerate().rev() {
        match segment {
            JsonPathSegment::Key(key) => {
                for (condition, absent_matches) in [
                    (&mut present, defaults[index].0),
                    (&mut invalid, defaults[index].1),
                ] {
                    let mut properties = serde_json::Map::new();
                    properties.insert(key.clone(), std::mem::take(condition));
                    *condition = serde_json::json!({
                        "type": "object",
                        "properties": properties,
                    });
                    if !absent_matches {
                        condition["required"] = serde_json::json!([key]);
                    }
                }
            }
            JsonPathSegment::AllItems => {
                for condition in [&mut present, &mut invalid] {
                    *condition = serde_json::json!({
                        "type": "array",
                        "not": {"items": {"not": std::mem::take(condition)}},
                    });
                }
            }
        }
    }
    let Some(object) = ensure_schema_object_at_pointer(schema, &resolved_pointer) else {
        return;
    };
    append_schema_not(
        object,
        serde_json::json!({"anyOf": [
            {"not": present}, invalid,
        ]}),
    );
}

fn path_schema_child<'a>(
    root: &'a Value,
    current: &'a Value,
    segment: &JsonPathSegment,
) -> std::result::Result<Option<&'a Value>, ()> {
    let current = resolve_schema_value(root, current);
    let expected = match segment {
        JsonPathSegment::Key(_) => "object",
        JsonPathSegment::AllItems => "array",
    };
    // A custom deserializer can turn a wire string into a Rust Vec, for
    // example. Its serialized validation path must not force the input to
    // become an array. Nullable containers retain their ordinary traversal.
    let incompatible = match current.get("type") {
        Some(Value::String(kind)) => kind != expected && kind != "null",
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .any(|kind| kind != expected && kind != "null"),
        _ => false,
    };
    if incompatible {
        return Err(());
    }
    let child = match segment {
        JsonPathSegment::Key(key) => current
            .get("properties")
            .and_then(|properties| properties.get(key)),
        JsonPathSegment::AllItems => current.get("items"),
    };
    if child.is_some() {
        return Ok(child);
    }
    for group in ["oneOf", "anyOf", "allOf"] {
        if let Some(branches) = current.get(group).and_then(Value::as_array) {
            let mut shapes = branches.iter().filter(|branch| {
                resolve_schema_value(root, branch)
                    .get("type")
                    .and_then(Value::as_str)
                    != Some("null")
            });
            if let Some(shape) = shapes.next() {
                if shapes.next().is_some() {
                    return Err(());
                }
                return path_schema_child(root, shape, segment);
            }
        }
    }
    // A validation path may describe a parsed enum payload or another
    // parse-only field. A wrapped public object does not expose that key at
    // this location; do not invent a raw-input requirement for it.
    if matches!(segment, JsonPathSegment::Key(_)) && current.get("properties").is_some() {
        return Err(());
    }
    Ok(None)
}

fn append_schema_not(object: &mut serde_json::Map<String, Value>, forbidden: Value) {
    if let Some(existing) = object.get_mut("not") {
        if existing == &forbidden {
            return;
        }
        if let Some(alternatives) = existing
            .as_object_mut()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.get_mut("anyOf"))
            .and_then(Value::as_array_mut)
        {
            if !alternatives.contains(&forbidden) {
                alternatives.push(forbidden);
            }
            return;
        }
        let previous = std::mem::take(existing);
        *existing = serde_json::json!({"anyOf": [previous, forbidden]});
    } else {
        object.insert("not".to_owned(), forbidden);
    }
}

pub fn prefix_schema_order_metadata(schema: &mut Value, prefix: &str) {
    if prefix.trim().is_empty() {
        return;
    }
    prefix_schema_order_metadata_on_value(schema, prefix);
}

pub fn remap_invalid_params_paths<T>(result: Result<T>, mappings: &[(&str, &str)]) -> Result<T> {
    result.map_err(|mut err| {
        if err.kind != PluginErrorKind::InvalidParams || mappings.is_empty() {
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
            err.diagnostic.message =
                remap_message_path_prefixes(err.diagnostic.message.as_str(), from, to);
            if let Some(data) = err.diagnostic.data.as_mut() {
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
    let mut collected = serde_json::Map::new();
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

fn collect_nested_object_properties(value: &Value, collected: &mut serde_json::Map<String, Value>) {
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
    let Some(mut literals) = super::schema_support::string_literals(existing) else {
        return existing.clone();
    };
    let Some(next_literals) = super::schema_support::string_literals(next) else {
        return existing.clone();
    };
    literals.extend(next_literals);
    serde_json::json!({
        "type": "string",
        "enum": literals.into_iter().collect::<Vec<_>>()
    })
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
    let value = if object.get(NON_EMPTY_METADATA).and_then(Value::as_bool) == Some(true)
        && non_empty_minimum_key(object) == Some(key)
    {
        value.max(1)
    } else {
        value
    };
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
    if let Some(key) = non_empty_minimum_key(object) {
        object.insert(NON_EMPTY_METADATA.to_string(), Value::Bool(true));
        set_schema_minimum_u64_metadata_on_object(object, key, 1);
    }
    if schema_kind_matches(object, "string") {
        append_schema_not(
            object,
            serde_json::json!({
                "type": "string", "pattern": EMPTY_STRING_PATTERN,
            }),
        );
    }
}

fn non_empty_minimum_key(object: &serde_json::Map<String, Value>) -> Option<&'static str> {
    if schema_kind_matches(object, "string") {
        Some("minLength")
    } else if schema_kind_matches(object, "array") {
        Some("minItems")
    } else if schema_kind_matches(object, "object") || object.contains_key("properties") {
        Some("minProperties")
    } else {
        None
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
