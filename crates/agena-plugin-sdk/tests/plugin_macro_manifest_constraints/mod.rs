#[test]
fn tool_input_direct_array_choices_auto_target_items() {
    let path_value = PathAutoItemChoiceInput::parse_input(json!({
        "tools": ["cargo", "git"]
    }))
    .expect("type-level direct array choices should accept matching items");
    assert_eq!(path_value.tools, vec!["cargo", "git"]);

    let renamed_value = RenamedAutoItemChoiceInput::parse_input(json!({
        "legacyTools": ["cargo"]
    }))
    .expect("field-level direct array choices should accept aliases");
    assert_eq!(renamed_value.tool_values, vec!["cargo"]);

    let path_error = PathAutoItemChoiceInput::parse_input(json!({
        "tools": ["npm"]
    }))
    .expect_err("direct choices on array fields should target items");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected direct array choices error: {path_error}"
    );

    let renamed_error = RenamedAutoItemChoiceInput::parse_input(json!({
        "tools": ["npm"]
    }))
    .expect_err("direct choices on aliased array fields should target items");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed direct array choices error: {renamed_error}"
    );

    let path_schema = PathAutoItemChoiceInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );

    let renamed_schema = RenamedAutoItemChoiceInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tools/x-agena-aliases"),
        Some(&json!(["tool_values", "legacyTools"]))
    );
}

#[test]
fn tool_input_enum_variant_local_normalization_applies_to_parse_and_schema() {
    let default_value = VariantNormalizeInput::parse_input(json!({}))
        .expect("enum default_when_empty should still apply with variant-local normalization");
    assert_eq!(default_value, VariantNormalizeInput::List {});

    let query_value = VariantNormalizeInput::parse_input(json!({
        "query": " cargo "
    }))
    .expect("variant-level trim should apply after infer_when_present");
    assert_eq!(
        query_value,
        VariantNormalizeInput::Query {
            query: "cargo".to_string()
        }
    );

    let tags_value = VariantNormalizeInput::parse_input(json!({
        "tags": [" cargo.rs ", " git.rs "]
    }))
    .expect("variant-level item normalization should trim and strip suffixes");
    assert_eq!(
        tags_value,
        VariantNormalizeInput::Tags {
            tags: vec!["cargo".to_string(), "git".to_string()]
        }
    );

    let tags_error = VariantNormalizeInput::parse_input(json!({
        "tags": [" .rs "]
    }))
    .expect_err("variant-level item_non_empty should validate normalized items");
    assert!(
        tags_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty"#),
        "unexpected enum variant item normalization error: {tags_error}"
    );

    let schema = VariantNormalizeInput::input_schema();
    let tags_schema = enum_variant_schema_by_action(&schema, "tags")
        .expect("enum schema should include the tags variant branch");
    assert_eq!(
        tags_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(1))
    );
}

#[test]
fn tool_input_enum_variant_direct_array_constraints_auto_target_items() {
    let auto_tags_value = VariantNormalizeInput::parse_input(json!({
        "auto_tags": [" cargo.rs ", " git.rs "]
    }))
    .expect("variant-level direct array string constraints should target items");
    assert_eq!(
        auto_tags_value,
        VariantNormalizeInput::AutoTags {
            auto_tags: vec!["cargo".to_string(), "git".to_string()]
        }
    );

    let distinct_error = VariantNormalizeInput::parse_input(json!({
        "auto_tags": [" cargo.rs ", "cargo.rs"]
    }))
    .expect_err("variant-level distinct_trimmed should target normalized array items");
    assert!(
        distinct_error
            .diagnostic_message()
            .contains(r#"field `auto_tags[]` must not contain duplicate values"#),
        "unexpected variant direct-array distinct error: {distinct_error}"
    );

    let renamed_tools_value = VariantNormalizeInput::parse_input(json!({
        "action": "renamed_tools",
        "legacyTools": ["cargo"]
    }))
    .expect("variant-level direct array choices should accept aliases after remapping");
    assert_eq!(
        renamed_tools_value,
        VariantNormalizeInput::RenamedTools {
            tool_values: vec!["cargo".to_string()]
        }
    );

    let renamed_tools_error = VariantNormalizeInput::parse_input(json!({
        "action": "renamed_tools",
        "tools": ["npm"]
    }))
    .expect_err("variant-level direct array choices should target renamed array items");
    assert!(
        renamed_tools_error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed variant direct-array choice error: {renamed_tools_error}"
    );

    let schema = VariantNormalizeInput::input_schema();
    let auto_tags_schema = enum_variant_schema_by_action(&schema, "auto_tags")
        .expect("enum schema should include the auto_tags variant branch");
    assert_eq!(
        auto_tags_schema.pointer("/properties/auto_tags/items/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        auto_tags_schema.pointer("/properties/auto_tags/items/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );
    let auto_tags_relations = schema_relation_labels(&auto_tags_schema);
    assert!(auto_tags_relations.contains(&"forbid_substrings `auto_tags[]`: \"..\"".to_string()));
    assert!(auto_tags_relations.contains(&"distinct_trimmed `auto_tags[]`".to_string()));

    let renamed_tools_schema = enum_variant_schema_by_action(&schema, "renamed_tools")
        .expect("enum schema should include the renamed_tools variant branch");
    assert_eq!(
        renamed_tools_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn tool_input_enum_variant_renamed_fields_resolve_constraint_paths() {
    let query_value = VariantRenamedFieldInput::parse_input(json!({
        "action": "query",
        "filePath": " Cargo.toml "
    }))
    .expect("variant-level trim should resolve rust field names through rename_all_fields");
    assert_eq!(
        query_value,
        VariantRenamedFieldInput::Query {
            file_path: "Cargo.toml".to_string()
        }
    );

    let query_error = VariantRenamedFieldInput::parse_input(json!({
        "action": "query",
        "filePath": " "
    }))
    .expect_err("variant-level non_empty should use the renamed field key");
    assert!(
        query_error
            .diagnostic_message()
            .contains(r#"field `filePath` must not be empty"#),
        "unexpected renamed variant non-empty error: {query_error}"
    );

    let run_error = VariantRenamedFieldInput::parse_input(json!({
        "action": "run",
        "filePath": "Cargo.toml"
    }))
    .expect_err("variant-level requires should resolve renamed field paths");
    assert!(
        run_error
            .diagnostic_message()
            .contains(r#"field `filePath` requires `mode`"#),
        "unexpected renamed variant requires error: {run_error}"
    );

    let tags_error = VariantRenamedFieldInput::parse_input(json!({
        "action": "tags",
        "tagValues": [" cargo ", "cargo"]
    }))
    .expect_err("variant-level direct array relation rules should resolve renamed field paths");
    assert!(
        tags_error
            .diagnostic_message()
            .contains(r#"field `tagValues[]` must not contain duplicate values"#),
        "unexpected renamed variant array relation error: {tags_error}"
    );

    let schema = VariantRenamedFieldInput::input_schema();
    let run_schema = enum_variant_schema_by_action(&schema, "run")
        .expect("enum schema should include the run variant branch");
    let run_relations = schema_relation_labels(&run_schema);
    assert!(run_relations.contains(&"requires `filePath` -> `mode`".to_string()));

    let tags_schema = enum_variant_schema_by_action(&schema, "tags")
        .expect("enum schema should include the tags variant branch");
    let tags_relations = schema_relation_labels(&tags_schema);
    assert!(tags_relations.contains(&"distinct_trimmed `tagValues[]`".to_string()));
}

#[test]
fn tool_input_enum_variant_field_args_support_name_alias_default_and_constraints() {
    let query_value = VariantFieldArgInput::parse_input(json!({
        "path": " Cargo.toml "
    }))
    .expect("variant field arg aliases should participate in action inference and trim");
    assert_eq!(
        query_value,
        VariantFieldArgInput::Query {
            file_path: "Cargo.toml".to_string()
        }
    );

    let query_error = VariantFieldArgInput::parse_input(json!({
        "action": "query",
        "filePath": " "
    }))
    .expect_err("variant field arg non_empty should report the schema-side key");
    assert!(
        query_error
            .diagnostic_message()
            .contains(r#"field `filePath` must not be empty"#),
        "unexpected variant field arg error: {query_error}"
    );

    let run_value = VariantFieldArgInput::parse_input(json!({
        "action": "run",
        "path": "Cargo.toml"
    }))
    .expect("variant field arg defaults should populate missing fields after alias normalization");
    assert_eq!(
        run_value,
        VariantFieldArgInput::Run {
            file_path: Some("Cargo.toml".to_string()),
            mode: "read".to_string()
        }
    );

    let tags_error = VariantFieldArgInput::parse_input(json!({
        "action": "tags",
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("variant field arg array rules should use the renamed schema key");
    assert!(
        tags_error
            .diagnostic_message()
            .contains(r#"field `tagValues[]` must not contain duplicate values"#),
        "unexpected variant field arg array error: {tags_error}"
    );

    let schema = VariantFieldArgInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the query variant branch");
    assert!(query_schema.pointer("/properties/filePath").is_some());
    assert_eq!(
        query_schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );

    let run_schema = enum_variant_schema_by_action(&schema, "run")
        .expect("enum schema should include the run variant branch");
    assert_eq!(
        run_schema.pointer("/properties/mode/default"),
        Some(&json!("read"))
    );

    let tags_schema = enum_variant_schema_by_action(&schema, "tags")
        .expect("enum schema should include the tags variant branch");
    let tags_relations = schema_relation_labels(&tags_schema);
    assert!(tags_relations.contains(&"distinct_trimmed `tagValues[]`".to_string()));
}

#[test]
fn tool_input_enum_variant_inference_resolves_renamed_paths() {
    let query_value = VariantInferenceInput::parse_input(json!({
        "filePath": "marker",
        "queryText": " cargo "
    }))
    .expect("variant inference should resolve rename_all_fields paths");
    assert_eq!(
        query_value,
        VariantInferenceInput::Query {
            file_path: None,
            query_text: "cargo".to_string()
        }
    );

    let list_value = VariantInferenceInput::parse_input(json!({}))
        .expect("enum default_when_empty should still apply");
    assert_eq!(list_value, VariantInferenceInput::List {});

    let schema = VariantInferenceInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the query branch");
    assert!(query_schema.pointer("/properties/filePath").is_some());
    assert!(query_schema.pointer("/properties/queryText").is_some());
}

#[test]
fn tool_input_enum_variant_permissions_are_optional_at_root() {
    let paths = VariantSemanticInput::input_paths();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(
        paths[0].optional,
        "variant-derived enum path permissions should be optional on the root shape"
    );

    let networks = VariantSemanticInput::input_networks();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].jsonpath, "$.endpoint");
    assert!(
        networks[0].optional,
        "variant-derived enum network permissions should be optional on the root shape"
    );

    let schema = VariantSemanticInput::input_schema();
    let file_schema = enum_variant_schema_by_action(&schema, "file")
        .expect("enum schema should include the file branch");
    assert_eq!(
        file_schema.pointer("/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    let remote_schema = enum_variant_schema_by_action(&schema, "remote")
        .expect("enum schema should include the remote branch");
    assert_eq!(
        remote_schema.pointer("/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "variant_semantic");
    assert_eq!(tool.permissions.input_paths, paths);
    assert_eq!(tool.permissions.input_networks, networks);
}

#[test]
fn tool_input_field_relation_constraints_apply_to_parse_and_schema() {
    let path_value = PathRelationInput::parse_input(json!({
        "path": "README.md",
        "mode": "read",
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": ["cargo", "git"]
    }))
    .expect("path-level field relation sugar should accept valid input");
    assert_eq!(path_value.path.as_deref(), Some("README.md"));
    assert_eq!(path_value.mode.as_deref(), Some("read"));

    let renamed_value = RenamedRelationInput::parse_input(json!({
        "legacyPath": "README.md",
        "legacyMode": "read",
        "filePath": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect("renamed field relation sugar should accept aliases");
    assert_eq!(renamed_value.file_path_value.as_deref(), Some("README.md"));
    assert_eq!(renamed_value.mode_value.as_deref(), Some("read"));

    let requires_error = PathRelationInput::parse_input(json!({
        "path": "README.md",
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("requires should reject missing peer fields");
    assert!(
        requires_error
            .diagnostic_message()
            .contains(r#"field `path` requires `mode`"#),
        "unexpected requires error: {requires_error}"
    );

    let conflicts_error = PathRelationInput::parse_input(json!({
        "mode": "read",
        "slug": "docs",
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("conflicts_with should reject simultaneous fields");
    assert!(
        conflicts_error
            .diagnostic_message()
            .contains(r#"field `slug` conflicts with `mode`"#),
        "unexpected conflicts_with error: {conflicts_error}"
    );

    let required_unless_error = PathRelationInput::parse_input(json!({
        "file_path": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("required_unless_present should reject missing fallback");
    assert!(
        required_unless_error
            .diagnostic_message()
            .contains(r#"field `fallback` is required unless `mode` is present"#),
        "unexpected required_unless_present error: {required_unless_error}"
    );

    let forbid_error = PathRelationInput::parse_input(json!({
        "fallback": "default",
        "file_path": "../etc/passwd",
        "tags": ["cargo"]
    }))
    .expect_err("forbid_substrings should reject matching substrings");
    assert!(
        forbid_error
            .diagnostic_message()
            .contains(r#"field `file_path` must not contain `..`"#),
        "unexpected forbid_substrings error: {forbid_error}"
    );

    let distinct_error = PathRelationInput::parse_input(json!({
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("distinct_trimmed should reject duplicate trimmed values");
    assert!(
        distinct_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain duplicate values"#),
        "unexpected distinct_trimmed error: {distinct_error}"
    );

    let renamed_requires_error = RenamedRelationInput::parse_input(json!({
        "path": "README.md",
        "filePath": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("renamed requires should report schema-side wire names");
    assert!(
        renamed_requires_error
            .diagnostic_message()
            .contains(r#"field `path` requires `mode`"#),
        "unexpected renamed requires error: {renamed_requires_error}"
    );

    let renamed_forbid_error = RenamedRelationInput::parse_input(json!({
        "filePath": "../etc/passwd",
        "tags": ["cargo"]
    }))
    .expect_err("renamed forbid_substrings should use schema-side wire names");
    assert!(
        renamed_forbid_error
            .diagnostic_message()
            .contains(r#"field `filePath` must not contain `..`"#),
        "unexpected renamed forbid_substrings error: {renamed_forbid_error}"
    );

    let renamed_distinct_error = RenamedRelationInput::parse_input(json!({
        "filePath": "src/lib.rs",
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("renamed distinct_trimmed should use schema-side wire names");
    assert!(
        renamed_distinct_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain duplicate values"#),
        "unexpected renamed distinct_trimmed error: {renamed_distinct_error}"
    );

    let path_schema = PathRelationInput::input_schema();
    let path_relations = schema_relation_labels(&path_schema);
    assert!(path_relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(path_relations.contains(&"conflicts_with `slug` x `mode`".to_string()));
    assert!(
        path_relations
            .contains(&"required_unless_present `fallback` unless `mode` present".to_string())
    );
    assert!(path_relations.contains(&"forbid_substrings `file_path`: \"..\", \"~\"".to_string()));
    assert!(path_relations.contains(&"distinct_trimmed `tags[]`".to_string()));

    let renamed_schema = RenamedRelationInput::input_schema();
    let renamed_relations = schema_relation_labels(&renamed_schema);
    assert!(renamed_relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(renamed_relations.contains(&"forbid_substrings `filePath`: \"..\", \"~\"".to_string()));
    assert!(renamed_relations.contains(&"distinct_trimmed `tags[]`".to_string()));
    assert_eq!(
        renamed_schema.pointer("/properties/path/x-agena-aliases"),
        Some(&json!(["file_path_value", "legacyPath"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["output_path", "legacyFilePath"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/x-agena-aliases"),
        Some(&json!(["tag_values", "legacyTags"]))
    );
}

#[test]
fn tool_input_field_group_constraints_apply_to_parse_and_schema() {
    let path_value = PathGroupInput::parse_input(json!({
        "path": "README.md",
        "text": "hello"
    }))
    .expect("path-level field group sugar should accept valid input");
    assert_eq!(path_value.path.as_deref(), Some("README.md"));

    let renamed_value = RenamedGroupInput::parse_input(json!({
        "legacyPath": "README.md",
        "text": "hello"
    }))
    .expect("renamed field group sugar should accept aliases");
    assert_eq!(renamed_value.file_path_value.as_deref(), Some("README.md"));

    let exactly_one_error = PathGroupInput::parse_input(json!({
        "path": "README.md",
        "stdin": "payload",
        "text": "hello"
    }))
    .expect_err("exactly_one_of should reject both fields present");
    assert!(
        exactly_one_error
            .diagnostic_message()
            .contains(r#"exactly one of `path` or `stdin` is required"#),
        "unexpected exactly_one_of error: {exactly_one_error}"
    );

    let exactly_one_missing_error = PathGroupInput::parse_input(json!({
        "text": "hello"
    }))
    .expect_err("exactly_one_of should reject both fields missing");
    assert!(
        exactly_one_missing_error
            .diagnostic_message()
            .contains(r#"exactly one of `path` or `stdin` is required"#),
        "unexpected exactly_one_of missing error: {exactly_one_missing_error}"
    );

    let at_least_one_error = PathGroupInput::parse_input(json!({
        "path": "README.md"
    }))
    .expect_err("at_least_one_of should reject missing peers");
    assert!(
        at_least_one_error
            .diagnostic_message()
            .contains(r#"at least one of `text` or `stdin` is required"#),
        "unexpected at_least_one_of error: {at_least_one_error}"
    );

    let renamed_exactly_one_error = RenamedGroupInput::parse_input(json!({
        "filePath": "README.md",
        "stdinPayload": "payload",
        "text": "hello"
    }))
    .expect_err("renamed exactly_one_of should use schema-side wire names");
    assert!(
        renamed_exactly_one_error
            .diagnostic_message()
            .contains(r#"exactly one of `filePath` or `stdinPayload` is required"#),
        "unexpected renamed exactly_one_of error: {renamed_exactly_one_error}"
    );

    let renamed_at_least_one_error = RenamedGroupInput::parse_input(json!({
        "filePath": "README.md"
    }))
    .expect_err("renamed at_least_one_of should use schema-side wire names");
    assert!(
        renamed_at_least_one_error
            .diagnostic_message()
            .contains(r#"at least one of `text` or `stdinPayload` is required"#),
        "unexpected renamed at_least_one_of error: {renamed_at_least_one_error}"
    );

    let path_schema = PathGroupInput::input_schema();
    let path_relations = schema_relation_labels(&path_schema);
    assert!(path_relations.contains(&"exactly_one_of: `path`, `stdin`".to_string()));
    assert!(path_relations.contains(&"at_least_one_of: `text`, `stdin`".to_string()));

    let renamed_schema = RenamedGroupInput::input_schema();
    let renamed_relations = schema_relation_labels(&renamed_schema);
    assert!(renamed_relations.contains(&"exactly_one_of: `filePath`, `stdinPayload`".to_string()));
    assert!(renamed_relations.contains(&"at_least_one_of: `text`, `stdinPayload`".to_string()));
    assert_eq!(
        renamed_schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path_value", "legacyPath"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/stdinPayload/x-agena-aliases"),
        Some(&json!(["stdin_payload", "legacyStdin"]))
    );
}

#[test]
fn tool_input_root_example_attr_drives_schema_example_and_usage() {
    assert_eq!(
        RootExampleInput::input_example(),
        Some(json!({
            "query": "rust",
            "filters": ["code"],
            "limit": 3
        }))
    );
    assert_eq!(
        RootExampleInput::input_usage().as_deref(),
        Some("query=rust filters=[\"code\"] limit=3")
    );

    let schema = RootExampleInput::input_schema();
    assert_eq!(
        schema.pointer("/examples"),
        Some(&json!([{
            "query": "rust",
            "filters": ["code"],
            "limit": 3
        }]))
    );
}

#[test]
fn tool_input_partial_root_example_still_fills_required_usage_fields() {
    assert_eq!(
        RootPartialExampleInput::input_example(),
        Some(json!({
            "query": "rust"
        }))
    );
    assert_eq!(
        RootPartialExampleInput::input_usage().as_deref(),
        Some("query=rust limit=1")
    );
}

#[test]
fn tool_input_root_default_attr_applies_to_null_input_and_schema() {
    let parsed = RootDefaultInput::parse_input(Value::Null)
        .expect("root input default should populate null input");
    assert_eq!(parsed.query, "rust");
    assert_eq!(parsed.limit, 3);

    let schema = RootDefaultInput::input_schema();
    assert_eq!(
        schema.pointer("/default"),
        Some(&json!({
            "query": "rust",
            "limit": 3
        }))
    );
    assert_eq!(
        RootDefaultInput::input_usage().as_deref(),
        Some("query=rust limit=3")
    );

    assert!(
        RootDefaultInput::parse_input(json!({ "query": "go" })).is_err(),
        "root input default should not silently merge partial object payloads",
    );
}
use super::ManifestPlugin;
use super::{
    PathAutoItemChoiceInput, PathGroupInput, PathRelationInput, RenamedAutoItemChoiceInput,
    RenamedGroupInput, RenamedRelationInput, RootDefaultInput, RootExampleInput,
    RootPartialExampleInput, VariantFieldArgInput, VariantInferenceInput, VariantNormalizeInput,
    VariantRenamedFieldInput, VariantSemanticInput, enum_variant_schema_by_action,
    schema_relation_labels, tool_by_name,
};
use agena_plugin_sdk::prelude::*;
