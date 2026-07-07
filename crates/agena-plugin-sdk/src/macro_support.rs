use std::borrow::ToOwned;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use std::sync::{OnceLock, RwLock};

use schemars::{JsonSchema, schema_for};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::{PluginError, Result};

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
    sanitize_schema_json(value)
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
    merge_schema_overlay(target, &overlay_without_root_meta);
    promote_nested_composite_properties(target);
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

pub fn sanitize_schema_json(value: Value) -> Value {
    sanitize_schema_json_value(value, true)
}

fn sanitize_schema_json_value(value: Value, remove_schema_metadata: bool) -> Value {
    match value {
        Value::Object(mut object) => {
            if remove_schema_metadata {
                object.remove("$schema");
                object.remove("title");
            }
            let mut cleaned = serde_json::Map::new();
            for (key, value) in object {
                let sanitized = match key.as_str() {
                    "properties" | "$defs" | "definitions" | "patternProperties"
                    | "dependentSchemas" => match value {
                        Value::Object(map) => Value::Object(
                            map.into_iter()
                                .map(|(nested_key, nested_value)| {
                                    (nested_key, sanitize_schema_json_value(nested_value, true))
                                })
                                .collect(),
                        ),
                        other => sanitize_schema_json_value(other, true),
                    },
                    _ => sanitize_schema_json_value(value, true),
                };
                cleaned.insert(key, sanitized);
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
                .map(|item| sanitize_schema_json_value(item, true))
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
                let left_order = schema_order_key(left_property);
                let right_order = schema_order_key(right_property);
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

const MAX_VARIANT_EXAMPLES: usize = 6;

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
        Some("string") => Some(serde_json::Value::String(format!("<{field_name}>"))),
        Some("integer") => Some(serde_json::Value::Number(1.into())),
        Some("number") => Some(serde_json::json!(1.0)),
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
    if item_schema.get("type").and_then(serde_json::Value::as_str) == Some("object")
        || item_schema.get("properties").is_some()
    {
        return None;
    }

    let mut labels = Vec::new();
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

pub fn normalize_trim_paths(input: &mut Value, paths: &[&str]) {
    for path in paths {
        let segments = parse_json_path(path);
        mutate_json_path_strings(input, &segments, &mut |text| {
            *text = text.trim().to_string();
        });
    }
}

pub fn normalize_trim_suffix_path(input: &mut Value, path: &str, suffix: &str) {
    let segments = parse_json_path(path);
    mutate_json_path_strings(input, &segments, &mut |text| {
        if let Some(stripped) = text.strip_suffix(suffix) {
            *text = stripped.to_string();
        }
    });
}

pub fn validate_non_empty_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for path in paths {
        let matches = json_path_matches(&json, path);
        if matches.is_empty() || matches.iter().any(|value| !value_present(value)) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must not be empty",
                display_path(path)
            )));
        }
    }
    Ok(())
}

pub fn validate_non_empty_if_present_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for path in paths {
        let matches = json_path_matches(&json, path);
        if matches.iter().any(|value| !value_present(value)) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must not be empty when present",
                display_path(path)
            )));
        }
    }
    Ok(())
}

pub fn validate_exactly_one_of_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let present = paths
        .iter()
        .filter(|path| json_path_present(&json, path))
        .count();
    if present != 1 {
        return Err(PluginError::invalid_params(format!(
            "exactly one of {} is required",
            human_join_paths(paths)
        )));
    }
    Ok(())
}

pub fn validate_at_least_one_of_paths<T>(value: &T, paths: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    if !paths.iter().any(|path| json_path_present(&json, path)) {
        return Err(PluginError::invalid_params(format!(
            "at least one of {} is required",
            human_join_paths(paths)
        )));
    }
    Ok(())
}

pub fn validate_min_items_path<T>(value: &T, path: &str, minimum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let matches = json_path_matches(&json, path);
    if matches.is_empty() || matches.iter().any(|value| array_len(value) < minimum) {
        return Err(PluginError::invalid_params(format!(
            "field `{}` requires at least {minimum} item{}",
            display_path(path),
            if minimum == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

pub fn validate_max_items_path<T>(value: &T, path: &str, maximum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    if json_path_matches(&json, path)
        .iter()
        .any(|value| array_len(value) > maximum)
    {
        return Err(PluginError::invalid_params(format!(
            "field `{}` accepts at most {maximum} item{}",
            display_path(path),
            if maximum == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

pub fn validate_max_chars_path<T>(value: &T, path: &str, maximum: usize) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    if json_path_matches(&json, path)
        .iter()
        .any(|value| string_char_count(value) > maximum)
    {
        return Err(PluginError::invalid_params(format!(
            "field `{}` must be at most {maximum} character{}",
            display_path(path),
            if maximum == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

pub fn validate_forbid_substrings_path<T>(value: &T, path: &str, forbidden: &[&str]) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    for candidate in json_path_matches(&json, path) {
        let Value::String(text) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a string",
                display_path(path)
            )));
        };
        if let Some(found) = forbidden.iter().find(|needle| text.contains(**needle)) {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must not contain `{}`",
                display_path(path),
                found
            )));
        }
    }
    Ok(())
}

pub fn validate_distinct_trimmed_path<T>(value: &T, path: &str) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    validate_distinct_trimmed_matches(json_path_matches(&json, path), path, None)
}

pub fn validate_distinct_trimmed_within_path<T>(
    value: &T,
    path: &str,
    scope_path: &str,
) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let path_segments = parse_json_path(path);
    let scope_segments = parse_json_path(scope_path);
    let Some(suffix) = path_segments.strip_prefix(scope_segments.as_slice()) else {
        return Err(PluginError::invalid_params(format!(
            "field `{}` must be nested within `{}`",
            display_path(path),
            display_path(scope_path)
        )));
    };
    for scope_root in json_path_matches_segments(&json, &scope_segments) {
        validate_distinct_trimmed_matches(
            json_path_matches_segments(scope_root, suffix),
            path,
            Some(scope_path),
        )?;
    }
    Ok(())
}

pub fn validate_requires_path<T>(value: &T, path: &str, required_path: &str) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let relation = parsed_relation_paths(path, required_path);
    for subroot in relation_subroots(&json, &relation) {
        if path_present_segments(subroot, &relation.left_suffix)
            && !path_present_segments(subroot, &relation.right_suffix)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` requires `{}`",
                display_path(path),
                display_path(required_path)
            )));
        }
    }
    Ok(())
}

pub fn validate_conflicts_with_path<T>(value: &T, path: &str, other_path: &str) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let relation = parsed_relation_paths(path, other_path);
    for subroot in relation_subroots(&json, &relation) {
        if path_present_segments(subroot, &relation.left_suffix)
            && path_present_segments(subroot, &relation.right_suffix)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` conflicts with `{}`",
                display_path(path),
                display_path(other_path)
            )));
        }
    }
    Ok(())
}

pub fn validate_required_unless_present_path<T>(
    value: &T,
    path: &str,
    unless_path: &str,
) -> Result<()>
where
    T: Serialize,
{
    let json =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params(err.to_string()))?;
    let relation = parsed_relation_paths(path, unless_path);
    for subroot in relation_subroots(&json, &relation) {
        if !path_present_segments(subroot, &relation.right_suffix)
            && !path_present_segments(subroot, &relation.left_suffix)
        {
            return Err(PluginError::invalid_params(format!(
                "field `{}` is required unless `{}` is present",
                display_path(path),
                display_path(unless_path)
            )));
        }
    }
    Ok(())
}

fn invalid_json_syntax_error(err: serde_json::Error, source: &str) -> PluginError {
    let detail = err.to_string();
    let message = format!("invalid JSON {source}: {detail}");
    PluginError::invalid_params_with_data(message, json_error_data(err, source, None))
}

fn invalid_json_data_error(
    err: serde_json::Error,
    source: &str,
    path: Option<String>,
) -> PluginError {
    let detail = err.to_string();
    let message = match path.as_deref().filter(|value| !value.is_empty()) {
        Some(path) => format!("invalid JSON {source} at `{path}`: {detail}"),
        None => format!("invalid JSON {source}: {detail}"),
    };
    PluginError::invalid_params_with_data(message, json_error_data(err, source, path))
}

fn json_error_data(
    err: serde_json::Error,
    source: &str,
    path: Option<String>,
) -> serde_json::Value {
    let category = match err.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    };
    let mut data = Map::new();
    data.insert("kind".into(), Value::String("json_input".into()));
    data.insert("source".into(), Value::String(source.to_string()));
    data.insert("category".into(), Value::String(category.to_string()));
    data.insert("detail".into(), Value::String(err.to_string()));
    if let Some(path) = path.filter(|value| !value.is_empty()) {
        data.insert("path".into(), Value::String(path));
    }
    if err.line() > 0 {
        data.insert("line".into(), Value::from(err.line() as u64));
    }
    if err.column() > 0 {
        data.insert("column".into(), Value::from(err.column() as u64));
    }
    Value::Object(data)
}

fn json_path_present(root: &Value, path: &str) -> bool {
    let segments = parse_json_path(path);
    path_present_segments(root, &segments)
}

fn value_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
    }
}

fn array_len(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        _ => 0,
    }
}

fn string_char_count(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        _ => 0,
    }
}

fn json_path_matches<'a>(root: &'a Value, path: &str) -> Vec<&'a Value> {
    let segments = parse_json_path(path);
    json_path_matches_segments(root, &segments)
}

fn json_path_matches_segments<'a>(root: &'a Value, segments: &[JsonPathSegment]) -> Vec<&'a Value> {
    let mut matches = Vec::new();
    collect_json_path_matches(root, segments, &mut matches);
    matches
}

fn path_present_segments(root: &Value, segments: &[JsonPathSegment]) -> bool {
    json_path_matches_segments(root, segments)
        .iter()
        .any(|value| value_present(value))
}

fn relation_subroots<'a>(root: &'a Value, relation: &ParsedRelationPaths) -> Vec<&'a Value> {
    let prefix = relation.common_prefix.as_slice();
    if prefix.is_empty() {
        vec![root]
    } else {
        json_path_matches_segments(root, prefix)
    }
}

fn parsed_relation_paths(left: &str, right: &str) -> ParsedRelationPaths {
    let left_segments = parse_json_path(left);
    let right_segments = parse_json_path(right);
    let prefix_len = common_prefix_len(&left_segments, &right_segments);
    ParsedRelationPaths {
        common_prefix: left_segments[..prefix_len].to_vec(),
        left_suffix: left_segments[prefix_len..].to_vec(),
        right_suffix: right_segments[prefix_len..].to_vec(),
    }
}

fn collect_json_path_matches<'a>(
    current: &'a Value,
    segments: &[JsonPathSegment],
    matches: &mut Vec<&'a Value>,
) {
    if segments.is_empty() {
        matches.push(current);
        return;
    }

    match &segments[0] {
        JsonPathSegment::Key(key) => {
            if let Value::Object(object) = current
                && let Some(next) = object.get(key)
            {
                collect_json_path_matches(next, &segments[1..], matches);
            }
        }
        JsonPathSegment::AllItems => {
            if let Value::Array(items) = current {
                for item in items {
                    collect_json_path_matches(item, &segments[1..], matches);
                }
            }
        }
    }
}

fn normalized_name_distance(left: &str, right: &str) -> usize {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    if left == right {
        return 0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0; right_chars.len() + 1];
    for (i, left_ch) in left_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, right_ch) in right_chars.iter().enumerate() {
            let replace = prev[j] + usize::from(left_ch != right_ch);
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            curr[j + 1] = replace.min(insert.min(delete));
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[right_chars.len()]
}

fn unknown_field_from_error_detail(detail: &str) -> Option<String> {
    let prefix = "unknown field `";
    let start = detail.find(prefix)? + prefix.len();
    let rest = &detail[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

fn schema_field_candidates(schema: &Value) -> Vec<String> {
    let mut candidates = Vec::new();
    collect_schema_field_candidates(schema, &mut candidates);
    candidates.sort();
    candidates.dedup();
    candidates
}

fn reject_unknown_object_fields(input: &Value, schema: &Value, kind: &str) -> Result<()> {
    let Some(object) = input.as_object() else {
        return Ok(());
    };
    if !schema_denies_unknown_properties(schema) {
        return Ok(());
    }
    let candidates = schema_field_candidates(schema);
    if candidates.is_empty() {
        return Ok(());
    }
    let candidate_set = candidates.iter().collect::<HashSet<_>>();
    for key in object.keys() {
        if candidate_set.contains(key) {
            continue;
        }
        let suggestions = suggest_name_candidates(key, candidates.iter(), 1);
        let message = if suggestions.is_empty() {
            format!("unknown {kind} '{key}'")
        } else {
            unknown_name_message(kind, key, &suggestions)
        };
        return Err(PluginError::invalid_params(message));
    }
    Ok(())
}

fn schema_denies_unknown_properties(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object
        .get("additionalProperties")
        .is_some_and(|value| matches!(value, Value::Bool(false)))
    {
        return true;
    }
    if object
        .get("unevaluatedProperties")
        .is_some_and(|value| matches!(value, Value::Bool(false)))
    {
        return true;
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get(key)
            && !items.is_empty()
            && items.iter().all(schema_denies_unknown_properties)
        {
            return true;
        }
    }
    false
}

fn collect_schema_field_candidates(schema: &Value, candidates: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property_schema) in properties {
            candidates.push(name.clone());
            if let Some(Value::Array(aliases)) = property_schema.get("x-agena-aliases") {
                candidates.extend(
                    aliases
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned),
                );
            }
            collect_schema_field_candidates(property_schema, candidates);
        }
    }
    if let Some(Value::Array(aliases)) = object.get("x-agena-aliases") {
        candidates.extend(
            aliases
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned),
        );
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(items)) = object.get(key) {
            for item in items {
                collect_schema_field_candidates(item, candidates);
            }
        }
    }
    if let Some(items) = object.get("items") {
        collect_schema_field_candidates(items, candidates);
    }
}

fn mutate_json_path_strings<F>(current: &mut Value, segments: &[JsonPathSegment], f: &mut F)
where
    F: FnMut(&mut String),
{
    if segments.is_empty() {
        if let Value::String(text) = current {
            f(text);
        }
        return;
    }

    match &segments[0] {
        JsonPathSegment::Key(key) => {
            if let Value::Object(object) = current
                && let Some(next) = object.get_mut(key)
            {
                mutate_json_path_strings(next, &segments[1..], f);
            }
        }
        JsonPathSegment::AllItems => {
            if let Value::Array(items) = current {
                for item in items {
                    mutate_json_path_strings(item, &segments[1..], f);
                }
            }
        }
    }
}

fn validate_distinct_trimmed_matches(
    matches: Vec<&Value>,
    path: &str,
    scope_path: Option<&str>,
) -> Result<()> {
    let mut seen = HashSet::new();
    for candidate in matches {
        let Value::String(text) = candidate else {
            return Err(PluginError::invalid_params(format!(
                "field `{}` must be a string",
                display_path(path)
            )));
        };
        let trimmed = text.trim();
        if !seen.insert(trimmed.to_string()) {
            let message = match scope_path {
                Some(scope) => format!(
                    "field `{}` must not contain duplicate values within `{}`",
                    display_path(path),
                    display_path(scope)
                ),
                None => format!(
                    "field `{}` must not contain duplicate values",
                    display_path(path)
                ),
            };
            return Err(PluginError::invalid_params(message));
        }
    }
    Ok(())
}

fn parse_json_path(path: &str) -> Vec<JsonPathSegment> {
    let mut segments = Vec::new();
    for segment in path.split('.') {
        if let Some(key) = segment.strip_suffix("[]") {
            if !key.is_empty() {
                segments.push(JsonPathSegment::Key(key.to_string()));
            }
            segments.push(JsonPathSegment::AllItems);
        } else if !segment.is_empty() {
            segments.push(JsonPathSegment::Key(segment.to_string()));
        }
    }
    segments
}

#[derive(Clone, PartialEq, Eq)]
enum JsonPathSegment {
    Key(String),
    AllItems,
}

struct ParsedRelationPaths {
    common_prefix: Vec<JsonPathSegment>,
    left_suffix: Vec<JsonPathSegment>,
    right_suffix: Vec<JsonPathSegment>,
}

fn common_prefix_len(left: &[JsonPathSegment], right: &[JsonPathSegment]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(a, b)| a == b)
        .count()
}

fn human_join_paths(paths: &[&str]) -> String {
    paths
        .iter()
        .map(|path| format!("`{}`", display_path(path)))
        .collect::<Vec<_>>()
        .join(" or ")
}

fn display_path(path: &str) -> &str {
    path.strip_prefix("args.").unwrap_or(path)
}
