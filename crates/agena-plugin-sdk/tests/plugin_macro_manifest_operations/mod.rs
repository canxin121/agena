#[test]
fn tool_macro_permission_dispatch_parses_tool_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let input = json!({ "text": "/tmp/render.txt" });
    let requests = runtime
        .block_on(Plugin::permission_paths(&plugin, "render", &input))
        .expect("permission dispatch should succeed");

    assert_eq!(requests, vec![PathRequest::read("/tmp/render.txt")]);
}

#[test]
fn plugin_macro_declares_service_exports_and_imports_without_ambient_lookup() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    assert_eq!(manifest.services.exports.len(), 1);
    assert_eq!(manifest.services.exports[0].id, "test.echo");
    assert_eq!(manifest.services.exports[0].api_version, 1);
    assert_eq!(manifest.services.imports.len(), 1);
    assert_eq!(manifest.services.imports[0].id, "test.telemetry");
    assert!(manifest.services.imports[0].optional);
    manifest
        .services
        .validate()
        .expect("macro-generated service declarations are valid");
}

#[test]
fn tool_macro_permission_dsl_generates_dynamic_permissions() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let input = json!({
        "path": "/tmp/dynamic",
        "host": "example.com",
        "optional_path": "/tmp/dynamic/optional",
        "optional_host": "optional.example.com"
    });
    let paths = runtime
        .block_on(Plugin::permission_paths(
            &plugin,
            "dynamic_permission",
            &input,
        ))
        .expect("path permission DSL should succeed");
    let networks = runtime
        .block_on(Plugin::permission_networks(
            &plugin,
            "dynamic_permission",
            &input,
        ))
        .expect("network permission DSL should succeed");

    assert_eq!(
        paths,
        vec![
            PathRequest::read("/tmp/dynamic/resolved"),
            PathRequest::read("/tmp/dynamic/optional"),
            PathRequest::write("/tmp/dynamic/extra"),
            PathRequest::read("/tmp/dynamic/related-read"),
            PathRequest::write("/tmp/dynamic/related-write")
        ]
    );
    assert_eq!(
        networks,
        vec![
            NetworkRequest::connect("example.com"),
            NetworkRequest::connect("optional.example.com"),
            NetworkRequest::connect("static.example.com"),
            NetworkRequest::connect("api.example.com")
        ]
    );

    let manifest = Plugin::manifest(&plugin);
    assert!(tool_by_name(&manifest, "dynamic_permission").name == "dynamic_permission");
}

#[test]
fn tool_macro_invoke_dispatch_parses_and_serializes_output() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "render".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "text": "hello" }),
            },
        ))
        .expect("tool invoke should succeed");

    assert_eq!(output.payload, Some(json!({ "rendered": "hello" })));
    assert_eq!(output.output_text, r#"{"rendered":"hello"}"#);
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_rename_and_alias() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_rename".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "path": " README.md " }),
            },
        ))
        .expect("inline rename tool invoke should succeed");

    assert_eq!(output.output_text, "README.md");
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_default_expr() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_default".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({}),
            },
        ))
        .expect("inline default tool invoke should succeed");

    assert_eq!(output.output_text, "3");
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_nested_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_nested".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({
                    "body": { "path": " Cargo.toml " },
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline nested tool invoke should succeed");

    assert_eq!(output.output_text, "Cargo.toml:cargo");
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_flatten_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_flatten".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({
                    "path": " Cargo.toml ",
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline flatten tool invoke should succeed");

    assert_eq!(output.output_text, "Cargo.toml:cargo");
}

#[test]
fn tool_macro_manifest_supports_type_level_inline_item_value_relations() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "inline_item_value_relations");
    let relations = schema_relation_labels(&tool.contract.input_schema);

    assert!(relations.contains(&"forbid_substrings `tags[]`: \"..\", \"~\"".to_string()));
    assert!(relations.contains(&"distinct_trimmed `tags[]`".to_string()));
}

#[test]
fn tool_macro_invoke_dispatch_applies_type_level_inline_item_value_relations() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_item_value_relations".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "tags": ["cargo", "git"] }),
            },
        ))
        .expect("inline item value relations tool invoke should succeed");
    assert_eq!(output.output_text, "cargo,git");

    let forbid_error = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_item_value_relations".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "tags": ["../etc/passwd"] }),
            },
        ))
        .expect_err("type-level inline forbid_substrings should target array items");
    assert!(
        forbid_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain `..`"#)
    );

    let distinct_error = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_item_value_relations".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "tags": [" cargo ", "cargo"] }),
            },
        ))
        .expect_err("type-level inline distinct_trimmed should target array items");
    assert!(
        distinct_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain duplicate values"#)
    );
}

#[test]
fn typed_operation_declaration_is_generated_from_method_signature() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let operation = operation_by_id(&manifest, "manifest.greet");

    assert_eq!(operation.title, "Manifest Greet");
    assert_eq!(operation.description, "Greet from a typed command.");
    assert_eq!(operation.group, "command_palette");
    assert_eq!(operation.category.as_deref(), Some("Test"));
    assert_eq!(operation.slash.as_deref(), Some("/manifest-greet"));
    assert_eq!(operation.aliases, vec!["hello-manifest"]);
    assert_eq!(
        operation.usage.as_deref(),
        Some("/manifest-greet {\"name\":\"Ada\"}")
    );
    assert_eq!(
        operation.target,
        PluginOperationTarget::Method {
            handler: "greet_operation".to_string(),
        }
    );
    let SettingsNodeKind::Object { fields } = &operation.input.root.kind else {
        panic!("typed operation input should use an object root")
    };
    let name = fields
        .iter()
        .find(|field| field.id == "name")
        .expect("name field");
    assert!(matches!(name.kind, SettingsNodeKind::Text));
    assert!(name.required);
    assert_eq!(name.constraints.min_length, Some(1));
    operation
        .input
        .validate_value(&json!({"name":"Ada"}))
        .expect("valid operation input");
    assert!(operation.input.validate_value(&json!({"name":""})).is_err());
}

#[test]
fn inline_operation_arguments_generate_the_same_closed_contract() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let operation = operation_by_id(&manifest, "manifest.inline");

    let SettingsNodeKind::Object { fields } = &operation.input.root.kind else {
        panic!("inline operation input should use an object root")
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].id, "name");
    assert_eq!(fields[0].title, "Name");
    assert_eq!(fields[0].description, "Name to greet.");
    assert_eq!(fields[0].constraints.min_length, Some(1));
    assert_eq!(
        operation
            .input
            .parse_shorthand("Ada")
            .expect("shared shorthand parser"),
        json!({"name":"Ada"})
    );
}

#[test]
fn renamed_defaulted_and_nested_operation_arguments_remain_typed() {
    let manifest = Plugin::manifest(&ManifestPlugin);

    let renamed = operation_by_id(&manifest, "manifest.renamed");
    let SettingsNodeKind::Object { fields } = &renamed.input.root.kind else {
        panic!("renamed input should be an object")
    };
    assert_eq!(fields[0].id, "filePath");

    let defaulted = operation_by_id(&manifest, "manifest.default");
    assert_eq!(
        defaulted.input.default_value().expect("operation default"),
        json!({"count":3})
    );

    let nested = operation_by_id(&manifest, "manifest.inline_nested");
    let SettingsNodeKind::Object { fields } = &nested.input.root.kind else {
        panic!("nested input should be an object")
    };
    assert!(fields.iter().any(|field| {
        field.id == "payload" && matches!(field.kind, SettingsNodeKind::Object { .. })
    }));
    assert!(fields.iter().any(|field| field.id == "query_text"));
}

#[test]
fn operation_contract_handles_choices_numbers_patterns_and_objects() {
    let manifest = Plugin::manifest(&ManifestPlugin);

    let choice = operation_by_id(&manifest, "manifest.inline_choice");
    let SettingsNodeKind::Object { fields } = &choice.input.root.kind else {
        panic!("choice input should be an object")
    };
    assert!(matches!(fields[0].kind, SettingsNodeKind::Choice { .. }));

    let pattern = operation_by_id(&manifest, "manifest.inline_pattern");
    let SettingsNodeKind::Object { fields } = &pattern.input.root.kind else {
        panic!("pattern input should be an object")
    };
    assert!(fields[0].constraints.pattern.is_some());

    let number = operation_by_id(&manifest, "manifest.inline_number");
    let SettingsNodeKind::Object { fields } = &number.input.root.kind else {
        panic!("number input should be an object")
    };
    assert!(matches!(fields[0].kind, SettingsNodeKind::Integer));
    assert!(fields[0].constraints.minimum.is_some());

    let object = operation_by_id(&manifest, "manifest.inline_object");
    let SettingsNodeKind::Object { fields } = &object.input.root.kind else {
        panic!("object input should be an object")
    };
    assert!(matches!(fields[0].kind, SettingsNodeKind::Record { .. }));
}

#[test]
fn tool_declared_operation_targets_the_normal_tool_execution_path() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let operation = manifest
        .operations
        .iter()
        .find(|operation| {
            matches!(
                &operation.target,
                PluginOperationTarget::Tool { tool } if tool == "path_choice"
            )
        })
        .expect("path_choice tool operation");

    assert!(operation.discoverability.catalog);
    operation
        .input
        .validate_value(&json!({"mode":"fast"}))
        .expect("tool-backed operation uses the same input contract");
}

use super::ManifestPlugin;
use super::{operation_by_id, schema_relation_labels, tool_by_name};
use agena_plugin_sdk::prelude::*;
