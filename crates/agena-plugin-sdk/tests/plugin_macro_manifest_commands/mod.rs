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
fn command_macro_manifest_generates_command_definition() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.greet");

    assert_eq!(command.title, "Manifest Greet");
    assert_eq!(command.description, "Greet from a typed command.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-greet"));
    assert_eq!(command.aliases, vec!["hello-manifest"]);
    assert_eq!(command.handler.as_deref(), Some("manifest.greet"));
    assert!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name"))
            .is_some(),
        "typed command input should generate JSON schema"
    );
    match &command.action {
        PluginUiAction::InvokeCommand { command, input } => {
            assert_eq!(command, "manifest.greet");
            assert!(input.is_none());
        }
        other => panic!("expected default InvokeCommand action, got {other:?}"),
    }
}

#[test]
fn command_macro_supports_inline_arg_generated_input() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline");

    assert_eq!(command.title, "Manifest Inline");
    assert_eq!(command.description, "Greet from inline command arguments.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline"));
    assert!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name"))
            .and_then(Value::as_object)
            .is_some(),
        "inline command args should generate an input schema"
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name/description")),
        Some(&json!("Name to greet."))
    );
    assert_eq!(command.usage.as_deref(), Some("/manifest-inline Ada"));
    match &command.action {
        PluginUiAction::InvokeCommand { command, input } => {
            assert_eq!(command, "manifest.inline");
            assert!(input.is_none());
        }
        other => panic!("expected default InvokeCommand action, got {other:?}"),
    }
}

#[test]
fn command_macro_supports_inline_generated_input_without_examples() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto");

    assert_eq!(command.title, "Manifest Inline Auto");
    assert_eq!(
        command.description,
        "Greet from inline command arguments without explicit examples."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline-auto"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline_auto"));
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-auto <name>")
    );
}

#[test]
fn command_macro_supports_inline_arg_rename_and_alias() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.renamed");

    assert_eq!(command.title, "Manifest Renamed");
    assert_eq!(command.description, "Command arg rename and alias support.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-renamed"));
    assert_eq!(command.handler.as_deref(), Some("manifest.renamed"));
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-renamed <filePath>")
    );
    assert!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath"))
            .and_then(Value::as_object)
            .is_some(),
        "renamed inline command args should expose the renamed input field"
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath/x-agena-aliases")),
        Some(&json!(["path"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_default_expr() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.default");

    assert_eq!(command.title, "Manifest Default");
    assert_eq!(command.description, "Inline command default support.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-default"));
    assert_eq!(command.handler.as_deref(), Some("manifest.default"));
    assert_eq!(command.usage.as_deref(), Some("/manifest-default 3"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/default")),
        Some(&json!(3))
    );
}

#[test]
fn command_macro_supports_inline_arg_nested_shape() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_nested");

    assert_eq!(command.title, "Manifest Inline Nested");
    assert_eq!(
        command.description,
        "Inline command nested ToolInput support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline-nested"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline_nested"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/payload/properties/filePath/default")),
        Some(&json!("README.md"))
    );
    assert_eq!(
        command.input_schema.as_ref().and_then(
            |schema| schema.pointer("/properties/payload/properties/filePath/x-agena-aliases")
        ),
        Some(&json!(["file_path", "path"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_flatten_shape() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_flatten");

    assert_eq!(command.title, "Manifest Inline Flatten");
    assert_eq!(
        command.description,
        "Inline command flatten ToolInput support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline-flatten"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline_flatten"));
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-flatten filePath=Cargo.toml query_text=<query_text>")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/required")),
        Some(&json!(["filePath", "query_text"]))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/file_path")),
        None
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath/default")),
        Some(&json!("README.md"))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath/x-agena-aliases")),
        Some(&json!(["file_path", "path"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_choices() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_choice");

    assert_eq!(command.title, "Manifest Inline Choice");
    assert_eq!(command.description, "Inline command choices support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-choice cargo")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tool/enum")),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_format() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_format");

    assert_eq!(command.title, "Manifest Inline Format");
    assert_eq!(command.description, "Inline command format support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-format https://example.com")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/endpoint/format")),
        Some(&json!("uri"))
    );
}

#[test]
fn command_macro_supports_inline_arg_pattern() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_pattern");

    assert_eq!(command.title, "Manifest Inline Pattern");
    assert_eq!(command.description, "Inline command pattern support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-pattern <slug>")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/slug/minLength")),
        Some(&json!(3))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/slug/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
    );
}

#[test]
fn command_macro_supports_inline_arg_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_number");

    assert_eq!(command.title, "Manifest Inline Number");
    assert_eq!(
        command.description,
        "Inline command numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.usage.as_deref(), Some("/manifest-inline-number 2"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/minimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/maximum")),
        Some(&json!(4))
    );
}

#[test]
fn command_macro_supports_inline_arg_exclusive_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_exclusive_number");

    assert_eq!(command.title, "Manifest Inline Exclusive Number");
    assert_eq!(
        command.description,
        "Inline command strict numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-exclusive-number 3")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/exclusiveMinimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/exclusiveMaximum")),
        Some(&json!(5))
    );
}

#[test]
fn command_macro_supports_inline_arg_object_property_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_object");

    assert_eq!(command.title, "Manifest Inline Object");
    assert_eq!(
        command.description,
        "Inline command object property bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/labels/minProperties")),
        Some(&json!(1))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/labels/maxProperties")),
        Some(&json!(2))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_format() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_format");

    assert_eq!(command.title, "Manifest Inline Item Format");
    assert_eq!(command.description, "Inline command item format support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-item-format [\"550e8400-e29b-41d4-a716-446655440000\"]")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/ids/items/format")),
        Some(&json!("uuid"))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_constraints() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_pattern");

    assert_eq!(command.title, "Manifest Inline Item Pattern");
    assert_eq!(
        command.description,
        "Inline command item constraints support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(3))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_choices() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_choice");

    assert_eq!(command.title, "Manifest Inline Item Choice");
    assert_eq!(command.description, "Inline command item choices support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_normalization_and_non_empty() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_normalize");

    assert_eq!(command.title, "Manifest Inline Item Normalize");
    assert_eq!(
        command.description,
        "Inline command item normalization support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_non_empty_if_present() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_non_empty_if_present");

    assert_eq!(command.title, "Manifest Inline Item Optional");
    assert_eq!(
        command.description,
        "Inline command optional item non-empty support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
    );
}

#[test]
fn command_macro_supports_inline_arg_direct_array_string_constraints() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto_item_pattern");

    assert_eq!(command.title, "Manifest Inline Auto Item Pattern");
    assert_eq!(
        command.description,
        "Inline command direct array string constraints support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(3))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
    );
}

#[test]
fn command_macro_supports_inline_arg_direct_array_numeric_constraints() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto_item_number");

    assert_eq!(command.title, "Manifest Inline Auto Item Number");
    assert_eq!(
        command.description,
        "Inline command direct array numeric constraints support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/minimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/maximum")),
        Some(&json!(4))
    );
}

#[test]
fn command_macro_supports_inline_arg_direct_array_choices() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto_item_choice");

    assert_eq!(command.title, "Manifest Inline Auto Item Choice");
    assert_eq!(
        command.description,
        "Inline command direct array choices support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_number");

    assert_eq!(command.title, "Manifest Inline Item Number");
    assert_eq!(
        command.description,
        "Inline command item numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/minimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/maximum")),
        Some(&json!(4))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_exclusive_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_exclusive_number");

    assert_eq!(command.title, "Manifest Inline Item Exclusive Number");
    assert_eq!(
        command.description,
        "Inline command item strict numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-item-exclusive-number [3]")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/exclusiveMinimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/exclusiveMaximum")),
        Some(&json!(5))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_object_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_object");

    assert_eq!(command.title, "Manifest Inline Item Object");
    assert_eq!(
        command.description,
        "Inline command item object property bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/entries/items/minProperties")),
        Some(&json!(1))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/entries/items/maxProperties")),
        Some(&json!(2))
    );
}

#[test]
fn command_macro_supports_inline_arg_relations() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_relation");

    assert_eq!(command.title, "Manifest Inline Relation");
    assert_eq!(
        command.description,
        "Inline command relation and string-list rules support."
    );
    assert_eq!(command.category, "Test");
    let relations = schema_relation_labels(
        command
            .input_schema
            .as_ref()
            .expect("inline relation command should expose schema"),
    );
    assert!(relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(relations.contains(&"conflicts_with `slug` x `mode`".to_string()));
    assert!(
        relations.contains(&"required_unless_present `fallback` unless `mode` present".to_string())
    );
    assert!(relations.contains(&"forbid_substrings `file_path`: \"..\", \"~\"".to_string()));
    assert!(relations.contains(&"distinct_trimmed `tags[]`".to_string()));
}

#[test]
fn command_macro_supports_inline_arg_groups() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_group");

    assert_eq!(command.title, "Manifest Inline Group");
    assert_eq!(command.description, "Inline command group rules support.");
    assert_eq!(command.category, "Test");
    let relations = schema_relation_labels(
        command
            .input_schema
            .as_ref()
            .expect("inline group command should expose schema"),
    );
    assert!(relations.contains(&"exactly_one_of: `filePath`, `stdinPayload`".to_string()));
    assert!(relations.contains(&"at_least_one_of: `text`, `stdinPayload`".to_string()));
}

#[test]
fn command_macro_supports_typed_input_with_command_context() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.context");

    assert_eq!(command.title, "Manifest Context");
    assert_eq!(command.description, "Greet with command context.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-context"));
    assert_eq!(command.handler.as_deref(), Some("manifest.context"));
    assert_eq!(command.usage.as_deref(), Some("/manifest-context <name>"));
    assert!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name"))
            .and_then(Value::as_object)
            .is_some(),
        "typed command + context should still expose the typed input schema"
    );
}

#[test]
fn tool_command_macro_generates_command_definition() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.render");
    let default_command = command_by_id(&manifest, "doc_render");

    assert_eq!(command.title, "Manifest Render");
    assert_eq!(command.description, "Render text.");
    assert_eq!(command.slash.as_deref(), Some("/manifest-render"));
    assert_eq!(command.aliases, vec!["render-manifest"]);
    assert_eq!(command.handler.as_deref(), Some("manifest.render"));
    assert!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/text"))
            .is_some(),
        "tool-backed command should reuse the tool input schema"
    );
    match &command.action {
        PluginUiAction::InvokeTool {
            tool,
            input,
            submit_output_as_prompt,
        } => {
            assert_eq!(tool, "render");
            assert!(input.is_none());
            assert!(*submit_output_as_prompt);
        }
        other => panic!("expected default InvokeTool action, got {other:?}"),
    }

    assert_eq!(default_command.title, "Doc Render");
    assert_eq!(default_command.description, "Render docs summary.");
    assert!(default_command.slash.is_none());
    assert_eq!(default_command.handler.as_deref(), Some("doc_render"));
    match &default_command.action {
        PluginUiAction::InvokeTool {
            tool,
            input,
            submit_output_as_prompt,
        } => {
            assert_eq!(tool, "doc_render");
            assert!(input.is_none());
            assert!(!*submit_output_as_prompt);
        }
        other => panic!("expected default InvokeTool action, got {other:?}"),
    }
}
use super::ManifestPlugin;
use super::{command_by_id, schema_relation_labels, tool_by_name};
use agena_plugin_sdk::prelude::*;
