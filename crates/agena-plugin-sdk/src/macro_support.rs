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
    set_schema_input_default_metadata, set_schema_metadata, set_schema_minimum_u64_metadata,
    set_schema_non_empty_metadata, set_schema_number_metadata,
    set_schema_strict_non_empty_path_metadata, set_schema_string_list_metadata,
    set_schema_string_metadata, set_schema_u64_metadata, set_schema_value_list_metadata,
    set_schema_value_metadata, suggest_name_candidates, unknown_name_message,
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
/// and commands, so service authors get typed RPC without hand-written JSON
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

/// Contract for a command that accepts no structured input.
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

/// Contract for a command whose handler explicitly opts into bounded JSON.
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
    validation_paths::validate_json_container_shapes(&input, schema)?;
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

pub use crate::manifest_support::normalize_schema_json;

/// Normalize structural Schemars output while preserving author-facing
/// titles/descriptions. Settings contracts use this path so Rust type names and
/// doc comments survive compilation into the closed UI contract; tool input
/// schemas keep using `normalize_schema_json`, which intentionally strips
/// presentation metadata from their machine-facing schema.
pub(crate) fn normalize_settings_schema_json(value: Value) -> Value {
    crate::manifest_support::normalize_schema_json_value(value, false)
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
mod numeric_generation_tests {
    use serde_json::json;

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(
        minimum("limit", 0), maximum("ceiling", 10),
        exclusive_minimum("after", -1), exclusive_maximum("before", 11),
        item_minimum("samples", 0), item_maximum("samples", 10),
        item_exclusive_minimum("samples", -1), item_exclusive_maximum("samples", 11)
    )]
    struct NumericInput {
        #[serde(rename = "limit")]
        #[arg(minimum = 1)]
        lower: i64,
        #[arg(maximum = 9)]
        ceiling: i64,
        #[arg(exclusive_minimum = 2)]
        after: i64,
        #[arg(exclusive_maximum = 8)]
        before: i64,
        #[arg(
            item_minimum = 1,
            item_maximum = 9,
            item_exclusive_minimum = 2,
            item_exclusive_maximum = 8
        )]
        samples: Vec<i64>,
    }

    fn valid() -> serde_json::Value {
        json!({"limit": 1, "ceiling": 9, "after": 3, "before": 7, "samples": [3, 7]})
    }

    #[test]
    fn numeric_field_overrides_and_array_items_keep_schema_bounds() {
        let schema = NumericInput::input_schema();
        assert_eq!(schema["properties"]["limit"]["minimum"], 1);
        assert_eq!(schema["properties"]["ceiling"]["maximum"], 9);
        assert_eq!(schema["properties"]["after"]["exclusiveMinimum"], 2);
        assert_eq!(schema["properties"]["before"]["exclusiveMaximum"], 8);
        let items = &schema["properties"]["samples"]["items"];
        assert_eq!(items["minimum"], 1);
        assert_eq!(items["maximum"], 9);
        assert_eq!(items["exclusiveMinimum"], 2);
        assert_eq!(items["exclusiveMaximum"], 8);
        assert!(NumericInput::parse_input(valid()).is_ok());
    }

    #[test]
    fn numeric_runtime_boundaries_keep_field_and_item_error_paths() {
        for (field, invalid) in [
            ("limit", json!(0)),
            ("ceiling", json!(10)),
            ("after", json!(2)),
            ("before", json!(8)),
            ("samples", json!([2])),
        ] {
            let mut input = valid();
            input[field] = invalid;
            let error = NumericInput::parse_input(input).unwrap_err();
            assert!(error.to_string().contains(field), "{field}: {error}");
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct PreciseInteger {
        #[arg(minimum = 9_007_199_254_740_993_u64)]
        value: u64,
    }

    #[test]
    fn numeric_schema_and_validation_preserve_large_integer_precision() {
        assert_eq!(
            PreciseInteger::input_schema()["properties"]["value"]["minimum"],
            json!(9_007_199_254_740_993_u64),
        );
        assert!(PreciseInteger::parse_input(json!({"value": 9_007_199_254_740_993_u64})).is_ok());
        assert!(PreciseInteger::parse_input(json!({"value": 9_007_199_254_740_992_u64})).is_err());
    }
}

#[cfg(test)]
mod count_generation_tests {
    use serde_json::json;
    use std::collections::BTreeMap;

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(
        min_items("entries", 0),
        max_items("entries", 5),
        min_properties("settings", 0),
        max_properties("settings", 5),
        min_chars("caption", 0),
        max_chars("caption", 5),
        item_min_properties("objects", 0),
        item_max_properties("objects", 5),
        item_min_chars("labels", 0),
        item_max_chars("labels", 5)
    )]
    struct CountInput {
        #[serde(rename = "entries")]
        #[arg(min_items = 1, max_items = 2)]
        elements: Vec<String>,
        #[arg(min_properties = 1, max_properties = 2)]
        settings: BTreeMap<String, u8>,
        #[arg(min_chars = 2, max_chars = 3)]
        caption: String,
        #[arg(item_min_properties = 1, item_max_properties = 2)]
        objects: Vec<BTreeMap<String, u8>>,
        #[arg(item_min_chars = 2, item_max_chars = 3)]
        labels: Vec<String>,
    }

    fn valid() -> serde_json::Value {
        json!({
            "entries": ["a"], "settings": {"a": 1}, "caption": "界🙂",
            "objects": [{"a": 1}], "labels": ["界🙂"]
        })
    }

    #[test]
    fn count_field_and_item_overrides_keep_schema_bounds() {
        let schema = CountInput::input_schema();
        for (path, key, expected) in [
            ("/properties/entries", "minItems", 1),
            ("/properties/entries", "maxItems", 2),
            ("/properties/settings", "minProperties", 1),
            ("/properties/settings", "maxProperties", 2),
            ("/properties/caption", "minLength", 2),
            ("/properties/caption", "maxLength", 3),
            ("/properties/objects/items", "minProperties", 1),
            ("/properties/objects/items", "maxProperties", 2),
            ("/properties/labels/items", "minLength", 2),
            ("/properties/labels/items", "maxLength", 3),
        ] {
            assert_eq!(schema.pointer(path).unwrap()[key], expected, "{path}/{key}");
        }
        assert!(CountInput::parse_input(valid()).is_ok());
    }

    #[test]
    fn count_runtime_bounds_preserve_unicode_paths_and_error_order() {
        for (field, value) in [
            ("entries", json!([])),
            ("entries", json!(["a", "b", "c"])),
            ("settings", json!({})),
            ("settings", json!({"a":1,"b":2,"c":3})),
            ("caption", json!("界")),
            ("caption", json!("🙂🙂🙂🙂")),
            ("objects", json!([{}])),
            ("objects", json!([{"a":1,"b":2,"c":3}])),
            ("labels", json!(["界"])),
            ("labels", json!(["🙂🙂🙂🙂"])),
        ] {
            let mut input = valid();
            input[field] = value;
            let error = CountInput::parse_input(input).unwrap_err();
            assert_eq!(error.kind, crate::PluginErrorKind::InvalidParams);
            assert!(error.to_string().contains(field), "{field}: {error}");
        }
        let mut input = valid();
        input["entries"] = json!([]);
        input["settings"] = json!({});
        input["caption"] = json!("");
        let error = CountInput::parse_input(input).unwrap_err();
        assert!(error.to_string().contains("entries"), "{error}");
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct LowerBoundPolicies {
        #[arg(non_empty, min_items = 0)]
        entries: Vec<String>,
        #[arg(non_empty, min_properties = 0)]
        settings: BTreeMap<String, u8>,
        #[arg(non_empty, min_chars = 0)]
        caption: String,
    }

    #[test]
    fn count_schema_non_empty_lower_bound_matches_runtime() {
        let schema = LowerBoundPolicies::input_schema();
        assert_eq!(schema["properties"]["entries"]["minItems"], 1);
        assert_eq!(schema["properties"]["settings"]["minProperties"], 1);
        assert_eq!(schema["properties"]["caption"]["minLength"], 1);
        for field in ["entries", "settings", "caption"] {
            assert_eq!(schema["properties"][field]["x-agena-non-empty"], true);
        }
        let valid = json!({"entries":["a"],"settings":{"a":1},"caption":"界"});
        assert!(LowerBoundPolicies::parse_input(valid.clone()).is_ok());
        #[cfg(feature = "schema-validation")]
        assert!(jsonschema::is_valid(&schema, &valid));
        for field in ["entries", "settings", "caption"] {
            let mut input = valid.clone();
            input[field] = match field {
                "entries" => json!([]),
                "settings" => json!({}),
                _ => json!(""),
            };
            #[cfg(feature = "schema-validation")]
            assert!(!jsonschema::is_valid(&schema, &input), "{field}");
            assert!(LowerBoundPolicies::parse_input(input).is_err());
        }
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    struct SourceCountBatch {
        entries: Vec<String>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(
        non_empty("batch.entries"),
        min_items("batch.entries", 0),
        min_items("rows[]", 0)
    )]
    struct SourceNonEmptyCounts {
        batch: SourceCountBatch,
        #[arg(item_non_empty)]
        rows: Vec<Vec<String>>,
    }

    #[test]
    fn count_source_and_item_non_empty_survive_zero_minimums() {
        let schema = SourceNonEmptyCounts::input_schema();
        let batch = super::resolve_schema_value(&schema, &schema["properties"]["batch"]);
        assert_eq!(batch["properties"]["entries"]["minItems"], 1, "{schema:#}");
        assert_eq!(batch["properties"]["entries"]["x-agena-non-empty"], true);
        assert_eq!(schema["properties"]["rows"]["items"]["minItems"], 1);
        let valid = json!({"batch":{"entries":["a"]},"rows":[["a"]]});
        assert!(SourceNonEmptyCounts::parse_input(valid.clone()).is_ok());
        #[cfg(feature = "schema-validation")]
        assert!(jsonschema::is_valid(&schema, &valid));
        for (pointer, value) in [("/batch/entries", json!([])), ("/rows", json!([[]]))] {
            let mut input = valid.clone();
            *input.pointer_mut(pointer).unwrap() = value;
            #[cfg(feature = "schema-validation")]
            assert!(!jsonschema::is_valid(&schema, &input), "{pointer}");
            assert!(SourceNonEmptyCounts::parse_input(input).is_err());
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("entries"), min_items("entries", 0))]
    struct SourceNonEmptyElements {
        entries: Vec<String>,
    }

    #[test]
    fn count_direct_array_source_non_empty_keeps_element_semantics() {
        let schema = SourceNonEmptyElements::input_schema();
        assert_eq!(schema["properties"]["entries"]["items"]["minLength"], 1);
        assert_eq!(
            schema["properties"]["entries"]["items"]["x-agena-non-empty"],
            true
        );
        for (entries, accepted) in [
            (json!(["a"]), true),
            (json!([""]), false),
            (json!(["a", ""]), false),
        ] {
            let input = json!({"entries": entries});
            #[cfg(feature = "schema-validation")]
            assert_eq!(jsonschema::is_valid(&schema, &input), accepted);
            assert_eq!(SourceNonEmptyElements::parse_input(input).is_ok(), accepted);
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(min_items("entries", 0))]
    struct SchemarsCountOverrides {
        #[schemars(length(min = 3))]
        #[arg(min_items = 1)]
        entries: Vec<String>,
        #[schemars(length(min = 3))]
        #[arg(min_items = 0)]
        empty_allowed: Vec<String>,
        #[schemars(length(min = 3))]
        #[arg(non_empty, min_items = 0)]
        required_entries: Vec<String>,
    }

    #[test]
    fn count_explicit_array_minimum_replaces_schemars_before_non_empty() {
        let schema = SchemarsCountOverrides::input_schema();
        assert_eq!(schema["properties"]["entries"]["minItems"], 1);
        assert_eq!(schema["properties"]["empty_allowed"]["minItems"], 0);
        assert_eq!(schema["properties"]["required_entries"]["minItems"], 1);
        assert!(
            schema["properties"]["entries"]
                .get("x-agena-non-empty")
                .is_none()
        );
        assert!(
            schema["properties"]["empty_allowed"]
                .get("x-agena-non-empty")
                .is_none()
        );
        assert_eq!(
            schema["properties"]["required_entries"]["x-agena-non-empty"],
            true
        );
        let valid = json!({"entries":["a"],"empty_allowed":[],"required_entries":["a"]});
        assert!(SchemarsCountOverrides::parse_input(valid.clone()).is_ok());
        #[cfg(feature = "schema-validation")]
        assert!(jsonschema::is_valid(&schema, &valid));
        for field in ["entries", "required_entries"] {
            let mut input = valid.clone();
            input[field] = json!([]);
            #[cfg(feature = "schema-validation")]
            assert!(!jsonschema::is_valid(&schema, &input), "{field}");
            assert!(SchemarsCountOverrides::parse_input(input).is_err());
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("entries"))]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum EnumNonEmptyCounts {
        #[input(min_items("entries", 0))]
        Values { entries: Vec<String> },
    }

    #[test]
    fn count_enum_non_empty_survives_variant_minimum() {
        let schema = EnumNonEmptyCounts::input_schema();
        let entries = ["oneOf", "anyOf", "allOf"]
            .iter()
            .find_map(|group| schema.pointer(&format!("/{group}/0/properties/entries")))
            .expect("enum branch must expose the entries property");
        assert_eq!(entries["minItems"], 1);
        assert_eq!(entries["x-agena-non-empty"], true);
        for (entries, accepted) in [(json!(["a"]), true), (json!([]), false)] {
            let input = json!({"kind":"values", "entries":entries});
            #[cfg(feature = "schema-validation")]
            assert_eq!(jsonschema::is_valid(&schema, &input), accepted);
            assert_eq!(EnumNonEmptyCounts::parse_input(input).is_ok(), accepted);
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct ShapeNonEmptyCounts {
        #[arg(non_empty)]
        entries: Vec<String>,
    }

    struct CountOverlayPlugin;

    #[crate::agena_plugin(
        namespace = "count_fixture",
        name = "non_empty_overlay",
        version = "0.0.0",
        summary = "Verify typed shape count overlays"
    )]
    impl CountOverlayPlugin {
        #[tool(
            name = "count",
            summary = "Count entries after typed input validation",
            min_items("entries", 0)
        )]
        async fn count(&self, input: ShapeNonEmptyCounts) -> crate::Result<String> {
            Ok(input.entries.len().to_string())
        }
    }

    #[tokio::test]
    async fn count_typed_shape_non_empty_survives_tool_spec_minimum() {
        let plugin = CountOverlayPlugin;
        let manifest = crate::Plugin::manifest(&plugin);
        let schema = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "count")
            .expect("generated tool must be in the manifest")
            .input_schema();
        assert_eq!(
            ShapeNonEmptyCounts::input_schema()["properties"]["entries"]["minItems"],
            1
        );
        assert_eq!(schema["properties"]["entries"]["minItems"], 1);
        assert_eq!(schema["properties"]["entries"]["x-agena-non-empty"], true);
        let invoke = |input| crate::ToolInvokeInput {
            tool_name: "count".to_owned(),
            session_id: 1,
            call_id: 1,
            workspace_root: "/fixture".to_owned(),
            input,
        };
        let valid = json!({"entries":["a"]});
        #[cfg(feature = "schema-validation")]
        assert!(jsonschema::is_valid(&schema, &valid));
        let result = crate::Plugin::tool_invoke(&plugin, invoke(valid))
            .await
            .unwrap();
        assert_eq!(result.output_text, "1");
        let invalid = json!({"entries":[]});
        #[cfg(feature = "schema-validation")]
        assert!(!jsonschema::is_valid(&schema, &invalid));
        let error = crate::Plugin::tool_invoke(&plugin, invoke(invalid))
            .await
            .unwrap_err();
        assert_eq!(error.kind, crate::PluginErrorKind::InvalidParams);
    }

    #[test]
    fn count_non_empty_fact_survives_composite_schema_overlays() {
        let mut schema = json!({"anyOf":[
            {"type":"array", "items":{"type":"string"}},
            {"type":"string"},
            {"type":"object"},
            {"type":"null"}
        ]});
        super::set_schema_non_empty_metadata(&mut schema, "");
        for key in ["minItems", "minLength", "minProperties"] {
            super::set_schema_u64_metadata(&mut schema, "", key, 0);
        }
        for (index, key) in [(0, "minItems"), (1, "minLength"), (2, "minProperties")] {
            assert_eq!(schema["anyOf"][index][key], 1);
            assert_eq!(schema["anyOf"][index]["x-agena-non-empty"], true);
        }
        assert!(schema["anyOf"][3].get("x-agena-non-empty").is_none());
        #[cfg(feature = "schema-validation")]
        {
            for input in [json!([]), json!(""), json!({})] {
                assert!(!jsonschema::is_valid(&schema, &input));
            }
            for input in [json!(["a"]), json!("a"), json!({"a":1}), json!(null)] {
                assert!(jsonschema::is_valid(&schema, &input));
            }
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(
        min_items("entries", 1),
        max_items("entries", 2),
        min_properties("settings", 1),
        max_properties("settings", 2),
        min_chars("caption", 2),
        max_chars("caption", 3)
    )]
    struct SourceCounts {
        entries: Vec<String>,
        settings: BTreeMap<String, u8>,
        caption: String,
    }

    #[test]
    fn count_source_constraints_keep_schema_and_runtime_rules() {
        let schema = SourceCounts::input_schema();
        for (field, key, expected) in [
            ("entries", "minItems", 1),
            ("entries", "maxItems", 2),
            ("settings", "minProperties", 1),
            ("settings", "maxProperties", 2),
            ("caption", "minLength", 2),
            ("caption", "maxLength", 3),
        ] {
            assert_eq!(schema["properties"][field][key], expected, "{field}/{key}");
        }
        assert!(
            SourceCounts::parse_input(json!({
                "entries":["a"], "settings":{"a":1}, "caption":"界🙂"
            }))
            .is_ok()
        );
        assert!(
            SourceCounts::parse_input(json!({
                "entries":["a"], "settings":{"a":1}, "caption":"界"
            }))
            .is_err()
        );
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct CountShape {
        #[serde(rename = "caption")]
        label: String,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(min_chars("caption", 2), max_chars("caption", 3))]
    struct FlattenedCount {
        #[serde(flatten)]
        #[input(flatten_shape)]
        shape: CountShape,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "mode", rename_all = "snake_case")]
    enum CountMode {
        Read {
            #[arg(min_chars = 2, max_chars = 3)]
            caption: String,
        },
    }

    #[test]
    fn count_validation_preserves_flattened_paths_and_enum_fields() {
        assert!(FlattenedCount::parse_input(json!({"caption":"界🙂"})).is_ok());
        let error = FlattenedCount::parse_input(json!({"caption":"界"})).unwrap_err();
        assert!(error.to_string().contains("caption"), "{error}");
        assert!(CountMode::parse_input(json!({"mode":"read","caption":"界🙂"})).is_ok());
        let error =
            CountMode::parse_input(json!({"mode":"read","caption":"🙂🙂🙂🙂"})).unwrap_err();
        assert!(error.to_string().contains("caption"), "{error}");
    }
}

#[cfg(test)]
mod strict_non_empty_path_tests {
    use serde_json::{Value, json};

    fn agrees<T: crate::ToolInput>(input: Value, accepted: bool) {
        #[cfg(feature = "schema-validation")]
        {
            let schema = T::input_schema();
            let normalized = crate::manifest_support::normalize_schema_json(schema.clone());
            for candidate in [&schema, &normalized] {
                assert_eq!(
                    jsonschema::is_valid(candidate, &input),
                    accepted,
                    "schema for {}: {input}",
                    std::any::type_name::<T>(),
                );
            }
        }
        let result = T::parse_input(input.clone());
        assert_eq!(result.is_ok(), accepted, "runtime: {input}");
        if let Err(error) = result {
            assert_eq!(error.kind, crate::PluginErrorKind::InvalidParams);
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(deny_unknown_fields)]
    #[input(non_empty("entries"))]
    struct SourceItems {
        entries: Vec<String>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(deny_unknown_fields)]
    struct FieldItems {
        #[arg(item_non_empty)]
        entries: Vec<String>,
    }

    #[test]
    fn strict_source_and_field_items_require_a_match() {
        for (input, accepted) in [
            (json!({"entries": ["a"]}), true),
            (json!({"entries": []}), false),
            (json!({"entries": [""]}), false),
            (json!({"entries": ["a", ""]}), false),
            (json!({"entries": ["a"], "unexpected": true}), false),
        ] {
            agrees::<SourceItems>(input.clone(), accepted);
            agrees::<FieldItems>(input, accepted);
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("entries"))]
    struct StrictOptional {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entries: Option<Vec<String>>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty_if_present("entries"))]
    struct OptionalItems {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entries: Option<Vec<String>>,
    }

    #[test]
    fn strict_optional_and_if_present_keep_distinct_zero_match_rules() {
        for input in [json!({}), json!({"entries": null}), json!({"entries": []})] {
            agrees::<StrictOptional>(input.clone(), false);
            agrees::<OptionalItems>(input, true);
        }
        for (input, accepted) in [
            (json!({"entries": ["a"]}), true),
            (json!({"entries": [""]}), false),
        ] {
            agrees::<StrictOptional>(input.clone(), accepted);
            agrees::<OptionalItems>(input, accepted);
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct StrictText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[arg(non_empty)]
        text: Option<String>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct OptionalText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[arg(non_empty_if_present)]
        text: Option<String>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct NullableItems {
        #[arg(item_non_empty)]
        entries: Vec<Option<String>>,
    }

    #[test]
    fn nullable_values_and_rust_whitespace_keep_runtime_meaning() {
        for input in [json!({}), json!({"text": null})] {
            agrees::<StrictText>(input.clone(), false);
            agrees::<OptionalText>(input, true);
        }
        let whitespace = "\u{0009}\u{000a}\u{000b}\u{000c}\u{000d}\u{0020}\u{0085}\u{00a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}";
        assert!(whitespace.chars().all(char::is_whitespace));
        for (text, accepted) in [
            (String::new(), false),
            (whitespace.to_owned(), false),
            (format!("{whitespace}a{whitespace}"), true),
            ("\u{200b}".to_owned(), true),
            ("\u{feff}".to_owned(), true),
        ] {
            agrees::<StrictText>(json!({"text": text}), accepted);
            agrees::<OptionalText>(json!({"text": text}), accepted);
            agrees::<NullableItems>(json!({"entries": [text]}), accepted);
        }
        for (entries, accepted) in [
            (json!([]), false),
            (json!([null]), false),
            (json!(["a", null]), false),
            (json!(["a", " "]), false),
            (json!(["a", "b"]), true),
        ] {
            agrees::<NullableItems>(json!({"entries": entries}), accepted);
        }
    }

    #[test]
    fn whitespace_schema_tracks_rust_unicode_white_space() {
        for whitespace in (0..=u32::from(char::MAX))
            .filter_map(char::from_u32)
            .filter(|ch| ch.is_whitespace())
        {
            agrees::<StrictText>(json!({"text": whitespace.to_string()}), false);
        }
        for visible in ['\u{200b}', '\u{feff}'] {
            assert!(!visible.is_whitespace());
            agrees::<StrictText>(json!({"text": visible.to_string()}), true);
        }
    }

    fn comma_values<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(text.split(',').map(ToOwned::to_owned).collect())
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    struct ParsedStrings {
        #[serde(deserialize_with = "comma_values")]
        #[schemars(with = "String")]
        #[arg(non_empty)]
        entries: Vec<String>,
    }

    #[test]
    fn parsed_validation_paths_do_not_replace_the_wire_shape() {
        agrees::<ParsedStrings>(json!({"entries":"a,b"}), true);
        // The parser owns interpretation of the comma-separated values. The
        // public string schema must not invent an array requirement for it.
        assert_eq!(
            ParsedStrings::input_schema()["properties"]["entries"]["type"],
            "string"
        );
        assert!(ParsedStrings::parse_input(json!({"entries":"a,,b"})).is_err());
    }

    fn good_default() -> Vec<String> {
        vec!["a".to_owned()]
    }

    fn mixed_default() -> Vec<String> {
        vec!["a".to_owned(), String::new()]
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("entries"))]
    struct SerdeDefaultItems {
        #[serde(default = "good_default")]
        entries: Vec<String>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("entries"))]
    struct ArgDefaultItems {
        #[arg(default = good_default())]
        entries: Vec<String>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("entries"))]
    struct MixedDefaultItems {
        #[serde(default = "mixed_default")]
        entries: Vec<String>,
    }

    #[test]
    fn defaults_preserve_omission_without_hiding_invalid_values() {
        agrees::<SerdeDefaultItems>(json!({}), true);
        agrees::<ArgDefaultItems>(json!({}), true);
        assert!(ArgDefaultItems::input_schema().get("required").is_none());
        agrees::<MixedDefaultItems>(json!({}), false);
        for (input, accepted) in [
            (json!({"entries": ["a"]}), true),
            (json!({"entries": []}), false),
            (json!({"entries": ["a", ""]}), false),
            (json!({"entries": [" "]}), false),
        ] {
            agrees::<SerdeDefaultItems>(input.clone(), accepted);
            agrees::<ArgDefaultItems>(input.clone(), accepted);
            agrees::<MixedDefaultItems>(input, accepted);
        }
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind", rename_all = "snake_case")]
    #[input(non_empty("entries"))]
    enum DefaultedMode {
        Values {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind", content = "payload")]
    enum AdjacentValues {
        #[input(non_empty("entries"))]
        Values { entries: Vec<String> },
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    enum ExternalValues {
        #[input(non_empty("entries"))]
        Values { entries: Vec<String> },
    }

    #[test]
    fn wrapped_variant_payloads_keep_their_public_envelopes() {
        agrees::<AdjacentValues>(json!({"kind":"Values", "payload":{"entries":["a"]}}), true);
        agrees::<ExternalValues>(json!({"Values":{"entries":["a"]}}), true);
        // The existing metadata path does not describe these envelope forms.
        // Preserve accepted wire data; typed variant validation still rejects
        // empty entries rather than requiring a nonexistent top-level key.
        assert!(
            AdjacentValues::parse_input(json!({"kind":"Values", "payload":{"entries":[]}}))
                .is_err()
        );
        assert!(ExternalValues::parse_input(json!({"Values":{"entries":[]}})).is_err());
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind")]
    enum OmittedSchemaVariants {
        #[serde(skip)]
        #[input(non_empty("entries"))]
        Skipped {
            entries: Vec<String>,
        },
        #[serde(skip_deserializing)]
        #[input(non_empty("entries"))]
        OutputOnly {
            entries: Vec<String>,
        },
        #[schemars(skip)]
        #[input(non_empty("entries"))]
        Hidden {
            entries: Vec<String>,
        },
        Ordinary {
            entries: Vec<String>,
        },
        #[input(non_empty("entries"))]
        Required {
            entries: Vec<String>,
        },
    }

    #[test]
    fn omitted_variants_do_not_shift_constraints_to_other_branches() {
        assert!(serde_json::to_value(OmittedSchemaVariants::Skipped { entries: vec![] }).is_err());
        for entries in [json!([]), json!([""]), json!(["a"])] {
            agrees::<OmittedSchemaVariants>(json!({"kind":"Ordinary", "entries":entries}), true);
        }
        agrees::<OmittedSchemaVariants>(json!({"kind":"Required", "entries":["a"]}), true);
        agrees::<OmittedSchemaVariants>(json!({"kind":"Required", "entries":[]}), false);
        agrees::<OmittedSchemaVariants>(json!({"kind":"Required", "entries":[""]}), false);
        let output_only = serde_json::to_value(OmittedSchemaVariants::OutputOnly {
            entries: good_default(),
        })
        .unwrap();
        assert!(OmittedSchemaVariants::parse_input(output_only).is_err());
        // A schema-only omission does not disable the serde/runtime variant.
        assert!(
            OmittedSchemaVariants::parse_input(json!({"kind":"Hidden", "entries":["a"]})).is_ok()
        );
    }

    #[test]
    fn variant_defaults_are_visible_to_global_strict_constraints() {
        agrees::<DefaultedMode>(json!({"kind":"values"}), true);
        agrees::<DefaultedMode>(json!({"kind":"values", "entries":[]}), false);
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind")]
    enum DefaultSpelling {
        ReadHTTP {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
        #[serde(rename = "renamed_values", alias = "old_values")]
        Renamed {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum SnakeSpelling {
        ReadHTTP {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
    }

    #[test]
    fn variant_names_follow_serde_spelling_and_aliases() {
        for mut input in [
            serde_json::to_value(DefaultSpelling::ReadHTTP {
                entries: good_default(),
            })
            .unwrap(),
            serde_json::to_value(DefaultSpelling::Renamed {
                entries: good_default(),
            })
            .unwrap(),
        ] {
            input.as_object_mut().unwrap().remove("entries");
            agrees::<DefaultSpelling>(input, true);
        }
        let mut input = serde_json::to_value(SnakeSpelling::ReadHTTP {
            entries: good_default(),
        })
        .unwrap();
        input.as_object_mut().unwrap().remove("entries");
        agrees::<SnakeSpelling>(input, true);
        assert!(DefaultSpelling::parse_input(json!({"kind":"old_values"})).is_ok());
        let error = DefaultSpelling::parse_input(json!({"kind":"Renmaed"})).unwrap_err();
        assert!(error.to_string().contains("kind"));
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind", rename_all(serialize = "snake_case"))]
    enum OutputOnlySpelling {
        ReadHTTP {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
        #[serde(rename(serialize = "output_value"))]
        OutputValue {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
        #[serde(rename(deserialize = "input_value", serialize = "output_both"))]
        BothDirections {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(
        tag = "kind",
        rename_all(deserialize = "snake_case", serialize = "PascalCase")
    )]
    enum InputOnlySpelling {
        ReadHTTP {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
    }

    #[test]
    fn deserialize_names_ignore_output_only_renames() {
        for name in ["ReadHTTP", "OutputValue", "input_value"] {
            assert!(
                OutputOnlySpelling::parse_input(json!({"kind":name})).is_ok(),
                "{name}"
            );
        }
        for name in ["read_h_t_t_p", "output_value", "output_both"] {
            assert!(
                OutputOnlySpelling::parse_input(json!({"kind":name})).is_err(),
                "{name}"
            );
        }
        assert!(InputOnlySpelling::parse_input(json!({"kind":"read_h_t_t_p"})).is_ok());
        assert!(InputOnlySpelling::parse_input(json!({"kind":"ReadHTTP"})).is_err());
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind")]
    enum OtherFallback {
        Values {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
        #[serde(other)]
        Other,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind")]
    enum UntaggedFallback {
        Values {
            #[arg(default = good_default())]
            entries: Vec<String>,
        },
        #[serde(untagged)]
        Raw(serde_json::Value),
    }

    #[test]
    fn serde_fallback_variants_keep_unknown_inputs() {
        assert!(matches!(
            OtherFallback::parse_input(json!({"kind":"future"})).unwrap(),
            OtherFallback::Other
        ));
        for input in [
            json!({"kind":"future", "x":1}),
            json!({"kind":"Raw", "x":1}),
            json!({"x":1}),
        ] {
            let UntaggedFallback::Raw(raw) = UntaggedFallback::parse_input(input.clone()).unwrap()
            else {
                panic!("fallback not selected");
            };
            assert_eq!(raw, input);
        }
        assert!(matches!(
            OtherFallback::parse_input(json!({"kind":"Values"})).unwrap(),
            OtherFallback::Values { .. }
        ));
        assert!(matches!(
            UntaggedFallback::parse_input(json!({"kind":"Values"})).unwrap(),
            UntaggedFallback::Values { .. }
        ));
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "action", rename_all = "snake_case")]
    enum ActionCompatibility {
        #[input(
            default_when_empty = true,
            infer_when_present("entries"),
            drop_keys("obsolete")
        )]
        ReadValues {
            #[arg(default = good_default(), alias = "names", item_trim)]
            entries: Vec<String>,
        },
    }

    #[test]
    fn action_tag_inference_aliases_drop_and_errors_are_preserved() {
        assert!(ActionCompatibility::parse_input(json!({})).is_ok());
        let parsed =
            ActionCompatibility::parse_input(json!({"names":[" a "], "obsolete":true})).unwrap();
        let ActionCompatibility::ReadValues { entries } = parsed;
        assert_eq!(entries, ["a"]);
        let error = ActionCompatibility::parse_input(json!({"action":"read_valeus"})).unwrap_err();
        assert!(error.to_string().contains("action"));
        assert!(error.to_string().contains("read_values"));
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    struct DefaultRow {
        #[serde(default = "mixed_default")]
        entries: Vec<String>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("rows[].entries[]"))]
    struct DefaultRows {
        rows: Vec<DefaultRow>,
    }

    #[test]
    fn invalid_default_sibling_is_not_hidden_by_a_valid_match() {
        agrees::<DefaultRows>(json!({"rows":[{}, {"entries":["a"]}]}), false);
        agrees::<DefaultRows>(json!({"rows":[{"entries":[]}, {"entries":["a"]}]}), true);
        agrees::<DefaultRows>(json!({"rows":[{"entries":["b"]}, {"entries":["a"]}]}), true);
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("matrix[][]"))]
    struct MatrixItems {
        matrix: Vec<Vec<String>>,
    }

    #[test]
    fn consecutive_wildcards_share_runtime_and_schema_meaning() {
        for (matrix, accepted) in [
            (json!([]), false),
            (json!([[]]), false),
            (json!([[], []]), false),
            (json!([[], ["a"], []]), true),
            (json!([["a"], [""]]), false),
        ] {
            agrees::<MatrixItems>(json!({"matrix": matrix}), accepted);
        }
        let mut value = json!({"matrix":[[" a "],[],[" b "]]});
        super::normalize_trim_paths(&mut value, &["matrix[][]"]);
        assert_eq!(value, json!({"matrix":[["a"],[],["b"]]}));
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Row {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        children: Option<Vec<String>>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(deny_unknown_fields)]
    #[input(non_empty("rows[].children[]"))]
    struct NestedItems {
        rows: Vec<Row>,
    }

    #[test]
    fn nested_existence_allows_empty_or_missing_siblings() {
        for (rows, accepted) in [
            (json!([]), false),
            (json!([{}]), false),
            (json!([{"children":null}, {"children":[]}]), false),
            (json!([{}, {"children":[]}, {"children":["a"]}]), true),
            (json!([{"children":null}, {"children":["a"]}]), true),
            (json!([{"children":[""]}, {"children":["a"]}]), false),
        ] {
            agrees::<NestedItems>(json!({"rows": rows}), accepted);
        }
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    struct ChildItems {
        #[serde(default = "good_default")]
        entries: Vec<String>,
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    struct OptionalChild {
        children: Option<ChildItems>,
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("rows[].children.entries[]"))]
    struct OptionalChildRows {
        rows: Vec<OptionalChild>,
    }

    #[test]
    fn nullable_parent_is_no_match_instead_of_an_invalid_leaf() {
        for (rows, accepted) in [
            (json!([{"children":null}]), false),
            (
                json!([{"children":null}, {"children":{"entries":["a"]}}]),
                true,
            ),
            (json!([{}, {"children":{"entries":["a"]}}]), true),
            (json!([{"children":{}}]), true),
            (
                json!([{"children":{"entries":[""]}}, {"children":{"entries":["a"]}}]),
                false,
            ),
        ] {
            agrees::<OptionalChildRows>(json!({"rows": rows}), accepted);
        }
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    #[serde(untagged)]
    enum RuntimeSelectedDefaults {
        First {
            #[serde(default = "good_default")]
            entries: Vec<String>,
        },
        Second {
            #[serde(default = "mixed_default")]
            entries: Vec<String>,
            other: Option<bool>,
        },
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[input(non_empty("choice.entries[]"))]
    struct AmbiguousDefaultShape {
        choice: RuntimeSelectedDefaults,
    }

    #[test]
    fn opaque_branch_defaults_keep_valid_input_compatibility() {
        // JSON Schema does not describe serde's first-success selection for
        // overlapping untagged branches. Do not guess its injected defaults.
        agrees::<AmbiguousDefaultShape>(json!({"choice":{}}), true);
        assert!(AmbiguousDefaultShape::parse_input(json!({"choice":{"entries":[]}})).is_err());
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(deny_unknown_fields)]
    #[input(non_empty("entries"))]
    struct RenamedItems {
        #[serde(rename = "serialized_entries")]
        #[arg(name = "public_entries")]
        entries: Vec<String>,
    }

    #[test]
    fn source_paths_follow_serialization_and_public_field_names() {
        for (entries, accepted) in [
            (json!(["a"]), true),
            (json!([]), false),
            (json!([""]), false),
        ] {
            agrees::<RenamedItems>(json!({"public_entries": entries}), accepted);
        }
        let schema = RenamedItems::input_schema();
        assert_eq!(
            schema["properties"]["public_entries"]["items"]["minLength"],
            1
        );
    }

    #[derive(
        Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, crate::ToolInput,
    )]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum ItemMode {
        #[input(non_empty("entries"))]
        Required {
            entries: Vec<String>,
        },
        Ordinary {
            entries: Vec<String>,
        },
    }

    #[test]
    fn existence_stays_inside_its_enum_branch() {
        agrees::<ItemMode>(json!({"kind":"required", "entries":[]}), false);
        agrees::<ItemMode>(json!({"kind":"required", "entries":["a"]}), true);
        agrees::<ItemMode>(json!({"kind":"ordinary", "entries":[]}), true);
    }

    #[cfg(feature = "schema-validation")]
    #[test]
    fn existence_preserves_original_not_and_closed_properties() {
        let mut schema = json!({
            "type":"object",
            "properties": {
                "entries":{"type":"array", "items":{"type":"string"}},
                "blocked":{"type":"boolean"}
            },
            "required":["entries"],
            "additionalProperties":false,
            "not":{"properties":{"blocked":{"const":true}}, "required":["blocked"]}
        });
        super::set_schema_strict_non_empty_path_metadata(&mut schema, "", "entries[]");
        let once = schema.clone();
        super::set_schema_strict_non_empty_path_metadata(&mut schema, "", "entries[]");
        assert_eq!(schema, once);
        for (input, accepted) in [
            (json!({"entries":["a"], "blocked":false}), true),
            (json!({"entries":[]}), false),
            (json!({"entries":["a"], "blocked":true}), false),
            (json!({"entries":["a"], "extra":true}), false),
        ] {
            assert_eq!(jsonschema::is_valid(&schema, &input), accepted, "{input}");
        }
        let mut text_schema =
            json!({"type":"string", "pattern":"^[a-z ]+$", "not":{"const":"bad"}});
        super::set_schema_non_empty_metadata(&mut text_schema, "");
        assert_eq!(text_schema["pattern"], "^[a-z ]+$");
        for (input, accepted) in [
            (json!("good"), true),
            (json!("bad"), false),
            (json!("  "), false),
            (json!("123"), false),
        ] {
            assert_eq!(
                jsonschema::is_valid(&text_schema, &input),
                accepted,
                "{input}"
            );
        }
    }

    struct RequiredCommandPlugin;

    #[crate::agena_plugin(
        namespace = "pathfixture",
        name = "required",
        version = "0.0.0",
        summary = "Verify strict wildcard input paths"
    )]
    impl RequiredCommandPlugin {
        #[tool(name = "collect", summary = "Collect child values", command)]
        async fn collect(&self, input: NestedItems) -> crate::Result<String> {
            Ok(input.rows.len().to_string())
        }

        #[command(id = "collect_direct", title = "Collect child values")]
        async fn collect_direct(&self, input: NestedItems) -> crate::Result<crate::CommandResult> {
            Ok(crate::CommandResult::succeeded(
                input.rows.len().to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn typed_command_keeps_form_shape_and_authoritative_runtime_validation() {
        let plugin = RequiredCommandPlugin;
        let manifest = crate::Plugin::manifest(&plugin);
        let tool_schema = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "collect")
            .unwrap()
            .input_schema();
        assert!(tool_schema.get("not").is_some());
        let command = manifest
            .commands
            .iter()
            .find(|command| command.id == "collect_direct")
            .unwrap();
        command.input.validate().unwrap();
        let valid = json!({"rows":[{}, {"children":["a"]}]});
        command.input.validate_value(&valid).unwrap();
        let empty = json!({"rows":[]});
        #[cfg(feature = "schema-validation")]
        {
            assert!(jsonschema::is_valid(&tool_schema, &valid));
            assert!(!jsonschema::is_valid(&tool_schema, &empty));
        }
        // The closed form contract represents shape and local bounds, not
        // relational path existence. Typed command parsing still enforces it.
        command.input.validate_value(&empty).unwrap();
        let invoke = |input| crate::CommandInvokeInput {
            command_id: "collect_direct".to_owned(),
            input,
            session_id: Some(1),
            call_id: Some(1),
            workspace_root: Some("/fixture".to_owned()),
            slash: None,
            raw: String::new(),
        };
        let mut tool_command = invoke(valid.clone());
        tool_command.command_id = "collect".to_owned();
        let host_owned = crate::Plugin::command_invoke(&plugin, tool_command)
            .await
            .unwrap();
        assert_eq!(host_owned.status, crate::CommandStatus::Unavailable);
        let result = crate::Plugin::command_invoke(&plugin, invoke(valid))
            .await
            .unwrap();
        assert_eq!(result.status, crate::CommandStatus::Succeeded);
        let error = crate::Plugin::command_invoke(&plugin, invoke(empty))
            .await
            .unwrap_err();
        assert_eq!(error.kind, crate::PluginErrorKind::InvalidParams);
    }
}
