#[test]
fn tool_input_flatten_shape_propagates_declarative_permissions() {
    assert_eq!(FlattenSemanticInner::input_paths().len(), 1);
    assert_eq!(FlattenSemanticOuter::input_paths().len(), 1);
    assert_eq!(
        FlattenSemanticOuter::input_paths()[0].jsonpath,
        "$.file_path"
    );
    assert_eq!(FlattenSemanticOuter::input_paths()[0].kind, PathKind::Read);
}

#[test]
fn tool_input_enum_flatten_shape_propagates_declarative_permissions() {
    let parsed = FlattenVariantSemanticInput::parse_input(json!({
        "action": "query",
        "file_path": "Cargo.toml",
        "endpoint": "https://example.com"
    }))
    .expect("enum flatten_shape should still parse through the flattened ToolInput");
    assert_eq!(
        parsed,
        FlattenVariantSemanticInput::Query {
            inner: FlattenVariantSemanticInner {
                file_path: "Cargo.toml".to_string(),
                endpoint: "https://example.com".to_string(),
            }
        }
    );

    let paths = FlattenVariantSemanticInput::input_paths();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(
        paths[0].optional,
        "flattened enum-variant path permissions should be optional on the root shape"
    );

    let networks = FlattenVariantSemanticInput::input_networks();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].jsonpath, "$.endpoint");
    assert!(
        networks[0].optional,
        "flattened enum-variant network permissions should be optional on the root shape"
    );

    let schema = FlattenVariantSemanticInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the flattened query branch");
    assert_eq!(
        query_schema.pointer("/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        query_schema.pointer("/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "variant_flatten_semantic");
    assert_eq!(tool.permissions.input_paths, paths);
    assert_eq!(tool.permissions.input_networks, networks);
}

#[test]
fn tool_input_nested_shape_propagates_declarative_permissions() {
    let paths = NestedSemanticOuter::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.payload.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert_eq!(paths[1].jsonpath, "$.body.file_path");
    assert_eq!(paths[1].kind, PathKind::Read);

    let networks = NestedSemanticOuter::input_networks();
    assert_eq!(networks.len(), 2);
    assert_eq!(networks[0].jsonpath, "$.payload.endpoint");
    assert_eq!(networks[1].jsonpath, "$.body.endpoint");

    let schema = NestedSemanticOuter::input_schema();
    assert_eq!(
        schema.pointer("/properties/payload/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        schema.pointer("/properties/payload/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );
}

#[test]
fn tool_input_enum_nested_shape_propagates_declarative_permissions() {
    let parsed = NestedVariantSemanticInput::parse_input(json!({
        "action": "query",
        "body": {
            "file_path": "Cargo.toml",
            "endpoint": "https://example.com"
        }
    }))
    .expect("enum nested_shape should parse through the nested ToolInput");
    assert_eq!(
        parsed,
        NestedVariantSemanticInput::Query {
            payload: FlattenVariantSemanticInner {
                file_path: "Cargo.toml".to_string(),
                endpoint: "https://example.com".to_string(),
            }
        }
    );

    let paths = NestedVariantSemanticInput::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.payload.file_path");
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.body.file_path");
    assert!(paths[1].optional);

    let networks = NestedVariantSemanticInput::input_networks();
    assert_eq!(networks.len(), 2);
    assert_eq!(networks[0].jsonpath, "$.payload.endpoint");
    assert!(networks[0].optional);
    assert_eq!(networks[1].jsonpath, "$.body.endpoint");
    assert!(networks[1].optional);

    let schema = NestedVariantSemanticInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the nested query branch");
    assert_eq!(
        query_schema.pointer("/properties/payload/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        query_schema.pointer("/properties/payload/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );
}

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
fn tool_input_field_aliases_generate_alternative_permission_sources() {
    let paths = AliasSemanticInput::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.path");
    assert_eq!(paths[1].kind, PathKind::Read);
    assert!(paths[1].optional);

    let arg_alias_paths = ArgAliasSemanticInput::input_paths();
    assert_eq!(arg_alias_paths.len(), 2);
    assert_eq!(arg_alias_paths[0].jsonpath, "$.file_path");
    assert_eq!(arg_alias_paths[0].kind, PathKind::Read);
    assert!(arg_alias_paths[0].optional);
    assert_eq!(arg_alias_paths[1].jsonpath, "$.path");
    assert_eq!(arg_alias_paths[1].kind, PathKind::Read);
    assert!(arg_alias_paths[1].optional);

    let parsed = ArgAliasSemanticInput::parse_input(json!({ "path": " Cargo.toml " }))
        .expect("field-level arg alias should normalize into the canonical field");
    assert_eq!(parsed.file_path, "Cargo.toml");

    let schema = ArgAliasSemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/file_path/x-agena-aliases"),
        Some(&json!(["path"]))
    );
}

#[test]
fn tool_input_field_name_attr_renames_schema_and_preserves_compat_aliases() {
    let paths = ArgNameSemanticInput::input_paths();
    assert_eq!(paths.len(), 3);
    assert_eq!(paths[0].jsonpath, "$.filePath");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.file_path");
    assert_eq!(paths[1].kind, PathKind::Read);
    assert!(paths[1].optional);
    assert_eq!(paths[2].jsonpath, "$.path");
    assert_eq!(paths[2].kind, PathKind::Read);
    assert!(paths[2].optional);

    let canonical = ArgNameSemanticInput::parse_input(json!({ "filePath": " Cargo.toml " }))
        .expect("field-level arg name should become the canonical wire key");
    assert_eq!(canonical.file_path, "Cargo.toml");

    let legacy = ArgNameSemanticInput::parse_input(json!({ "file_path": " Cargo.toml " }))
        .expect("field-level arg name should keep the old field name as an alias");
    assert_eq!(legacy.file_path, "Cargo.toml");

    let explicit_alias = ArgNameSemanticInput::parse_input(json!({ "path": " Cargo.toml " }))
        .expect("field-level arg name should still honor explicit aliases");
    assert_eq!(explicit_alias.file_path, "Cargo.toml");

    let schema = ArgNameSemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );
    assert_eq!(
        ArgNameSemanticInput::input_usage().as_deref(),
        Some("<filePath>")
    );
}

#[test]
fn tool_input_serde_field_names_drive_permissions_and_metadata() {
    let paths = RenameAllSemanticInput::input_paths();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].jsonpath, "$.filePath");
    assert_eq!(paths[0].kind, PathKind::Read);

    let networks = RenameAllSemanticInput::input_networks();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].jsonpath, "$.apiUrl");

    let schema = RenameAllSemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        schema.pointer("/properties/apiUrl/x-agena-network"),
        Some(&json!("internet"))
    );

    let parsed = RenameAllSemanticInput::parse_input(json!({
        "filePath": "Cargo.toml",
        "apiUrl": "https://example.com"
    }))
    .expect("rename_all input should parse with serialized field names");
    assert_eq!(parsed.file_path, "Cargo.toml");
    assert_eq!(parsed.api_url, "https://example.com");

    let renamed = RenameListSemanticInput::input_paths();
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].jsonpath, "$.inputPath");
}

#[test]
fn tool_input_field_default_attrs_apply_to_parse_schema_and_usage() {
    let parsed = FieldDefaultInput::parse_input(json!({}))
        .expect("field-level arg defaults should populate missing values");
    assert_eq!(parsed.count, 3);
    assert!(!parsed.enabled);
    assert_eq!(parsed.file_path, "README.md");

    let aliased = FieldDefaultInput::parse_input(json!({ "path": "Cargo.toml" }))
        .expect("field-level arg defaults should not override aliases");
    assert_eq!(aliased.count, 3);
    assert!(!aliased.enabled);
    assert_eq!(aliased.file_path, "Cargo.toml");

    let schema = FieldDefaultInput::input_schema();
    assert_eq!(schema.pointer("/properties/count/default"), Some(&json!(3)));
    assert_eq!(
        schema.pointer("/properties/enabled/default"),
        Some(&json!(false))
    );
    assert_eq!(
        schema.pointer("/properties/file_path/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        FieldDefaultInput::input_example(),
        Some(json!({
            "count": 3,
            "file_path": "README.md",
            "enabled": false
        }))
    );
    assert_eq!(
        FieldDefaultInput::input_usage().as_deref(),
        Some("count=3 file_path=README.md enabled=false")
    );

    let paths = FieldDefaultInput::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.path");
    assert!(paths[1].optional);
}

#[test]
fn tool_input_choice_constraints_apply_to_parse_schema_and_usage() {
    let path_choice = PathChoiceInput::parse_input(json!({ "mode": "fast" }))
        .expect("path-level choices should accept allowed values");
    assert_eq!(path_choice.mode, "fast");
    let field_choice = FieldChoiceInput::parse_input(json!({ "legacyTool": "git" }))
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
        Some(&json!(["tool_name", "legacyTool"]))
    );
    assert_eq!(FieldChoiceInput::input_usage().as_deref(), Some("cargo"));

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_choice");
    assert_eq!(path_command.usage.as_deref(), Some("/path-choice fast"));
    let field_command = command_by_id(&manifest, "field_choice");
    assert_eq!(field_command.usage.as_deref(), Some("/field-choice cargo"));
}
use super::ManifestPlugin;
use super::{
    AliasSemanticInput, ArgAliasSemanticInput, ArgNameSemanticInput, FieldChoiceInput,
    FieldDefaultInput, FlattenArgInner, FlattenArgOuter, FlattenConstraintInner,
    FlattenConstraintOuter, FlattenNestedInferenceInner, FlattenSemanticInner,
    FlattenSemanticOuter, FlattenVariantArgInput, FlattenVariantConstraintInput,
    FlattenVariantInferenceInput, FlattenVariantNestedInferenceInput, FlattenVariantSemanticInner,
    FlattenVariantSemanticInput, NestedArgArrayOuter, NestedArgOuter, NestedConstraintArrayOuter,
    NestedConstraintOuter, NestedSemanticOuter, NestedVariantArgInput,
    NestedVariantArrayInferenceInput, NestedVariantConstraintArrayInput,
    NestedVariantConstraintInput, NestedVariantInferenceInput, NestedVariantSemanticInput,
    PathChoiceInput, RenameAllSemanticInput, RenameListSemanticInput, VariantInferenceSelector,
    VariantNestedFieldArgInferenceInput, VariantNestedInferenceInput, command_by_id,
    enum_variant_schema_by_action, tool_by_name,
};
use agena_plugin_sdk::prelude::*;
