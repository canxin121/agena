//! Internal compilation of typed Rust schemas into the closed plugin settings
//! contract. JSON Schema is an implementation detail and is never put on the
//! manifest wire.

use std::collections::BTreeSet;

use agena_plugin_contracts::{
    MAX_JSON_ESCAPE_BYTES, MAX_JSON_ESCAPE_DEPTH, OperationDiscoverability, PathInputKind,
    SettingsConstraints, SettingsContract, SettingsNode, SettingsNodeKind, SettingsOption,
    SettingsVariant,
};
use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::macro_support::normalize_settings_schema_json;

const JSON_ESCAPE_MARKER: &str = "x-agena-json-escape";
const JSON_ESCAPE_BYTES_MARKER: &str = "x-agena-json-max-bytes";
const JSON_ESCAPE_DEPTH_MARKER: &str = "x-agena-json-max-depth";
const PATH_KIND_MARKER: &str = "x-agena-path-kind";
const SENSITIVE_MARKER: &str = "x-agena-sensitive";
const SECRET_MARKER: &str = "x-agena-secret-reference";
const ORDER_MARKER: &str = "x-order";

/// Schemars `schema_with` helper for fields that intentionally contain
/// arbitrary JSON. Using this function is the explicit opt-in required by the
/// closed settings compiler; the resulting contract carries hard byte/depth
/// bounds instead of exposing an unbounded JSON-Schema escape hatch.
pub fn bounded_json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let mut schema = serde_json::Map::new();
    schema.insert(JSON_ESCAPE_MARKER.to_string(), Value::Bool(true));
    schema.insert(
        JSON_ESCAPE_BYTES_MARKER.to_string(),
        Value::from(MAX_JSON_ESCAPE_BYTES),
    );
    schema.insert(
        JSON_ESCAPE_DEPTH_MARKER.to_string(),
        Value::from(MAX_JSON_ESCAPE_DEPTH),
    );
    schema.into()
}

pub fn settings_contract_for<T>() -> Result<SettingsContract, String>
where
    T: JsonSchema,
{
    let schema = serde_json::to_value(schema_for!(T))
        .map_err(|error| format!("serialize generated settings schema: {error}"))?;
    let schema = normalize_settings_schema_json(schema);
    settings_contract_from_schema(&schema)
}

pub fn settings_contract_for_default<T>(default: T) -> Result<SettingsContract, String>
where
    T: JsonSchema + Serialize,
{
    let mut schema = serde_json::to_value(schema_for!(T))
        .map_err(|error| format!("serialize generated settings schema: {error}"))?;
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.insert(
            "default".to_string(),
            serde_json::to_value(default)
                .map_err(|error| format!("serialize settings default: {error}"))?,
        );
    }
    let schema = normalize_settings_schema_json(schema);
    settings_contract_from_schema(&schema)
}

pub fn settings_contract_from_schema(schema: &Value) -> Result<SettingsContract, String> {
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let root = compile_node(&schema, &defs, "root", "", true, &mut Vec::new())?;
    let contract = SettingsContract::new(root);
    contract
        .validate()
        .map_err(|error| format!("invalid compiled settings contract: {error}"))?;
    Ok(contract)
}

/// Apply presentation-only metadata to an already compiled settings contract.
/// Paths are the closed contract paths (`/fetch/request/timeout_secs`, not
/// JSON-Schema `/properties/...` paths). The decorator cannot add fields,
/// change node kinds, alter defaults/constraints, or relax validation.
pub fn decorate_settings_contract(
    mut contract: SettingsContract,
    metadata: &[(&str, &str, &str)],
) -> Result<SettingsContract, String> {
    for (path, title, description) in metadata {
        let mut matches = 0usize;
        decorate_node_metadata(&mut contract.root, path, title, description, &mut matches);
        if matches == 0 {
            return Err(format!(
                "settings metadata path `{path}` does not exist in the typed contract"
            ));
        }
    }
    contract
        .validate()
        .map_err(|error| format!("invalid decorated settings contract: {error}"))?;
    Ok(contract)
}

fn decorate_node_metadata(
    node: &mut SettingsNode,
    path: &str,
    title: &str,
    description: &str,
    matches: &mut usize,
) {
    if node.path == path {
        node.title = title.to_string();
        node.description = description.to_string();
        *matches += 1;
    }
    match &mut node.kind {
        SettingsNodeKind::Object { fields } => {
            for field in fields {
                decorate_node_metadata(field, path, title, description, matches);
            }
        }
        SettingsNodeKind::List { item } => {
            decorate_node_metadata(item, path, title, description, matches);
        }
        SettingsNodeKind::Record { value } => {
            decorate_node_metadata(value, path, title, description, matches);
        }
        SettingsNodeKind::TaggedVariant { variants, .. } => {
            for variant in variants {
                for field in &mut variant.fields {
                    decorate_node_metadata(field, path, title, description, matches);
                }
            }
        }
        SettingsNodeKind::Boolean
        | SettingsNodeKind::Text
        | SettingsNodeKind::SecretReference
        | SettingsNodeKind::Integer
        | SettingsNodeKind::Number
        | SettingsNodeKind::Choice { .. }
        | SettingsNodeKind::MultiChoice { .. }
        | SettingsNodeKind::Path { .. }
        | SettingsNodeKind::Url
        | SettingsNodeKind::Duration
        | SettingsNodeKind::Json { .. } => {}
    }
}

fn compile_node(
    schema: &Value,
    defs: &Map<String, Value>,
    id: &str,
    path: &str,
    required: bool,
    refs: &mut Vec<String>,
) -> Result<SettingsNode, String> {
    let schema = resolve_schema(schema, defs, refs)?;
    let schema = unwrap_nullable(schema, defs, refs)?;

    if let Some(composition) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
    {
        return compile_composition(&schema, composition, defs, id, path, required, refs);
    }
    if let Some(composition) = schema.get("allOf").and_then(Value::as_array) {
        let merged = merge_object_composition(composition, defs, refs)?;
        return compile_node(&merged, defs, id, path, required, refs);
    }

    let metadata = NodeMetadata::from_schema(&schema, id, path, required);
    if let Some(options) = enum_options(&schema)? {
        return Ok(metadata.finish(SettingsNodeKind::Choice { options }));
    }
    if schema.get(JSON_ESCAPE_MARKER).and_then(Value::as_bool) == Some(true) {
        let max_bytes = bounded_u32(
            schema.get(JSON_ESCAPE_BYTES_MARKER),
            MAX_JSON_ESCAPE_BYTES,
            JSON_ESCAPE_BYTES_MARKER,
        )?
        .unwrap_or(MAX_JSON_ESCAPE_BYTES);
        let max_depth = bounded_u8(
            schema.get(JSON_ESCAPE_DEPTH_MARKER),
            MAX_JSON_ESCAPE_DEPTH,
            JSON_ESCAPE_DEPTH_MARKER,
        )?;
        return Ok(metadata.finish(SettingsNodeKind::Json {
            max_bytes,
            max_depth,
        }));
    }

    let type_name = schema_type(&schema)?;
    match type_name {
        Some("boolean") => Ok(metadata.finish(SettingsNodeKind::Boolean)),
        Some("integer") => {
            Ok(metadata.finish_with_constraints(SettingsNodeKind::Integer, constraints(&schema)?))
        }
        Some("number") => {
            Ok(metadata.finish_with_constraints(SettingsNodeKind::Number, constraints(&schema)?))
        }
        Some("string") => {
            if let Some(format) = schema.get("format").and_then(Value::as_str) {
                let kind = match format {
                    "uri" | "uri-reference" | "url" => SettingsNodeKind::Url,
                    "duration" | "duration-string" => SettingsNodeKind::Duration,
                    "path" | "filesystem-path" | "file-path" | "directory-path" => {
                        SettingsNodeKind::Path {
                            path_kind: path_kind(&schema, format),
                        }
                    }
                    "password" | "secret" => {
                        return Ok(metadata
                            .sensitive(true)
                            .secret(true)
                            .finish(SettingsNodeKind::SecretReference));
                    }
                    _ => SettingsNodeKind::Text,
                };
                return Ok(metadata.finish_with_constraints(kind, constraints(&schema)?));
            }
            Ok(metadata.finish_with_constraints(SettingsNodeKind::Text, constraints(&schema)?))
        }
        Some("array") => {
            if schema.get("prefixItems").is_some() || schema.get("contains").is_some() {
                return Err(unsupported(
                    path,
                    "tuple/contains array schemas are not supported",
                ));
            }
            let item_schema = schema.get("items").ok_or_else(|| {
                unsupported(path, "array settings require one homogeneous items schema")
            })?;
            if item_schema.is_boolean() && item_schema == &Value::Bool(false) {
                return Err(unsupported(
                    path,
                    "array items=false is not a settings form",
                ));
            }
            let item = compile_node(
                item_schema,
                defs,
                &format!("{id}_item"),
                &format!("{path}/*"),
                true,
                refs,
            )?;
            Ok(metadata.finish_with_constraints(
                SettingsNodeKind::List {
                    item: Box::new(item),
                },
                constraints(&schema)?,
            ))
        }
        Some("object") => compile_object(&schema, defs, id, path, required, refs, metadata),
        None => Err(unsupported(
            path,
            "schema has no supported type; declare an explicit bounded JSON escape field for arbitrary JSON",
        )),
        Some(other) => Err(unsupported(
            path,
            format!("unsupported schema type `{other}`"),
        )),
    }
}

fn compile_object(
    schema: &Value,
    defs: &Map<String, Value>,
    id: &str,
    path: &str,
    _required: bool,
    refs: &mut Vec<String>,
    metadata: NodeMetadata,
) -> Result<SettingsNode, String> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required_fields = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();

    match schema.get("additionalProperties") {
        Some(Value::Bool(true)) => {
            return Err(unsupported(
                path,
                "unbounded record settings require an explicit value schema",
            ));
        }
        Some(Value::Object(value_schema)) => {
            if !properties.is_empty() {
                return Err(unsupported(
                    path,
                    "mixed fixed properties and record values are not supported",
                ));
            }
            let value = compile_node(
                &Value::Object(value_schema.clone()),
                defs,
                &format!("{id}_value"),
                &format!("{path}/*"),
                true,
                refs,
            )?;
            return Ok(metadata.finish_with_constraints(
                SettingsNodeKind::Record {
                    value: Box::new(value),
                },
                constraints(&schema)?,
            ));
        }
        Some(Value::Bool(false)) | None => {}
        Some(_) => return Err(unsupported(path, "invalid additionalProperties schema")),
    }

    let mut ordered = properties.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|(left_id, left), (right_id, right)| {
        schema_order(left)
            .cmp(&schema_order(right))
            .then_with(|| left_id.cmp(right_id))
    });
    let fields = ordered
        .into_iter()
        .map(|(field_id, field_schema)| {
            let child_path = format!("{path}/{}", escape_pointer_segment(&field_id));
            compile_node(
                &field_schema,
                defs,
                &field_id,
                &child_path,
                required_fields.contains(field_id.as_str()),
                refs,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(metadata.finish_with_constraints(SettingsNodeKind::Object { fields }, constraints(schema)?))
}

fn compile_composition(
    outer: &Value,
    variants: &[Value],
    defs: &Map<String, Value>,
    id: &str,
    path: &str,
    required: bool,
    refs: &mut Vec<String>,
) -> Result<SettingsNode, String> {
    let non_null = variants
        .iter()
        .filter(|variant| variant.get("type") != Some(&Value::String("null".to_string())))
        .collect::<Vec<_>>();
    if non_null.len() != variants.len() {
        if non_null.len() == 1 {
            return compile_node(non_null[0], defs, id, path, false, refs);
        }
        return Err(unsupported(
            path,
            "nullable union contains more than one non-null shape",
        ));
    }

    let resolved = non_null
        .iter()
        .map(|variant| resolve_schema(variant, defs, refs))
        .collect::<Result<Vec<_>, _>>()?;
    let discriminator = find_discriminator(&resolved);
    let Some(discriminator) = discriminator else {
        return Err(unsupported(
            path,
            "only tagged variants are supported; general oneOf/anyOf is not a settings form",
        ));
    };

    let mut compiled_variants = Vec::with_capacity(resolved.len());
    for (index, variant) in resolved.iter().enumerate() {
        let object = variant
            .as_object()
            .ok_or_else(|| unsupported(path, "tagged variant must contain object variants"))?;
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| unsupported(path, "tagged variant must contain object properties"))?;
        let tag_schema = properties
            .get(&discriminator)
            .ok_or_else(|| unsupported(path, "tagged variant discriminator is missing"))?;
        let tag = tag_value(tag_schema).ok_or_else(|| {
            unsupported(
                path,
                "tagged variant discriminator must be const or single-valued enum",
            )
        })?;
        let tag_id = tag
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("variant_{index}"));
        let required_fields = object
            .get("required")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let mut fields = properties
            .iter()
            .filter(|(field_id, _)| *field_id != &discriminator)
            .map(|(field_id, field_schema)| {
                let field_path = format!("{path}/{}", escape_pointer_segment(field_id));
                compile_node(
                    field_schema,
                    defs,
                    field_id,
                    &field_path,
                    required_fields.contains(field_id.as_str()),
                    refs,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        fields.sort_by(|left, right| left.path.cmp(&right.path));
        compiled_variants.push(SettingsVariant {
            id: tag_id,
            title: variant
                .get("title")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| humanize(&tag)),
            description: variant
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tag,
            fields,
        });
    }
    let metadata = NodeMetadata::from_schema(outer, id, path, required);
    Ok(metadata.finish(SettingsNodeKind::TaggedVariant {
        discriminator,
        variants: compiled_variants,
    }))
}

fn find_discriminator(variants: &[Value]) -> Option<String> {
    let first = variants.first()?.as_object()?;
    let first_properties = first.get("properties")?.as_object()?;
    let candidates = first_properties.keys().filter_map(|key| {
        let first_tag = tag_value(first_properties.get(key)?)?;
        variants
            .iter()
            .all(|variant| {
                variant
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get(key))
                    .and_then(tag_value)
                    .is_some()
            })
            .then_some((key.clone(), first_tag))
    });
    candidates.map(|(key, _)| key).next()
}

fn tag_value(schema: &Value) -> Option<Value> {
    schema.get("const").cloned().or_else(|| {
        schema
            .get("enum")
            .and_then(Value::as_array)
            .filter(|values| values.len() == 1)
            .and_then(|values| values.first().cloned())
    })
}

fn enum_options(schema: &Value) -> Result<Option<Vec<SettingsOption>>, String> {
    let Some(values) = schema.get("enum").and_then(Value::as_array) else {
        return Ok(None);
    };
    if values.is_empty() {
        return Err(unsupported("", "settings enum cannot be empty"));
    }
    let options = values
        .iter()
        .enumerate()
        .map(|(index, value)| SettingsOption {
            id: option_id(value, index),
            title: humanize(value),
            description: String::new(),
            value: value.clone(),
        })
        .collect();
    Ok(Some(options))
}

fn constraints(schema: &Value) -> Result<SettingsConstraints, String> {
    let mut constraints = SettingsConstraints {
        minimum: number(schema.get("minimum"))?,
        maximum: number(schema.get("maximum"))?,
        exclusive_minimum: number(schema.get("exclusiveMinimum"))?,
        exclusive_maximum: number(schema.get("exclusiveMaximum"))?,
        multiple_of: number(schema.get("multipleOf"))?,
        min_length: bounded_u32(schema.get("minLength"), u32::MAX, "minLength")?,
        max_length: bounded_u32(schema.get("maxLength"), u32::MAX, "maxLength")?,
        pattern: schema
            .get("pattern")
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| "settings pattern must be a string".to_string())
            })
            .transpose()?,
        min_items: bounded_u32(schema.get("minItems"), u32::MAX, "minItems")?,
        max_items: bounded_u32(schema.get("maxItems"), u32::MAX, "maxItems")?,
        max_entries: bounded_u32(schema.get("maxProperties"), u32::MAX, "maxProperties")?,
    };
    if let Some(exclusive) = schema.get("exclusiveMinimum").and_then(Value::as_bool)
        && exclusive
    {
        return Err(unsupported(
            "",
            "boolean exclusiveMinimum is not supported by the settings compiler",
        ));
    }
    if let Some(exclusive) = schema.get("exclusiveMaximum").and_then(Value::as_bool)
        && exclusive
    {
        return Err(unsupported(
            "",
            "boolean exclusiveMaximum is not supported by the settings compiler",
        ));
    }
    if constraints.max_entries.is_none() {
        constraints.max_entries =
            bounded_u32(schema.get("maxProperties"), u32::MAX, "maxProperties")?;
    }
    Ok(constraints)
}

fn number(value: Option<&Value>) -> Result<Option<f64>, String> {
    let Some(value) = value else { return Ok(None) };
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .map(Some)
        .ok_or_else(|| "settings numeric constraint must be a finite number".to_string())
}

fn schema_type(schema: &Value) -> Result<Option<&str>, String> {
    match schema.get("type") {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(Value::Array(values)) => {
            let non_null = values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| *value != "null")
                .collect::<Vec<_>>();
            if non_null.len() == 1 {
                Ok(Some(non_null[0]))
            } else {
                Err("settings type unions must be nullable or tagged variants".to_string())
            }
        }
        Some(_) => Err("settings schema type must be a string".to_string()),
    }
}

fn resolve_schema(
    schema: &Value,
    defs: &Map<String, Value>,
    refs: &mut Vec<String>,
) -> Result<Value, String> {
    let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
        return Ok(schema.clone());
    };
    if refs.iter().any(|seen| seen == reference) {
        return Err(unsupported(
            "",
            format!("recursive settings schema reference `{reference}`"),
        ));
    }
    let name = reference
        .strip_prefix("#/$defs/")
        .or_else(|| reference.strip_prefix("#/definitions/"))
        .ok_or_else(|| {
            unsupported(
                "",
                format!("external settings schema reference `{reference}`"),
            )
        })?;
    let target = defs.get(name).ok_or_else(|| {
        unsupported(
            "",
            format!("unresolved settings schema reference `{reference}`"),
        )
    })?;
    refs.push(reference.to_string());
    let mut resolved = resolve_schema(target, defs, refs)?;
    refs.pop();
    if let (Some(base), Some(overlay)) = (resolved.as_object_mut(), schema.as_object()) {
        for (key, value) in overlay {
            if key != "$ref" {
                base.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(resolved)
}

fn unwrap_nullable(
    schema: Value,
    defs: &Map<String, Value>,
    refs: &mut Vec<String>,
) -> Result<Value, String> {
    let Some(composition) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    else {
        return Ok(schema);
    };
    let non_null = composition
        .iter()
        .filter(|variant| variant.get("type") != Some(&Value::String("null".to_string())))
        .collect::<Vec<_>>();
    if non_null.len() == 1 && non_null.len() != composition.len() {
        let mut resolved = resolve_schema(non_null[0], defs, refs)?;
        if let (Some(base), Some(outer)) = (resolved.as_object_mut(), schema.as_object()) {
            for (key, value) in outer {
                if key != "anyOf" && key != "oneOf" {
                    base.insert(key.clone(), value.clone());
                }
            }
        }
        return Ok(resolved);
    }
    Ok(schema)
}

fn merge_object_composition(
    composition: &[Value],
    defs: &Map<String, Value>,
    refs: &mut Vec<String>,
) -> Result<Value, String> {
    let mut merged = Map::new();
    let mut properties = Map::new();
    let mut required = Vec::new();
    for item in composition {
        let item = resolve_schema(item, defs, refs)?;
        if item.get("type") != Some(&Value::String("object".to_string())) {
            return Err(unsupported("", "only object allOf schemas can be compiled"));
        }
        if let Some(item_properties) = item.get("properties").and_then(Value::as_object) {
            properties.extend(item_properties.clone());
        }
        if let Some(item_required) = item.get("required").and_then(Value::as_array) {
            required.extend(item_required.clone());
        }
        for (key, value) in item
            .as_object()
            .into_iter()
            .flat_map(|object| object.iter())
        {
            if key != "properties" && key != "required" {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged.insert("type".to_string(), Value::String("object".to_string()));
    merged.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        merged.insert("required".to_string(), Value::Array(required));
    }
    Ok(Value::Object(merged))
}

fn schema_order(schema: &Value) -> u64 {
    schema
        .get(ORDER_MARKER)
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
}

fn path_kind(schema: &Value, format: &str) -> PathInputKind {
    match schema
        .get(PATH_KIND_MARKER)
        .and_then(Value::as_str)
        .unwrap_or(format)
    {
        "file" | "file-path" => PathInputKind::File,
        "directory" | "directory-path" => PathInputKind::Directory,
        _ => PathInputKind::Any,
    }
}

fn option_id(value: &Value, index: usize) -> String {
    let raw = value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("option_{index}"));
    let mut id = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    if id.is_empty() {
        id = format!("option_{index}");
    }
    id
}

fn humanize(value: &Value) -> String {
    let raw = value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string());
    humanize_identifier(&raw)
}

fn humanize_identifier(raw: &str) -> String {
    let mut words = Vec::<String>::new();
    let mut current = String::new();
    let chars = raw.chars().collect::<Vec<_>>();
    for (index, ch) in chars.iter().copied().enumerate() {
        if matches!(ch, '_' | '-' | '.' | '/' | ':') || ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let camel_boundary = ch.is_uppercase()
            && previous
                .is_some_and(|previous| previous.is_lowercase() || previous.is_ascii_digit());
        let acronym_boundary = ch.is_uppercase()
            && previous.is_some_and(char::is_uppercase)
            && next.is_some_and(char::is_lowercase);
        if (camel_boundary || acronym_boundary) && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .into_iter()
        .map(|word| {
            let lowered = word.to_ascii_lowercase();
            if matches!(
                lowered.as_str(),
                "api"
                    | "cpu"
                    | "gpu"
                    | "http"
                    | "https"
                    | "id"
                    | "ip"
                    | "json"
                    | "lsp"
                    | "mcp"
                    | "oauth"
                    | "ttl"
                    | "ui"
                    | "url"
            ) {
                return lowered.to_ascii_uppercase();
            }
            if word
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
            {
                return word;
            }
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn bounded_u32(value: Option<&Value>, max: u32, label: &str) -> Result<Option<u32>, String> {
    let Some(value) = value else { return Ok(None) };
    let value = value
        .as_u64()
        .ok_or_else(|| format!("settings {label} must be an unsigned integer"))?;
    if value > max as u64 {
        return Err(format!("settings {label} exceeds {max}"));
    }
    Ok(Some(value as u32))
}

fn bounded_u8(value: Option<&Value>, max: u8, label: &str) -> Result<u8, String> {
    let value = value.and_then(Value::as_u64).unwrap_or(max as u64);
    if value == 0 || value > max as u64 {
        return Err(format!("settings {label} must be in 1..={max}"));
    }
    Ok(value as u8)
}

fn escape_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn unsupported(path: impl AsRef<str>, message: impl Into<String>) -> String {
    let path = path.as_ref();
    if path.is_empty() {
        message.into()
    } else {
        format!("{path}: {}", message.into())
    }
}

struct NodeMetadata {
    id: String,
    path: String,
    title: String,
    description: String,
    required: bool,
    default: Option<Value>,
    sensitive: bool,
    secret: bool,
}

impl NodeMetadata {
    fn from_schema(schema: &Value, id: &str, path: &str, required: bool) -> Self {
        Self {
            id: id.to_string(),
            path: path.to_string(),
            title: schema
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .map(humanize_identifier)
                .unwrap_or_else(|| humanize(&Value::String(id.to_string()))),
            description: schema
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            required,
            default: schema.get("default").cloned(),
            sensitive: schema
                .get(SENSITIVE_MARKER)
                .and_then(Value::as_bool)
                .or_else(|| schema.get("writeOnly").and_then(Value::as_bool))
                .unwrap_or(false),
            secret: schema
                .get(SECRET_MARKER)
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    fn sensitive(mut self, value: bool) -> Self {
        self.sensitive |= value;
        self
    }

    fn secret(mut self, value: bool) -> Self {
        self.secret |= value;
        self
    }

    fn finish(self, kind: SettingsNodeKind) -> SettingsNode {
        self.finish_with_constraints(kind, SettingsConstraints::default())
    }

    fn finish_with_constraints(
        self,
        kind: SettingsNodeKind,
        constraints: SettingsConstraints,
    ) -> SettingsNode {
        SettingsNode {
            id: self.id,
            path: self.path,
            title: self.title,
            description: self.description,
            required: self.required,
            default: self.default,
            constraints,
            sensitive: self.sensitive,
            secret: self.secret,
            kind,
        }
    }
}

#[allow(dead_code)]
fn _keep_contract_types_linked(_: OperationDiscoverability) {}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::Serialize;
    use serde_json::json;

    #[derive(Debug, Serialize, JsonSchema)]
    /// Runtime settings used to verify typed settings metadata survives the
    /// internal Schemars compilation boundary.
    struct McpRuntimeConfigFixture {
        /// Base URL used by the MCP runtime.
        api_url: String,
        /// Cache lifetime in seconds.
        ttl_secs: u32,
    }

    #[test]
    fn typed_settings_preserve_docs_and_humanize_rust_identifiers() {
        let contract = settings_contract_for_default(McpRuntimeConfigFixture {
            api_url: "https://example.test".to_string(),
            ttl_secs: 30,
        })
        .expect("compile typed settings");
        assert_eq!(contract.root.title, "MCP Runtime Config Fixture");
        assert!(
            contract
                .root
                .description
                .contains("typed settings metadata")
        );
        let SettingsNodeKind::Object { fields } = &contract.root.kind else {
            panic!("typed settings root should be an object");
        };
        assert_eq!(fields[0].title, "API URL");
        assert!(fields[0].description.contains("Base URL"));
        assert_eq!(fields[1].title, "TTL Secs");
        assert!(fields[1].description.contains("Cache lifetime"));
    }

    #[test]
    fn compiles_bounded_object_lists_choices_and_records_without_schema_keywords() {
        let schema = json!({
            "type": "object",
            "properties": {
                "enabled": {"type":"boolean", "default":true},
                "mode": {"type":"string", "enum":["fast","safe"]},
                "paths": {"type":"array", "items":{"type":"string", "format":"path"}},
                "labels": {"type":"object", "additionalProperties":{"type":"string"}}
            },
            "required": ["enabled"],
            "additionalProperties": false
        });
        let contract = settings_contract_from_schema(&schema).expect("compile settings");
        let wire = serde_json::to_string(&contract).expect("serialize settings");
        for forbidden in ["$ref", "allOf", "anyOf", "oneOf", "additionalProperties"] {
            assert!(!wire.contains(forbidden), "found forbidden key {forbidden}");
        }
        contract.validate().expect("valid contract");
    }

    #[test]
    fn rejects_general_untyped_json_and_accepts_explicit_bounded_escape() {
        let untyped =
            json!({"type":"object", "properties":{"value":{}}, "additionalProperties":false});
        let error = settings_contract_from_schema(&untyped).expect_err("untyped json must fail");
        assert!(error.contains("explicit bounded JSON"));
        let escaped = json!({
            "type":"object",
            "properties":{"value":{"x-agena-json-escape":true,"x-agena-json-max-bytes":1024,"x-agena-json-max-depth":4}},
            "additionalProperties":false
        });
        let contract = settings_contract_from_schema(&escaped).expect("bounded json");
        assert!(matches!(
            contract.root.kind,
            SettingsNodeKind::Object { .. }
        ));
        assert!(
            serde_json::to_string(&contract)
                .expect("wire")
                .contains("max_bytes")
        );
    }

    #[test]
    fn compiles_tagged_variants_and_rejects_untagged_unions() {
        let tagged = json!({
            "oneOf":[
                {"type":"object","properties":{"transport":{"const":"stdio"},"command":{"type":"string"}},"required":["transport","command"]},
                {"type":"object","properties":{"transport":{"const":"http"},"url":{"type":"string","format":"uri"}},"required":["transport","url"]}
            ]
        });
        let contract = settings_contract_from_schema(&tagged).expect("tagged variant");
        assert!(matches!(
            contract.root.kind,
            SettingsNodeKind::TaggedVariant { .. }
        ));
        let untagged = json!({"oneOf":[{"type":"string"},{"type":"integer"}]});
        assert!(settings_contract_from_schema(&untagged).is_err());
    }
}
