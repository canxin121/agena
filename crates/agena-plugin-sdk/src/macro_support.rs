//! Runtime support helpers used by generated macro code.

use std::sync::{OnceLock, RwLock};

use agena_plugin_contracts::{
    MAX_JSON_ESCAPE_BYTES, MAX_JSON_ESCAPE_DEPTH, PluginServiceMethod, SettingsConstraints,
    SettingsContract, SettingsNode, SettingsNodeKind,
};
use schemars::{JsonSchema, schema_for};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{PluginError, Result, ToolTag};

mod schema_examples;
mod schema_metadata_support;
mod schema_support;
mod schema_text;

mod validation_paths;

use validation_paths::{
    compare_json_numbers, invalid_json_data_error, invalid_json_syntax_error, json_error_data,
    missing_field_from_error_detail, reject_unknown_object_fields, schema_field_candidates,
    schema_field_description, unknown_field_from_error_detail,
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

pub use schema_examples::{
    flattened_input_keys_for_parse_path, normalize_flattened_input_object,
    resolve_input_constraint_path, schema_example_texts,
};
pub use schema_metadata_support::{
    merge_flattened_schema_at_pointer, merge_schema_overlay_at_pointer,
    prefix_schema_order_metadata, prefixed_input_error_path_mappings, remap_invalid_params_paths,
    remap_invalid_params_paths_owned, rename_schema_property, set_schema_bool_metadata,
    set_schema_metadata, set_schema_minimum_u64_metadata, set_schema_non_empty_metadata,
    set_schema_number_metadata, set_schema_string_list_metadata, set_schema_string_metadata,
    set_schema_u64_metadata, set_schema_value_list_metadata, set_schema_value_metadata,
    suggest_name_candidates, unknown_name_message,
};
pub(crate) use schema_support::{
    ordered_schema_properties, resolve_schema_value, schema_order_key, string_literals,
    top_level_discriminated_variants, top_level_union_variants,
};
pub use schema_text::{
    command_usage_text, command_usage_text_for_schema, command_usage_text_from_schema,
    example_value_from_schema, merge_example_with_schema, schema_usage_text,
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

/// Build a cross-plugin service method from ordinary Rust input/output types.
/// Both sides compile through the same constrained contract used by settings
/// and operations, so service authors get typed RPC without hand-written JSON
/// schemas or renderer-specific metadata.
pub fn service_method_for<I, O>(id: impl Into<String>) -> PluginServiceMethod
where
    I: JsonSchema,
    O: JsonSchema,
{
    PluginServiceMethod::new(
        id,
        settings_contract_for::<I>(),
        settings_contract_for::<O>(),
    )
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

/// Compile an internal schemars document into the closed settings contract.
///
/// JSON Schema is deliberately used only inside the SDK/macro boundary. The
/// returned value is the only settings description placed in a manifest or
/// sent to a host surface. Unsupported composition and open-ended shapes are
/// rejected here so renderers never need to implement a second schema engine.
pub fn settings_contract_from_schema(
    schema: Value,
) -> std::result::Result<SettingsContract, String> {
    crate::settings_contract::settings_contract_from_schema(&schema)
}

pub fn settings_contract_for<T>() -> SettingsContract
where
    T: JsonSchema,
{
    crate::settings_contract::settings_contract_for::<T>()
        .expect("typed plugin settings must compile to the constrained settings contract")
}

pub fn settings_contract_for_default<T>(default: T) -> SettingsContract
where
    T: JsonSchema + Serialize,
{
    crate::settings_contract::settings_contract_for_default(default)
        .expect("typed plugin settings must compile to the constrained settings contract")
}

/// Contract for an operation that accepts no structured input.
pub fn empty_settings_contract() -> SettingsContract {
    SettingsContract::new(SettingsNode {
        id: "root".to_string(),
        path: String::new(),
        title: "Input".to_string(),
        description: String::new(),
        required: true,
        default: Some(serde_json::json!({})),
        constraints: SettingsConstraints::default(),
        sensitive: false,
        secret: false,
        kind: SettingsNodeKind::Object { fields: Vec::new() },
    })
}

/// Contract for an operation whose handler explicitly opts into bounded JSON.
pub fn json_settings_contract() -> SettingsContract {
    SettingsContract::new(SettingsNode {
        id: "root".to_string(),
        path: String::new(),
        title: "JSON input".to_string(),
        description: String::new(),
        required: true,
        default: None,
        constraints: SettingsConstraints::default(),
        sensitive: false,
        secret: false,
        kind: SettingsNodeKind::Json {
            max_bytes: MAX_JSON_ESCAPE_BYTES,
            max_depth: MAX_JSON_ESCAPE_DEPTH,
        },
    })
}

pub fn typed_tool_output<T>(value: T) -> Result<crate::ToolInvokeOutput>
where
    T: Serialize,
{
    let payload =
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params_error(&err))?;
    let output_text = match &payload {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        _ => payload.to_string(),
    };
    let summary = typed_tool_summary(&payload);
    Ok(crate::ToolInvokeOutput::from_parts(
        String::new(),
        summary,
        output_text,
        Some(payload),
        Default::default(),
        Vec::new(),
    ))
}

/// Structural summary for SDK shorthand outputs. Tools that need a semantic
/// result sentence should return `ToolInvokeOutput` directly; this adapter is
/// intentionally limited to stable shape/count information.
pub fn typed_tool_summary(value: &Value) -> String {
    match value {
        Value::Null => "No result".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => agena_plugin_contracts::normalize_tool_summary(value),
        Value::Array(items) => format!("{} items", items.len()),
        Value::Object(fields) => {
            if let Some(summary) = fields
                .get("summary")
                .and_then(Value::as_str)
                .filter(|summary| !summary.trim().is_empty())
            {
                return agena_plugin_contracts::normalize_tool_summary(summary);
            }
            if let Some(status) = fields
                .get("status")
                .and_then(Value::as_str)
                .filter(|status| !status.trim().is_empty())
            {
                return format!("Status {status}");
            }
            for key in ["results", "items", "files", "matches", "entries"] {
                if let Some(items) = fields.get(key).and_then(Value::as_array) {
                    return format!("{} {key}", items.len());
                }
            }
            if let Some(count) = fields.get("count").and_then(Value::as_u64) {
                return format!("{count} items");
            }
            format!("{} fields", fields.len())
        }
    }
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
            if let Some(field) = missing_field_from_error_detail(&inner.to_string())
                && let Some(description) = schema_field_description(schema, &field)
            {
                let message = format!("missing required field `{field}`: {description}");
                return Err(PluginError::invalid_params_with_data(
                    message,
                    json_error_data(inner, "value", Some(path)),
                ));
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

pub fn parse_defaulted_settings<T>(input: Value, invalid: &str) -> Result<T>
where
    T: Default + DeserializeOwned,
{
    if input.is_null() {
        Ok(T::default())
    } else {
        parse_typed_json_value(input)
            .map_err(|err| PluginError::internal(format!("{invalid}: {err}")))
    }
}

pub fn store_once<T>(cell: &OnceLock<T>, value: T, already: &str) -> Result<()> {
    cell.set(value)
        .map_err(|_| PluginError::internal(already.to_string()))
}

pub fn store_rwlock_option<T>(cell: &RwLock<Option<T>>, value: T, poisoned: &str) -> Result<()> {
    *cell
        .write()
        .map_err(|error| PluginError::internal(format!("{poisoned}: {error}")))? = Some(value);
    Ok(())
}

pub fn normalize_schema_json(value: Value) -> Value {
    normalize_schema_json_value(value, true)
}

/// Normalize structural Schemars output while preserving author-facing
/// titles/descriptions. Settings contracts use this path so Rust type names and
/// doc comments survive compilation into the closed UI contract; tool input
/// schemas keep using `normalize_schema_json`, which intentionally strips
/// presentation metadata from their machine-facing schema.
pub(crate) fn normalize_settings_schema_json(value: Value) -> Value {
    normalize_schema_json_value(value, false)
}

fn normalize_schema_json_value(value: Value, remove_titles: bool) -> Value {
    match value {
        Value::Object(mut object) => {
            object.remove("$schema");
            if remove_titles {
                object.remove("title");
            }
            let mut cleaned = serde_json::Map::new();
            for (key, value) in object {
                let normalized = match key.as_str() {
                    "properties" => match value {
                        Value::Object(map) => Value::Object(
                            map.into_iter()
                                .map(|(nested_key, nested_value)| {
                                    (
                                        nested_key,
                                        normalize_schema_json_value(nested_value, remove_titles),
                                    )
                                })
                                .collect(),
                        ),
                        other => normalize_schema_json_value(other, remove_titles),
                    },
                    "required" => match value {
                        Value::Array(items) => Value::Array(items),
                        other => normalize_schema_json_value(other, remove_titles),
                    },
                    "$defs" | "definitions" | "patternProperties" | "dependentSchemas" => {
                        match value {
                            Value::Object(map) => Value::Object(
                                map.into_iter()
                                    .map(|(nested_key, nested_value)| {
                                        (
                                            nested_key,
                                            normalize_schema_json_value(
                                                nested_value,
                                                remove_titles,
                                            ),
                                        )
                                    })
                                    .collect(),
                            ),
                            other => normalize_schema_json_value(other, remove_titles),
                        }
                    }
                    _ => normalize_schema_json_value(value, remove_titles),
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
                .map(|item| normalize_schema_json_value(item, remove_titles))
                .collect(),
        ),
        other => other,
    }
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
        serde_json::to_value(value).map_err(|err| PluginError::invalid_params_error(&err))?;
    normalize(&mut json);
    parse_typed_json_value(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    #[derive(Debug, serde::Deserialize)]
    struct EffectsInput {
        command: String,
        filesystem_effects: serde_json::Value,
    }

    fn effects_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "filesystem_effects": {
                    "type": "object",
                    "description": "Declared filesystem effects (read/write paths) or empty list"
                }
            },
            "required": ["command", "filesystem_effects"],
            "additionalProperties": false
        })
    }

    #[test]
    fn missing_field_error_names_field_with_description() {
        let input = serde_json::json!({"command": "ls"});
        let result: Result<EffectsInput> =
            parse_typed_json_value_with_field_suggestions(input, &effects_schema(), "value");
        let err = result.expect_err("missing field should fail");
        let diagnostic = err.diagnostic_message();
        assert!(
            diagnostic.contains("missing required field `filesystem_effects`"),
            "diagnostic: {diagnostic}"
        );
        assert!(
            diagnostic.contains("Declared filesystem effects"),
            "diagnostic: {diagnostic}"
        );
        assert!(
            err.failure.user.fallback.contains("filesystem_effects"),
            "fallback: {}",
            err.failure.user.fallback
        );
    }

    #[test]
    fn unknown_field_suggestion_still_precedes_missing_field() {
        let input = serde_json::json!({"commnd": "ls", "filesystem_effects": {}});
        let result: Result<EffectsInput> =
            parse_typed_json_value_with_field_suggestions(input, &effects_schema(), "value");
        let err = result.expect_err("unknown field should fail");
        let diagnostic = err.diagnostic_message();
        assert!(
            diagnostic.contains("unknown") && diagnostic.contains("commnd"),
            "diagnostic: {diagnostic}"
        );
    }
}
