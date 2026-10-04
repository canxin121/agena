//! Official nbformat schemas, compiled once without network access or Python.
//! Keep future-minor compatibility aligned with nbformat's validator.
use std::sync::LazyLock;

use serde_json::{Value, json};

use super::{PluginError, SdkResult};

const SCHEMAS: [&str; 6] = [
    include_str!("schemas/nbformat.v4.0.schema.json"),
    include_str!("schemas/nbformat.v4.1.schema.json"),
    include_str!("schemas/nbformat.v4.2.schema.json"),
    include_str!("schemas/nbformat.v4.3.schema.json"),
    include_str!("schemas/nbformat.v4.4.schema.json"),
    include_str!("schemas/nbformat.v4.5.schema.json"),
];

fn relax_additional_properties(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key == "additionalProperties" {
                    *value = Value::Bool(true);
                } else {
                    relax_additional_properties(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(relax_additional_properties),
        _ => {}
    }
}

static VALIDATORS: LazyLock<Vec<jsonschema::Validator>> = LazyLock::new(|| {
    let mut schemas = SCHEMAS
        .iter()
        .map(|text| serde_json::from_str::<Value>(text).expect("vendored nbformat JSON schema"))
        .collect::<Vec<_>>();
    let mut future = schemas.last().expect("latest schema").clone();
    relax_additional_properties(&mut future);
    for (kind, reference) in [
        ("cell", "unrecognized_cell"),
        ("output", "unrecognized_output"),
    ] {
        future["definitions"][kind]["oneOf"]
            .as_array_mut()
            .expect("nbformat union")
            .push(json!({"$ref":format!("#/definitions/{reference}")}));
    }
    schemas.push(future);
    schemas
        .iter()
        .map(|schema| {
            jsonschema::options()
                .with_draft(jsonschema::Draft::Draft4)
                .build(schema)
                .expect("valid offline nbformat schema")
        })
        .collect()
});

pub(super) fn validate(notebook: &Value, minor: u64) -> SdkResult<()> {
    let validator = &VALIDATORS[minor.min(6) as usize];
    validator.validate(notebook).map_err(|error| {
        // Locate the problem without echoing potentially huge/private cell data.
        let path = error
            .instance_path()
            .to_string()
            .chars()
            .take(512)
            .collect::<String>();
        let rule = error
            .schema_path()
            .to_string()
            .chars()
            .take(256)
            .collect::<String>();
        PluginError::invalid_params(format!(
            "notebook schema validation failed at '{path}' (rule '{rule}'); no changes were written"
        ))
    })
}
