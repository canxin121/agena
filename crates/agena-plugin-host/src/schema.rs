//! Compiled JSON Schema contracts shared by registry and interactive callers.

use serde_json::Value as JsonValue;

pub(crate) struct CompiledSchema(jsonschema::Validator);

impl std::fmt::Debug for CompiledSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CompiledSchema")
    }
}

impl CompiledSchema {
    pub(crate) fn new(schema: &JsonValue) -> Result<Self, String> {
        let mut normalized = schema.clone();
        expand_agena_property_aliases(&mut normalized);
        jsonschema::options()
            .should_validate_formats(true)
            .build(&normalized)
            .map(Self)
            .map_err(|error| format!("invalid JSON Schema: {error}"))
    }

    pub(crate) fn validate(&self, value: &JsonValue) -> Result<(), String> {
        let Some(error) = self.0.iter_errors(value).next() else {
            return Ok(());
        };
        let instance_path = error.instance_path().to_string();
        Err(format!("${instance_path}: {error}"))
    }
}

/// Validate an interactive value using the same compiler as execution tools.
pub fn validate_json_schema_value(schema: &JsonValue, value: &JsonValue) -> Result<(), String> {
    CompiledSchema::new(schema)?.validate(value)
}

/// Convert Agena's property-alias annotation into ordinary JSON Schema before
/// compiling it with the standards-compliant validator. Alias properties keep
/// the canonical property's complete schema, and a required canonical field is
/// rewritten as an `anyOf` requirement covering its declared aliases.
fn expand_agena_property_aliases(schema: &mut JsonValue) {
    let JsonValue::Object(object) = schema else {
        if let JsonValue::Array(items) = schema {
            for item in items {
                expand_agena_property_aliases(item);
            }
        }
        return;
    };

    for child in object.values_mut() {
        expand_agena_property_aliases(child);
    }

    let aliases = object
        .get("properties")
        .and_then(JsonValue::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter_map(|(canonical, property_schema)| {
                    let aliases = property_schema
                        .get("x-agena-aliases")
                        .and_then(JsonValue::as_array)?
                        .iter()
                        .filter_map(JsonValue::as_str)
                        .filter(|alias| !alias.is_empty() && *alias != canonical)
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>();
                    (!aliases.is_empty())
                        .then(|| (canonical.clone(), aliases, property_schema.clone()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if aliases.is_empty() {
        return;
    }

    if let Some(properties) = object
        .get_mut("properties")
        .and_then(JsonValue::as_object_mut)
    {
        for (_, aliases, property_schema) in &aliases {
            for alias in aliases {
                properties
                    .entry(alias.clone())
                    .or_insert_with(|| property_schema.clone());
            }
        }
    }

    let required = object
        .get("required")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if required.is_empty() {
        return;
    }

    let mut retained = Vec::with_capacity(required.len());
    let mut alias_requirements = Vec::new();
    for required_property in required {
        let Some(canonical) = required_property.as_str() else {
            retained.push(required_property);
            continue;
        };
        let Some((_, property_aliases, _)) = aliases
            .iter()
            .find(|(property, _, _)| property == canonical)
        else {
            retained.push(required_property);
            continue;
        };
        let variants = std::iter::once(canonical)
            .chain(property_aliases.iter().map(String::as_str))
            .map(|property| serde_json::json!({ "required": [property] }))
            .collect::<Vec<_>>();
        alias_requirements.push(serde_json::json!({ "anyOf": variants }));
    }

    if retained.is_empty() {
        object.remove("required");
    } else {
        object.insert("required".to_owned(), JsonValue::Array(retained));
    }
    if !alias_requirements.is_empty() {
        let all_of = object
            .entry("allOf")
            .or_insert_with(|| JsonValue::Array(Vec::new()));
        if let JsonValue::Array(all_of) = all_of {
            all_of.extend(alias_requirements);
        }
    }
}

/// A fixed semantic marker; peer-provided failure policies are never trusted.
pub const OUTPUT_CONTRACT_VIOLATION_MARKER: &str = "output_contract_violation";

pub fn output_contract_failure() -> agena_failure::Failure {
    use agena_failure::*;
    Failure::new(
        FailureCode::new("tool.output_contract_violation"),
        FailureCategory::ProtocolFailure,
        FailureResponsibility::Dependency,
        RetryDirective::Never,
        RecoveryDirective::None,
        FailureImpact::PartialSuccess,
        UserPresentation::new(
            "tool.output_contract_violation",
            "The tool executed but returned a result that violates its output contract. Its effects may already be committed. Inspect the result before taking further action; do not repeat the operation automatically.",
        ),
    )
}

pub fn output_contract_error(diagnostic: impl std::fmt::Display) -> crate::sdk::PluginError {
    let mut error = crate::sdk::PluginError::internal(diagnostic)
        .with_public_problem(OUTPUT_CONTRACT_VIOLATION_MARKER);
    let mut failure = output_contract_failure();
    let detail =
        agena_failure::diagnostic::user_message_with_context(&error.diagnostic.message, 320);
    if !detail.is_empty() {
        failure.user = agena_failure::UserPresentation::validated(
            "tool.output_contract_violation-detail",
            format!("{} Validation: {detail}", failure.user.fallback),
        );
    }
    error.failure = Box::new(failure);
    error
}
