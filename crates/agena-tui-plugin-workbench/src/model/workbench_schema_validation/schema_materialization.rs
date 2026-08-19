use super::super::{
    ConfigDiagnostic, DiagnosticSeverity, JsonMap, JsonNumber, JsonValue, active_schema_for_value,
    array_item_schema, effective_schema_kind, merge_default_value, object_property_schema,
    resolve_schema, schema_declared_property_keys, schema_required_fields, schema_type_choices,
};
use super::{schema_const_value, schema_enum_values, validate_schema_at};

/// Convert the closed SettingsContract AST into the workbench's internal
/// editor shape. This adapter never accepts plugin-authored JSON Schema: every
/// keyword below is generated from the bounded wire contract.
pub(crate) fn plugin_settings_schema(
    manifest: &agena_plugin_host::PluginManifest,
) -> Option<JsonValue> {
    manifest
        .settings
        .as_ref()
        .map(settings_contract_editor_schema)
}

pub(crate) fn settings_contract_editor_schema(
    contract: &agena_plugin_host::sdk::SettingsContract,
) -> JsonValue {
    let mut schema = settings_node_editor_schema(&contract.root);
    if let Ok(default) = contract.default_value()
        && let Some(object) = schema.as_object_mut()
    {
        object.insert("default".to_string(), default);
    }
    schema
}

fn settings_node_editor_schema(node: &agena_plugin_host::sdk::SettingsNode) -> JsonValue {
    use agena_plugin_host::sdk::SettingsNodeKind;

    let mut schema = JsonMap::new();
    schema.insert("title".to_string(), JsonValue::String(node.title.clone()));
    if !node.description.is_empty() {
        schema.insert(
            "description".to_string(),
            JsonValue::String(node.description.clone()),
        );
    }
    if let Some(default) = &node.default {
        schema.insert("default".to_string(), default.clone());
    }
    if node.sensitive || node.secret {
        schema.insert("writeOnly".to_string(), JsonValue::Bool(true));
    }
    let constraints = &node.constraints;
    for (key, value) in [
        ("minimum", constraints.minimum),
        ("maximum", constraints.maximum),
        ("exclusiveMinimum", constraints.exclusive_minimum),
        ("exclusiveMaximum", constraints.exclusive_maximum),
        ("multipleOf", constraints.multiple_of),
    ] {
        if let Some(value) = value.and_then(JsonNumber::from_f64) {
            schema.insert(key.to_string(), JsonValue::Number(value));
        }
    }
    for (key, value) in [
        ("minLength", constraints.min_length),
        ("maxLength", constraints.max_length),
        ("minItems", constraints.min_items),
        ("maxItems", constraints.max_items),
        ("maxProperties", constraints.max_entries),
    ] {
        if let Some(value) = value {
            schema.insert(key.to_string(), JsonValue::Number(value.into()));
        }
    }

    match &node.kind {
        SettingsNodeKind::Boolean => {
            schema.insert("type".to_string(), JsonValue::String("boolean".to_string()));
        }
        SettingsNodeKind::Text | SettingsNodeKind::SecretReference => {
            schema.insert("type".to_string(), JsonValue::String("string".to_string()));
        }
        SettingsNodeKind::Integer => {
            schema.insert("type".to_string(), JsonValue::String("integer".to_string()));
        }
        SettingsNodeKind::Number => {
            schema.insert("type".to_string(), JsonValue::String("number".to_string()));
        }
        SettingsNodeKind::Choice { options } => {
            schema.insert(
                "enum".to_string(),
                JsonValue::Array(options.iter().map(|option| option.value.clone()).collect()),
            );
            schema.insert(
                "x-agena-option-labels".to_string(),
                JsonValue::Array(
                    options
                        .iter()
                        .map(|option| JsonValue::String(option.title.clone()))
                        .collect(),
                ),
            );
        }
        SettingsNodeKind::MultiChoice { options } => {
            schema.insert("type".to_string(), JsonValue::String("array".to_string()));
            schema.insert(
                "items".to_string(),
                serde_json::json!({
                    "enum": options.iter().map(|option| option.value.clone()).collect::<Vec<_>>()
                }),
            );
            schema.insert("uniqueItems".to_string(), JsonValue::Bool(true));
        }
        SettingsNodeKind::Path { path_kind } => {
            schema.insert("type".to_string(), JsonValue::String("string".to_string()));
            schema.insert("format".to_string(), JsonValue::String("path".to_string()));
            schema.insert(
                "x-agena-path-kind".to_string(),
                serde_json::to_value(path_kind).unwrap_or(JsonValue::Null),
            );
        }
        SettingsNodeKind::Url => {
            schema.insert("type".to_string(), JsonValue::String("string".to_string()));
            schema.insert("format".to_string(), JsonValue::String("uri".to_string()));
        }
        SettingsNodeKind::Duration => {
            schema.insert("type".to_string(), JsonValue::String("string".to_string()));
            schema.insert(
                "format".to_string(),
                JsonValue::String("duration".to_string()),
            );
        }
        SettingsNodeKind::Object { fields } => {
            schema.insert("type".to_string(), JsonValue::String("object".to_string()));
            schema.insert(
                "properties".to_string(),
                JsonValue::Object(
                    fields
                        .iter()
                        .map(|field| (field.id.clone(), settings_node_editor_schema(field)))
                        .collect(),
                ),
            );
            let required = fields
                .iter()
                .filter(|field| field.required)
                .map(|field| JsonValue::String(field.id.clone()))
                .collect::<Vec<_>>();
            if !required.is_empty() {
                schema.insert("required".to_string(), JsonValue::Array(required));
            }
            schema.insert("additionalProperties".to_string(), JsonValue::Bool(false));
        }
        SettingsNodeKind::List { item } => {
            schema.insert("type".to_string(), JsonValue::String("array".to_string()));
            schema.insert("items".to_string(), settings_node_editor_schema(item));
        }
        SettingsNodeKind::Record { value } => {
            schema.insert("type".to_string(), JsonValue::String("object".to_string()));
            schema.insert(
                "additionalProperties".to_string(),
                settings_node_editor_schema(value),
            );
        }
        SettingsNodeKind::TaggedVariant {
            discriminator,
            variants,
        } => {
            schema.insert(
                "oneOf".to_string(),
                JsonValue::Array(
                    variants
                        .iter()
                        .map(|variant| {
                            let mut properties = JsonMap::from_iter([(
                                discriminator.clone(),
                                serde_json::json!({
                                    "title": discriminator,
                                    "const": variant.tag,
                                    "readOnly": true
                                }),
                            )]);
                            for field in &variant.fields {
                                properties
                                    .insert(field.id.clone(), settings_node_editor_schema(field));
                            }
                            let mut required = vec![JsonValue::String(discriminator.clone())];
                            required.extend(
                                variant
                                    .fields
                                    .iter()
                                    .filter(|field| field.required)
                                    .map(|field| JsonValue::String(field.id.clone())),
                            );
                            serde_json::json!({
                                "title": variant.title,
                                "description": variant.description,
                                "type": "object",
                                "properties": properties,
                                "required": required,
                                "additionalProperties": false
                            })
                        })
                        .collect(),
                ),
            );
        }
        SettingsNodeKind::Json {
            max_bytes,
            max_depth,
        } => {
            schema.insert(
                "x-agena-editor".to_string(),
                JsonValue::String("json".to_string()),
            );
            schema.insert(
                "x-agena-json-max-bytes".to_string(),
                JsonValue::Number((*max_bytes).into()),
            );
            schema.insert(
                "x-agena-json-max-depth".to_string(),
                JsonValue::Number((*max_depth).into()),
            );
        }
    }
    JsonValue::Object(schema)
}

pub(crate) fn validate_config_value(
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

pub(crate) fn materialized_config_value(
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

pub(crate) fn merge_config_override(target: &mut JsonValue, override_value: &JsonValue) {
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

pub(crate) fn materialized_string_value_for_schema(schema: &JsonValue) -> String {
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

pub(crate) fn materialized_numeric_value_for_schema(
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

pub(crate) fn materialized_value_for_schema(schema: &JsonValue, root: &JsonValue) -> JsonValue {
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

pub(crate) fn materialize_schema_fields(
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

pub(crate) fn schema_prefers_materialized_presence(schema: &JsonValue, root: &JsonValue) -> bool {
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
