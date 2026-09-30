//! TypeScript mirror generator for Agena's state types.
//!
//! Agena defines every session, execution, background-activity, and
//! notification state exactly once in Rust (`agena-domain`, `agena-api`,
//! `agena-notification`). The web client consumes a generated mirror instead of
//! a hand-written copy:
//!
//! ```bash
//! cargo run -p agena-web-types > packages/agena-web/src/generated/agenaState.ts
//! ```
//!
//! `tests/drift.rs` fails whenever the committed file no longer matches this
//! output, so a backend state change cannot land without updating the mirror.
//! State semantics (busy / attention / terminal) are generated from the Rust
//! predicates, never re-derived in TypeScript.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Map, Value};

/// Header emitted at the top of the generated module.
const HEADER: &str = r#"// AUTO-GENERATED FILE - DO NOT EDIT.
//
// TypeScript mirror of Agena's backend state types. Regenerate with:
//
// cargo run -p agena-web-types > packages/agena-web/src/generated/agenaState.ts
//
// Source of truth: crates/agena-domain, crates/agena-api, crates/agena-notification.
// `cargo test -p agena-web-types` fails whenever this file and the backend
// definitions diverge, so the frontend cannot keep a second, silently drifting
// copy of a state type. Literal arrays carry the order of the Rust JSON schema.

/** JSON payload carried opaquely through the wire contract. */
export type JsonValue = unknown"#;

/// Schema keywords the converter deliberately refuses to guess at.
const UNSUPPORTED_KEYWORDS: [&str; 10] = [
    "allOf",
    "not",
    "if",
    "then",
    "else",
    "patternProperties",
    "propertyNames",
    "prefixItems",
    "contains",
    "dependentSchemas",
];

/// Predicate over one literal of a Rust enum. `None` means the literal is not
/// parseable by the canonical Rust enum, which the generator treats as drift.
type Predicate = for<'a> fn(&'a str) -> Option<bool>;

/// One literal Rust enum mirrored as an `as const` array plus literal union.
struct LiteralEnum {
    /// TypeScript type name; identical to the Rust type name.
    name: &'static str,
    /// One-line documentation.
    doc: &'static str,
    /// JSON schema derived from the Rust enum (authoritative literal list).
    schema: Value,
    /// Derived constants: `(constant name, documentation, predicate)`.
    lists: Vec<(&'static str, &'static str, Predicate)>,
}

/// Rust-side predicates used for the generated classification constants.
mod predicates {
    use agena_domain::{
        BackgroundActivityStatus, ExecutionStatus, SessionStateKind, SubtaskStatus,
    };

    pub(super) fn session_state_busy(value: &str) -> Option<bool> {
        SessionStateKind::parse(value).map(SessionStateKind::is_busy)
    }

    pub(super) fn session_state_attention(value: &str) -> Option<bool> {
        SessionStateKind::parse(value).map(SessionStateKind::is_attention)
    }

    pub(super) fn session_state_failed(value: &str) -> Option<bool> {
        SessionStateKind::parse(value).map(SessionStateKind::is_failed)
    }

    pub(super) fn subtask_terminal(value: &str) -> Option<bool> {
        SubtaskStatus::parse(value).map(SubtaskStatus::is_terminal)
    }

    pub(super) fn execution_status_active(value: &str) -> Option<bool> {
        value
            .parse::<ExecutionStatus>()
            .ok()
            .map(|status| !status.is_terminal())
    }

    pub(super) fn execution_status_terminal(value: &str) -> Option<bool> {
        value
            .parse::<ExecutionStatus>()
            .ok()
            .map(ExecutionStatus::is_terminal)
    }

    pub(super) fn background_activity_active(value: &str) -> Option<bool> {
        value
            .parse::<BackgroundActivityStatus>()
            .ok()
            .map(BackgroundActivityStatus::is_active)
    }

    pub(super) fn background_activity_terminal(value: &str) -> Option<bool> {
        value
            .parse::<BackgroundActivityStatus>()
            .ok()
            .map(BackgroundActivityStatus::is_terminal)
    }
}

/// Literal enums mirrored into TypeScript, in declaration order.
fn literal_enums() -> Vec<LiteralEnum> {
    vec![
        LiteralEnum {
            name: "SessionStateKind",
            doc: "The single derived processing state of a session.",
            schema: schema_of::<agena_api::resource::SessionStateKind>(),
            lists: vec![
                (
                    "SESSION_STATE_BUSY_KINDS",
                    "Kinds whose session is executing model work.",
                    predicates::session_state_busy,
                ),
                (
                    "SESSION_STATE_ATTENTION_KINDS",
                    "Kinds that always need user attention.",
                    predicates::session_state_attention,
                ),
                (
                    "SESSION_STATE_FAILED_KINDS",
                    "Kinds whose session terminally failed.",
                    predicates::session_state_failed,
                ),
            ],
        },
        LiteralEnum {
            name: "SessionLifecycleState",
            doc: "Visibility/readiness status of a persisted session.",
            schema: schema_of::<agena_api::resource::SessionLifecycleState>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "WorkflowState",
            doc: "Persistent state of a session's execution workflow.",
            schema: schema_of::<agena_api::resource::WorkflowState>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "ExecutionPhase",
            doc: "Active phase of a session execution.",
            schema: schema_of::<agena_api::resource::ExecutionPhase>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "SubtaskStatus",
            doc: "Lifecycle status of a delegated subtask.",
            schema: schema_of::<agena_api::resource::SubtaskStatus>(),
            lists: vec![(
                "SUBTASK_TERMINAL_STATUSES",
                "Statuses where a delegated subtask can no longer progress.",
                predicates::subtask_terminal,
            )],
        },
        LiteralEnum {
            name: "SessionRelationKind",
            doc: "Domain meaning of a session's immutable parent edge.",
            schema: schema_of::<agena_api::resource::SessionRelationKind>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "ExecutionStatus",
            doc: "Execution state of a message, operation part, or interactive request.",
            schema: schema_of::<agena_api::resource::RunStatus>(),
            lists: vec![
                (
                    "PART_EXECUTION_ACTIVE_STATUSES",
                    "Statuses where an operation part is still in flight.",
                    predicates::execution_status_active,
                ),
                (
                    "PART_EXECUTION_TERMINAL_STATUSES",
                    "Statuses where an operation part reached a terminal outcome.",
                    predicates::execution_status_terminal,
                ),
            ],
        },
        LiteralEnum {
            name: "BackgroundActivityKind",
            doc: "Kind of background activity.",
            schema: schema_of::<agena_domain::BackgroundActivityKind>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "BackgroundActivityStatus",
            doc: "Lifecycle status shared by every background activity source.",
            schema: schema_of::<agena_domain::BackgroundActivityStatus>(),
            lists: vec![
                (
                    "BACKGROUND_ACTIVITY_ACTIVE_STATUSES",
                    "Statuses where a background activity is still live.",
                    predicates::background_activity_active,
                ),
                (
                    "BACKGROUND_ACTIVITY_TERMINAL_STATUSES",
                    "Statuses where a background activity finished for good.",
                    predicates::background_activity_terminal,
                ),
            ],
        },
        LiteralEnum {
            name: "NotificationSeverity",
            doc: "Severity of a notification.",
            schema: schema_of::<agena_notification::model::NotificationSeverity>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "NotificationSource",
            doc: "Source of a notification.",
            schema: schema_of::<agena_notification::model::NotificationSource>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "NotificationSurface",
            doc: "Surface a notification is rendered on.",
            schema: schema_of::<agena_notification::model::NotificationSurface>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "NotificationControl",
            doc: "Control action of a notification.",
            schema: schema_of::<agena_notification::model::NotificationControl>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "NotificationState",
            doc: "State carried by a status notification.",
            schema: schema_of::<agena_notification::model::NotificationState>(),
            lists: Vec::new(),
        },
        LiteralEnum {
            name: "RunNotificationState",
            doc: "State carried by a run notification.",
            schema: schema_of::<agena_notification::model::RunNotificationState>(),
            lists: Vec::new(),
        },
    ]
}

/// Structured roots rendered from their JSON schema (discriminated unions with
/// their payload shapes). Referenced types are emitted once, by name.
fn structured_roots() -> Vec<(&'static str, Value)> {
    vec![(
        "SessionState",
        schema_of::<agena_api::resource::SessionState>(),
    )]
}

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_else(|error| {
        panic!(
            "failed to serialize the schema for `{}`: {error}",
            std::any::type_name::<T>()
        )
    })
}

/// The generated TypeScript module.
///
/// # Panics
///
/// Panics when the backend schema contains a construct the converter refuses to
/// guess at. That is deliberate: a new state shape must extend the generator
/// instead of silently producing a wrong mirror.
pub fn module_source() -> String {
    module_source_result()
        .unwrap_or_else(|error| panic!("failed to generate the TypeScript state mirror: {error}"))
}

/// Fallible variant of [`module_source`].
pub fn module_source_result() -> Result<String, String> {
    let specs = literal_enums();
    let mut definitions: BTreeMap<String, Value> = BTreeMap::new();
    let mut roots: Vec<(&'static str, Value)> = Vec::new();
    for (name, mut schema) in structured_roots() {
        let nested = schema
            .as_object_mut()
            .and_then(|object| object.remove("$defs"));
        if let Some(Value::Object(nested)) = nested {
            for (definition_name, definition) in nested {
                if let Some(spec) = specs.iter().find(|spec| spec.name == definition_name) {
                    // Literal enums are declared from their own schema. When a
                    // union embeds the same enum, both views must agree.
                    if let Some(embedded) = string_enum_values(&definition)
                        && embedded != literal_values(&spec.schema)?
                    {
                        return Err(format!(
                            "`{definition_name}` disagrees between its own schema and the \
                             schema embedded in a union"
                        ));
                    }
                    continue;
                }
                match definitions.entry(definition_name.clone()) {
                    std::collections::btree_map::Entry::Occupied(slot) => {
                        if slot.get() != &definition {
                            return Err(format!("conflicting schema for `{definition_name}`"));
                        }
                    }
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(definition);
                    }
                }
            }
        }
        roots.push((name, schema));
    }

    // Reference resolution needs every emitted name, including the literal
    // enums, which are declared from `specs` rather than from `definitions`.
    let mut known = definitions.clone();
    for spec in &specs {
        known.insert(spec.name.to_owned(), Value::Null);
    }

    let mut out = String::from(HEADER);
    out.push('\n');
    for spec in &specs {
        push_literal_enum(&mut out, spec)?;
    }
    for (name, schema) in &definitions {
        push_type_alias(&mut out, name, schema, &known)?;
    }
    for (name, schema) in &roots {
        push_type_alias(&mut out, name, schema, &known)?;
    }
    Ok(out)
}

fn string_enum_values(schema: &Value) -> Option<Vec<String>> {
    literal_values(schema).ok()
}

fn push_literal_enum(out: &mut String, spec: &LiteralEnum) -> Result<(), String> {
    let values = literal_values(&spec.schema)?;
    if values.is_empty() {
        return Err(format!("`{}` has no literals", spec.name));
    }
    let constant = const_name(spec.name);
    writeln!(out).expect("writing to a String cannot fail");
    writeln!(out, "/** {} */", spec.doc).expect("writing to a String cannot fail");
    writeln!(
        out,
        "export const {constant} = {} as const",
        ts_tuple(&values)
    )
    .expect("writing to a String cannot fail");
    writeln!(
        out,
        "export type {} = (typeof {constant})[number]",
        spec.name
    )
    .expect("writing to a String cannot fail");

    for (name, doc, predicate) in &spec.lists {
        let mut members = Vec::new();
        for value in &values {
            let matched = predicate(value.as_str()).ok_or_else(|| {
                format!(
                    "`{value}` is not parseable by the Rust enum behind `{}`",
                    spec.name
                )
            })?;
            if matched {
                members.push(value.clone());
            }
        }
        if members.is_empty() {
            return Err(format!("`{name}` resolved to no members"));
        }
        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "/** {doc} */").expect("writing to a String cannot fail");
        writeln!(out, "export const {name} = {} as const", ts_tuple(&members))
            .expect("writing to a String cannot fail");
    }
    Ok(())
}

fn push_type_alias(
    out: &mut String,
    name: &str,
    schema: &Value,
    definitions: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let rendered = render(schema, definitions, 0)?;
    writeln!(out).expect("writing to a String cannot fail");
    if let Some(description) = schema.get("description").and_then(Value::as_str) {
        writeln!(out, "/** {} */", collapse(description)).expect("writing to a String cannot fail");
    }
    writeln!(out, "export type {name} = {rendered}").expect("writing to a String cannot fail");
    Ok(())
}

fn literal_values(schema: &Value) -> Result<Vec<String>, String> {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("non-string enum literal in {schema}"))
            })
            .collect();
    }
    // schemars renders a unit enum with per-variant documentation as a union of
    // `const` variants instead of a single `enum` array, and it groups adjacent
    // undocumented variants back into an inner `enum`.
    for keyword in ["oneOf", "anyOf"] {
        let Some(variants) = schema.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let mut values = Vec::new();
        for variant in variants {
            if let Some(constant) = variant.get("const").and_then(Value::as_str) {
                values.push(constant.to_owned());
                continue;
            }
            values.extend(literal_values(variant)?);
        }
        if !values.is_empty() {
            return Ok(values);
        }
    }
    Err(format!("expected a literal enum schema, got {schema}"))
}

fn ts_tuple(values: &[String]) -> String {
    let rendered = values
        .iter()
        .map(|value| format!("'{value}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered}]")
}

/// `SessionStateKind` -> `SESSION_STATE_KINDS`.
fn const_name(type_name: &str) -> String {
    let mut snake = String::new();
    for (index, character) in type_name.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                snake.push('_');
            }
            snake.push(character.to_ascii_lowercase());
        } else {
            snake.push(character);
        }
    }
    let upper = snake.to_ascii_uppercase();
    if let Some(stem) = upper.strip_suffix('Y') {
        format!("{stem}IES")
    } else if upper.ends_with('S') {
        format!("{upper}ES")
    } else {
        format!("{upper}S")
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render(
    schema: &Value,
    definitions: &BTreeMap<String, Value>,
    indent: usize,
) -> Result<String, String> {
    match schema {
        Value::Bool(true) => Ok("JsonValue".to_owned()),
        Value::Bool(false) => Err("`false` schema is not supported".to_owned()),
        Value::Object(object) => render_node(object, definitions, indent),
        other => Err(format!("unsupported schema node: {other}")),
    }
}

fn render_node(
    object: &Map<String, Value>,
    definitions: &BTreeMap<String, Value>,
    indent: usize,
) -> Result<String, String> {
    for keyword in UNSUPPORTED_KEYWORDS {
        if object.contains_key(keyword) {
            return Err(format!("unsupported schema keyword `{keyword}`"));
        }
    }
    if let Some(reference) = object.get("$ref") {
        let reference = reference
            .as_str()
            .ok_or_else(|| format!("non-string `$ref`: {reference}"))?;
        let name = reference
            .strip_prefix("#/$defs/")
            .ok_or_else(|| format!("unsupported `$ref`: {reference}"))?;
        if !definitions.contains_key(name) {
            return Err(format!("unresolved `$ref`: {reference}"));
        }
        return Ok(name.to_owned());
    }
    if let Some(constant) = object.get("const") {
        return render_literal(constant);
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .ok_or_else(|| "`enum` must be an array".to_owned())?;
        let mut parts = Vec::new();
        for value in values {
            parts.push(render_literal(value)?);
        }
        return Ok(parts.join(" | "));
    }
    if let Some(variants) = object.get("anyOf").or_else(|| object.get("oneOf")) {
        let variants = variants
            .as_array()
            .ok_or_else(|| "`anyOf`/`oneOf` must be an array".to_owned())?;
        if variants.is_empty() {
            return Err("`anyOf`/`oneOf` must not be empty".to_owned());
        }
        let mut parts: Vec<String> = Vec::new();
        for variant in variants {
            let rendered = render(variant, definitions, indent)?;
            if !parts.contains(&rendered) {
                parts.push(rendered);
            }
        }
        return Ok(parts.join(" | "));
    }
    match object.get("type") {
        Some(Value::String(kind)) => render_kind(kind, object, definitions, indent),
        Some(Value::Array(kinds)) => {
            let mut parts: Vec<String> = Vec::new();
            for kind in kinds {
                let kind = kind
                    .as_str()
                    .ok_or_else(|| "`type` entries must be strings".to_owned())?;
                let mut single = object.clone();
                single.remove("type");
                single.insert("type".to_owned(), Value::String(kind.to_owned()));
                let rendered = render_node(&single, definitions, indent)?;
                if !parts.contains(&rendered) {
                    parts.push(rendered);
                }
            }
            Ok(parts.join(" | "))
        }
        Some(_) => Err("`type` must be a string or an array of strings".to_owned()),
        None => {
            if object.is_empty() {
                Ok("JsonValue".to_owned())
            } else {
                Err(format!(
                    "unsupported schema node: {}",
                    Value::Object(object.clone())
                ))
            }
        }
    }
}

fn render_kind(
    kind: &str,
    object: &Map<String, Value>,
    definitions: &BTreeMap<String, Value>,
    indent: usize,
) -> Result<String, String> {
    match kind {
        "string" => Ok("string".to_owned()),
        "integer" | "number" => Ok("number".to_owned()),
        "boolean" => Ok("boolean".to_owned()),
        "null" => Ok("null".to_owned()),
        "array" => {
            let items = match object.get("items") {
                Some(items) => render(items, definitions, indent)?,
                None => "JsonValue".to_owned(),
            };
            Ok(format!("{items}[]"))
        }
        "object" => render_object(object, definitions, indent),
        other => Err(format!("unsupported schema type `{other}`")),
    }
}

fn render_object(
    object: &Map<String, Value>,
    definitions: &BTreeMap<String, Value>,
    indent: usize,
) -> Result<String, String> {
    let Some(properties) = object.get("properties").and_then(Value::as_object) else {
        if let Some(additional) = object.get("additionalProperties")
            && !matches!(additional, Value::Bool(false))
        {
            let value_type = match additional {
                Value::Bool(true) => "JsonValue".to_owned(),
                schema => render(schema, definitions, indent)?,
            };
            return Ok(format!("Record<string, {value_type}>"));
        }
        return Ok("Record<string, JsonValue>".to_owned());
    };
    let required: Vec<&str> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let inner_indent = indent + 1;
    let pad = "  ".repeat(inner_indent);
    let mut lines = Vec::new();
    let mut names: Vec<&String> = properties.keys().collect();
    names.sort_by_key(|name| (name.as_str() != "kind", name.as_str()));
    for name in names {
        let property = &properties[name];
        let (property_schema, nullable) = split_nullable(property);
        let rendered = render(property_schema, definitions, inner_indent)?;
        let is_required = required.contains(&name.as_str());
        let value_type = if is_required && nullable {
            format!("{rendered} | null")
        } else {
            rendered
        };
        let optional = if is_required { "" } else { "?" };
        lines.push(format!("{pad}{}{optional}: {value_type}", ts_key(name)));
    }
    if lines.is_empty() {
        return Ok("Record<string, JsonValue>".to_owned());
    }
    Ok(format!(
        "{{\n{}\n{}}}",
        lines.join("\n"),
        "  ".repeat(indent)
    ))
}

fn split_nullable(schema: &Value) -> (&Value, bool) {
    for keyword in ["anyOf", "oneOf"] {
        let Some(variants) = schema.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let mut non_null = Vec::new();
        let mut nulls = 0;
        for variant in variants {
            if is_null_schema(variant) {
                nulls += 1;
            } else {
                non_null.push(variant);
            }
        }
        if nulls == 1 && non_null.len() == 1 {
            return (non_null[0], true);
        }
    }
    (schema, false)
}

fn is_null_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
}

fn render_literal(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(format!(
            "'{}'",
            text.replace('\\', "\\\\").replace('\'', "\\'")
        )),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        other => Err(format!("unsupported literal: {other}")),
    }
}

fn ts_key(name: &str) -> String {
    let mut characters = name.chars();
    let head = characters.next();
    let valid_head = head.is_some_and(|character| {
        character.is_ascii_alphabetic() || character == '_' || character == '$'
    });
    let valid_tail = characters
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '$');
    if valid_head && valid_tail {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\'', "\\'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_declares_every_state_type() {
        let module = module_source();
        for name in [
            "SessionStateKind",
            "SessionLifecycleState",
            "WorkflowState",
            "ExecutionPhase",
            "SubtaskStatus",
            "SessionRelationKind",
            "ExecutionStatus",
            "BackgroundActivityKind",
            "BackgroundActivityStatus",
            "NotificationSeverity",
            "NotificationSource",
            "NotificationSurface",
            "NotificationControl",
            "NotificationState",
            "RunNotificationState",
            "ActiveExecutionResource",
            "SessionState",
        ] {
            assert!(
                module.contains(&format!("export type {name}")),
                "generated module is missing `{name}`"
            );
        }
    }

    #[test]
    fn session_state_kinds_and_predicates_come_from_rust() {
        let module = module_source();
        assert!(
            module.contains(
                "export const SESSION_STATE_KINDS = ['creating', 'ready', 'running', 'awaiting_interaction', 'failed'] as const"
            ),
            "unexpected session-state literal list"
        );
        assert!(module.contains("export const SESSION_STATE_BUSY_KINDS = ['running'] as const"));
        assert!(module.contains(
            "export const SESSION_STATE_ATTENTION_KINDS = ['awaiting_interaction', 'failed'] as const"
        ));
    }

    #[test]
    fn session_state_union_carries_every_kind_literal() {
        let module = module_source();
        for kind in agena_domain::SessionStateKind::ALL {
            assert!(
                module.contains(&format!("kind: '{}'", kind.as_str())),
                "session-state union is missing `{}`",
                kind.as_str()
            );
        }
    }

    #[test]
    fn renderer_marks_optional_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "required_field": { "type": "string" },
                "optional_field": { "anyOf": [{ "type": "string" }, { "type": "null" }] }
            },
            "required": ["required_field"]
        });
        let rendered = render(&schema, &BTreeMap::new(), 0).expect("render object");
        assert!(rendered.contains("required_field: string"), "{rendered}");
        assert!(rendered.contains("optional_field?: string"), "{rendered}");
    }

    #[test]
    fn renderer_resolves_references_and_rejects_unknown_ones() {
        let mut definitions = BTreeMap::new();
        definitions.insert(
            "WorkflowState".to_owned(),
            serde_json::json!({ "type": "string", "enum": ["quiescent"] }),
        );
        assert_eq!(
            render(
                &serde_json::json!({ "$ref": "#/$defs/WorkflowState" }),
                &definitions,
                0
            )
            .expect("render reference"),
            "WorkflowState"
        );
        assert!(
            render(
                &serde_json::json!({ "$ref": "#/$defs/Missing" }),
                &definitions,
                0
            )
            .is_err()
        );
    }

    #[test]
    fn renderer_rejects_constructs_it_cannot_model() {
        for schema in [
            serde_json::json!({ "allOf": [] }),
            serde_json::json!({ "type": "object", "patternProperties": {} }),
            serde_json::json!("nonsense"),
        ] {
            assert!(
                render(&schema, &BTreeMap::new(), 0).is_err(),
                "expected a hard failure for {schema}"
            );
        }
    }

    #[test]
    fn constant_names_are_pluralized_like_the_docs_promise() {
        assert_eq!(const_name("SessionStateKind"), "SESSION_STATE_KINDS");
        assert_eq!(
            const_name("NotificationSeverity"),
            "NOTIFICATION_SEVERITIES"
        );
        assert_eq!(const_name("ExecutionStatus"), "EXECUTION_STATUSES");
        assert_eq!(const_name("NotificationScope"), "NOTIFICATION_SCOPES");
    }
}
