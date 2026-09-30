#[test]
fn tool_input_flatten_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = FlattenArgOuter::parse_input(json!({ "path": " Cargo.toml " }))
        .expect("flatten_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        FlattenArgOuter {
            inner: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = FlattenArgOuter::parse_input(json!({}))
        .expect("flatten_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        FlattenArgOuter {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );

    let schema = FlattenArgOuter::input_schema();
    assert_eq!(
        schema.pointer("/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-parse-name"),
        Some(&json!("file_path"))
    );
}

#[test]
fn tool_input_nested_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = NestedArgOuter::parse_input(json!({
        "body": { "path": " Cargo.toml " }
    }))
    .expect("nested_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        NestedArgOuter {
            payload: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = NestedArgOuter::parse_input(json!({
        "payload": {}
    }))
    .expect("nested_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        NestedArgOuter {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );

    let schema = NestedArgOuter::input_schema();
    assert_eq!(
        schema.pointer("/properties/payload/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        schema.pointer("/properties/payload/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );
    assert_eq!(
        schema.pointer("/properties/payload/properties/filePath/x-agena-parse-name"),
        Some(&json!("file_path"))
    );
}

#[test]
fn tool_input_enum_flatten_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = FlattenVariantArgInput::parse_input(json!({
        "action": "query",
        "path": " Cargo.toml "
    }))
    .expect("enum flatten_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        FlattenVariantArgInput::Query {
            inner: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = FlattenVariantArgInput::parse_input(json!({
        "action": "query"
    }))
    .expect("enum flatten_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        FlattenVariantArgInput::Query {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = NestedVariantArgInput::parse_input(json!({
        "action": "query",
        "body": { "path": " Cargo.toml " }
    }))
    .expect("enum nested_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        NestedVariantArgInput::Query {
            payload: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = NestedVariantArgInput::parse_input(json!({
        "action": "query",
        "payload": {}
    }))
    .expect("enum nested_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        NestedVariantArgInput::Query {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_inference_resolves_inner_aliases() {
    let aliased = NestedVariantInferenceInput::parse_input(json!({
        "body": { "path": "marker" },
        "query_text": " cargo "
    }))
    .expect("nested_shape inner aliases should participate in action inference and drop_keys");
    assert_eq!(
        aliased,
        NestedVariantInferenceInput::Query {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );

    let renamed = NestedVariantInferenceInput::parse_input(json!({
        "payload": { "filePath": "marker" },
        "query_text": " cargo "
    }))
    .expect(
        "nested_shape inner schema-side names should participate in action inference and drop_keys",
    );
    assert_eq!(
        renamed,
        NestedVariantInferenceInput::Query {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_array_inference_resolves_item_paths_without_brackets() {
    let aliased = NestedVariantArrayInferenceInput::parse_input(json!({
        "body": [{ "path": "marker" }],
        "query_text": " cargo "
    }))
    .expect(
        "nested_shape array inner aliases should participate in action inference and drop_keys",
    );
    assert_eq!(
        aliased,
        NestedVariantArrayInferenceInput::Query {
            payload: vec![FlattenArgInner {
                file_path: "README.md".to_string(),
            }],
            query_text: "cargo".to_string(),
        }
    );

    let renamed = NestedVariantArrayInferenceInput::parse_input(json!({
        "payload": [{ "filePath": "marker" }],
        "query_text": " cargo "
    }))
    .expect(
        "nested_shape array inner schema-side names should participate in action inference and drop_keys",
    );
    assert_eq!(
        renamed,
        NestedVariantArrayInferenceInput::Query {
            payload: vec![FlattenArgInner {
                file_path: "README.md".to_string(),
            }],
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_nested_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = NestedConstraintOuter::parse_input(json!({
        "body": { "path": " Cargo.toml " }
    }))
    .expect("outer type-level rules should resolve nested_shape inner schema-side names");
    assert_eq!(
        parsed,
        NestedConstraintOuter {
            payload: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let error = NestedConstraintOuter::parse_input(json!({
        "body": { "filePath": "   " }
    }))
    .expect_err("nested_shape outer non_empty should validate the resolved inner path");
    assert!(
        error.diagnostic_message().contains("must not be empty"),
        "unexpected nested_shape outer constraint error: {error}"
    );
}

#[test]
fn tool_input_nested_shape_array_outer_constraints_resolve_item_schema_side_paths() {
    let parsed = NestedConstraintArrayOuter::parse_input(json!({
        "body": [{ "path": " Cargo.toml " }]
    }))
    .expect("outer type-level rules should resolve nested_shape array item schema-side names");
    assert_eq!(
        parsed,
        NestedConstraintArrayOuter {
            payload: vec![FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }]
        }
    );

    let error = NestedConstraintArrayOuter::parse_input(json!({
        "body": [{ "filePath": "   " }]
    }))
    .expect_err("nested_shape array outer non_empty should validate the resolved inner item path");
    assert!(
        error.diagnostic_message().contains("must not be empty"),
        "unexpected nested_shape array outer constraint error: {error}"
    );
}

#[test]
fn tool_input_enum_nested_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = NestedVariantConstraintInput::parse_input(json!({
        "action": "query",
        "body": { "path": " Cargo.toml " }
    }))
    .expect("variant type-level rules should resolve nested_shape inner schema-side names");
    assert_eq!(
        parsed,
        NestedVariantConstraintInput::Query {
            payload: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_array_outer_constraints_resolve_item_schema_side_paths() {
    let parsed = NestedVariantConstraintArrayInput::parse_input(json!({
        "action": "query",
        "body": [{ "path": " Cargo.toml " }]
    }))
    .expect("variant type-level rules should resolve nested_shape array item schema-side names");
    assert_eq!(
        parsed,
        NestedVariantConstraintArrayInput::Query {
            payload: vec![FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }]
        }
    );
}

#[test]
fn tool_input_nested_shape_inner_validation_errors_are_prefixed() {
    let error = NestedArgOuter::parse_input(json!({
        "body": { "filePath": "   " }
    }))
    .expect_err("nested_shape inner validation should surface under the outer field path");
    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `payload.filePath` must not be empty"#),
        "unexpected nested_shape validation error: {error}"
    );
}

#[test]
fn tool_input_enum_nested_shape_inner_validation_errors_are_prefixed() {
    let error = NestedVariantArgInput::parse_input(json!({
        "action": "query",
        "body": { "filePath": "   " }
    }))
    .expect_err("enum nested_shape inner validation should surface under the outer field path");
    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `payload.filePath` must not be empty"#),
        "unexpected enum nested_shape validation error: {error}"
    );
}

#[test]
fn tool_input_nested_shape_array_inner_validation_errors_include_item_index() {
    let error = NestedArgArrayOuter::parse_input(json!({
        "payload": [
            { "filePath": "Cargo.toml" },
            { "path": "   " }
        ]
    }))
    .expect_err("nested_shape array item validation should keep the failing item index");
    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `payload[1].filePath` must not be empty"#),
        "unexpected nested_shape array validation error: {error}"
    );
}

#[test]
fn tool_input_enum_flatten_shape_inference_resolves_inner_aliases() {
    let aliased = FlattenVariantInferenceInput::parse_input(json!({
        "path": "marker",
        "query_text": " cargo "
    }))
    .expect("flattened inner aliases should participate in action inference and drop_keys");
    assert_eq!(
        aliased,
        FlattenVariantInferenceInput::Query {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );

    let renamed = FlattenVariantInferenceInput::parse_input(json!({
        "filePath": "marker",
        "query_text": " cargo "
    }))
    .expect("flattened inner renamed fields should participate in action inference and drop_keys");
    assert_eq!(
        renamed,
        FlattenVariantInferenceInput::Query {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_infer_when_present_supports_nested_paths() {
    let parsed = VariantNestedInferenceInput::parse_input(json!({
        "selector": { "kind": "query" },
        "query_text": " cargo "
    }))
    .expect("infer_when_present should match nested json paths");
    assert_eq!(
        parsed,
        VariantNestedInferenceInput::Query {
            selector: Some(VariantInferenceSelector { kind: None }),
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_infer_when_present_supports_nested_alias_paths() {
    let parsed = VariantNestedFieldArgInferenceInput::parse_input(json!({
        "hint": { "kind": "query" },
        "query_text": " cargo "
    }))
    .expect("nested alias heads should participate in infer_when_present/drop_keys");
    assert_eq!(
        parsed,
        VariantNestedFieldArgInferenceInput::Query {
            selector_value: Some(VariantInferenceSelector { kind: None }),
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_flatten_shape_inference_supports_nested_paths() {
    let parsed = FlattenVariantNestedInferenceInput::parse_input(json!({
        "hint": { "kind": "query" },
        "query_text": " cargo "
    }))
    .expect("flattened inner aliases should participate in nested infer_when_present/drop_keys");
    assert_eq!(
        parsed,
        FlattenVariantNestedInferenceInput::Query {
            inner: FlattenNestedInferenceInner {
                selector: Some(VariantInferenceSelector { kind: None }),
            },
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_flatten_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = FlattenConstraintOuter::parse_input(json!({
        "filePath": " Cargo.toml "
    }))
    .expect("outer type-level rules should resolve flattened inner schema-side names");
    assert_eq!(
        parsed,
        FlattenConstraintOuter {
            inner: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_flatten_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = FlattenVariantConstraintInput::parse_input(json!({
        "action": "query",
        "path": " Cargo.toml "
    }))
    .expect("variant type-level rules should resolve flattened inner schema-side names");
    assert_eq!(
        parsed,
        FlattenVariantConstraintInput::Query {
            inner: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_choice_constraints_apply_to_parse_schema_and_usage() {
    let path_choice = PathChoiceInput::parse_input(json!({ "mode": "fast" }))
        .expect("path-level choices should accept allowed values");
    assert_eq!(path_choice.mode, "fast");
    let field_choice = FieldChoiceInput::parse_input(json!({ "alternateTool": "git" }))
        .expect("field-level choices should accept aliases");
    assert_eq!(field_choice.tool_name, "git");

    let path_error =
        PathChoiceInput::parse_input(json!({ "mode": "turbo" })).expect_err("invalid enum value");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `mode` must be one of ["fast","slow"]"#),
        "unexpected path choice error: {path_error}"
    );
    assert!(
        FieldChoiceInput::parse_input(json!({ "tool": "npm" })).is_err(),
        "field-level choices should reject unsupported values",
    );

    let path_schema = PathChoiceInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/mode/enum"),
        Some(&json!(["fast", "slow"]))
    );
    assert_eq!(PathChoiceInput::input_usage().as_deref(), Some("fast"));

    let field_schema = FieldChoiceInput::input_schema();
    assert_eq!(
        field_schema.pointer("/properties/tool/enum"),
        Some(&json!(["cargo", "git"]))
    );
    assert_eq!(
        field_schema.pointer("/properties/tool/x-agena-aliases"),
        Some(&json!(["tool_name", "alternateTool"]))
    );
    assert_eq!(FieldChoiceInput::input_usage().as_deref(), Some("cargo"));
}
use super::{
    FieldChoiceInput, FlattenArgInner, FlattenArgOuter, FlattenConstraintInner,
    FlattenConstraintOuter, FlattenNestedInferenceInner, FlattenVariantArgInput,
    FlattenVariantConstraintInput, FlattenVariantInferenceInput,
    FlattenVariantNestedInferenceInput, NestedArgArrayOuter, NestedArgOuter,
    NestedConstraintArrayOuter, NestedConstraintOuter, NestedVariantArgInput,
    NestedVariantArrayInferenceInput, NestedVariantConstraintArrayInput,
    NestedVariantConstraintInput, NestedVariantInferenceInput, PathChoiceInput,
    VariantInferenceSelector, VariantNestedFieldArgInferenceInput, VariantNestedInferenceInput,
};
use agena_plugin_sdk::prelude::*;
