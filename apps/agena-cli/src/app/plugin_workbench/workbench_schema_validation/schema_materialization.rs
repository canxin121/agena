use super::super::{
    ConfigDiagnostic, DiagnosticSeverity, JsonMap, JsonNumber, JsonValue, active_schema_for_value,
    array_item_schema, effective_schema_kind, merge_default_value, object_property_schema,
    resolve_schema, schema_declared_property_keys, schema_required_fields, schema_type_choices,
};
use super::{schema_const_value, schema_enum_values, validate_schema_at};

pub(in crate::app) fn localized_config_schema(
    manifest: &agena::plugin::PluginManifest,
    locale: &str,
) -> Option<JsonValue> {
    let mut schema = manifest.config_schema.clone()?;
    if let Some(overlay) = manifest.config_schema_i18n.get(locale).or_else(|| {
        locale
            .split('-')
            .next()
            .and_then(|language| manifest.config_schema_i18n.get(language))
    }) {
        merge_schema_overlay(&mut schema, overlay);
    }
    Some(schema)
}

pub(in crate::app) fn merge_schema_overlay(target: &mut JsonValue, overlay: &JsonValue) {
    match (target, overlay) {
        (JsonValue::Object(target), JsonValue::Object(overlay)) => {
            for (key, overlay_value) in overlay {
                match target.get_mut(key) {
                    Some(target_value) => merge_schema_overlay(target_value, overlay_value),
                    None => {
                        target.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (target, overlay) => *target = overlay.clone(),
    }
}

pub(in crate::app) fn validate_config_value(
    schema: Option<&JsonValue>,
    value: &JsonValue,
    schema_missing: bool,
) -> Vec<ConfigDiagnostic> {
    if value.is_null() {
        return Vec::new();
    }
    if schema_missing {
        return vec![ConfigDiagnostic {
            severity: DiagnosticSeverity::Warning,
            source: "config".to_owned(),
            path: Vec::new(),
            field: "Config".to_owned(),
            message: "schema missing; using generic structured editor".to_owned(),
        }];
    }
    let Some(schema) = schema else {
        return Vec::new();
    };
    let mut diagnostics = Vec::new();
    validate_schema_at(
        &mut diagnostics,
        schema,
        schema,
        value,
        &Vec::new(),
        "Config",
    );
    diagnostics
}

pub(in crate::app) fn materialized_config_value(
    schema: Option<&JsonValue>,
    value: &JsonValue,
) -> JsonValue {
    let Some(schema) = schema else {
        return value.clone();
    };
    let mut materialized = materialized_value_for_schema(schema, schema);
    if !value.is_null() {
        merge_config_override(&mut materialized, value);
    }
    materialize_schema_fields(&mut materialized, schema, schema);
    materialized
}

pub(in crate::app) fn merge_config_override(target: &mut JsonValue, override_value: &JsonValue) {
    match (target, override_value) {
        (JsonValue::Object(target), JsonValue::Object(override_object)) => {
            for (key, value) in override_object {
                match target.get_mut(key) {
                    Some(existing) => merge_config_override(existing, value),
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, value) => *target = value.clone(),
    }
}

pub(in crate::app) fn materialized_string_value_for_schema(schema: &JsonValue) -> String {
    let min_length = schema
        .get("minLength")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default() as usize;
    if min_length == 0 {
        String::new()
    } else {
        "x".repeat(min_length)
    }
}

pub(in crate::app) fn materialized_numeric_value_for_schema(
    schema: &JsonValue,
    integer: bool,
) -> JsonValue {
    let multiple_of = schema
        .get("multipleOf")
        .and_then(JsonValue::as_f64)
        .filter(|value| *value > 0.0);
    let step = if integer {
        1.0
    } else {
        multiple_of.unwrap_or(1.0)
    };
    let mut candidate = 0.0_f64;
    if let Some(minimum) = schema.get("minimum").and_then(JsonValue::as_f64) {
        candidate = candidate.max(minimum);
    }
    if let Some(exclusive_minimum) = schema.get("exclusiveMinimum").and_then(JsonValue::as_f64) {
        candidate = candidate.max(exclusive_minimum + step);
    }
    if let Some(multiple_of) = multiple_of {
        candidate = (candidate / multiple_of).ceil() * multiple_of;
    }
    if integer {
        JsonValue::Number(JsonNumber::from(candidate.ceil() as i64))
    } else {
        JsonValue::Number(JsonNumber::from_f64(candidate).unwrap_or_else(|| JsonNumber::from(0)))
    }
}

pub(in crate::app) fn materialized_value_for_schema(
    schema: &JsonValue,
    root: &JsonValue,
) -> JsonValue {
    let schema = resolve_schema(root, schema);
    if let Some(default) = schema.get("default") {
        return default.clone();
    }
    if let Some(constant) = schema_const_value(schema) {
        return constant;
    }
    if schema_type_choices(schema)
        .iter()
        .any(|kind| kind == "null")
    {
        return JsonValue::Null;
    }
    if let Some(variants) = schema_enum_values(schema)
        && let Some(first) = variants.first()
    {
        return first.clone();
    }
    if let Some(branches) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(JsonValue::as_array)
        && let Some(first) = branches.first()
    {
        return materialized_value_for_schema(first, root);
    }
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        let mut value = JsonValue::Object(JsonMap::new());
        for branch in all_of {
            let branch_value = materialized_value_for_schema(branch, root);
            merge_default_value(&mut value, branch_value);
        }
        materialize_schema_fields(&mut value, schema, root);
        return value;
    }
    match effective_schema_kind(schema).as_deref() {
        Some("object") => {
            let mut object = JsonMap::new();
            let required = schema_required_fields(schema);
            for key in schema_declared_property_keys(schema) {
                let Some(child_schema) = object_property_schema(root, schema, key.as_str()) else {
                    continue;
                };
                if required.contains(key.as_str())
                    || schema_prefers_materialized_presence(&child_schema, root)
                {
                    object.insert(
                        key.clone(),
                        materialized_value_for_schema(&child_schema, root),
                    );
                }
            }
            JsonValue::Object(object)
        }
        Some("array") => JsonValue::Array(Vec::new()),
        Some("string") => JsonValue::String(materialized_string_value_for_schema(schema)),
        Some("integer") => materialized_numeric_value_for_schema(schema, true),
        Some("number") => materialized_numeric_value_for_schema(schema, false),
        Some("boolean") => JsonValue::Bool(false),
        Some("null") => JsonValue::Null,
        _ => JsonValue::Null,
    }
}

pub(in crate::app) fn materialize_schema_fields(
    value: &mut JsonValue,
    schema: &JsonValue,
    root: &JsonValue,
) {
    let schema = active_schema_for_value(root, schema, value);
    match effective_schema_kind(&schema).as_deref() {
        Some("object") => {
            let JsonValue::Object(object) = value else {
                return;
            };
            let required = schema_required_fields(&schema);
            for key in schema_declared_property_keys(&schema) {
                let Some(child_schema) = object_property_schema(root, &schema, key.as_str()) else {
                    continue;
                };
                if required.contains(key.as_str())
                    || schema_prefers_materialized_presence(&child_schema, root)
                {
                    let child = object
                        .entry(key.clone())
                        .or_insert_with(|| materialized_value_for_schema(&child_schema, root));
                    materialize_schema_fields(child, &child_schema, root);
                }
            }
        }
        Some("array") => {
            let JsonValue::Array(items) = value else {
                return;
            };
            for (index, item) in items.iter_mut().enumerate() {
                if let Some(item_schema) = array_item_schema(root, &schema, index) {
                    materialize_schema_fields(item, &item_schema, root);
                }
            }
        }
        _ => {}
    }
}

pub(in crate::app) fn schema_prefers_materialized_presence(
    schema: &JsonValue,
    root: &JsonValue,
) -> bool {
    let schema = resolve_schema(root, schema);
    if schema.get("default").is_some() || schema_const_value(schema).is_some() {
        return true;
    }
    if schema_enum_values(schema).is_some() {
        return true;
    }
    if schema.get("oneOf").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("allOf").is_some()
    {
        return true;
    }
    matches!(
        effective_schema_kind(schema).as_deref(),
        Some("object") | Some("array")
    )
}
