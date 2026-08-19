#[test]
fn command_macro_dispatch_parses_typed_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.greet".to_string(),
                slash: Some("/manifest-greet".to_string()),
                raw: "/manifest-greet Ada".to_string(),
                input: json!({ "name": " Ada " }),
            },
        ))
        .expect("command invoke should succeed");

    assert_operation_summary(output, "hello Ada");
}

#[test]
fn service_macro_dispatch_decodes_and_encodes_typed_methods() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let echo = runtime
        .block_on(Plugin::service_invoke(
            &plugin,
            PluginServiceInvokeInput {
                service: "test.echo".to_string(),
                api_version: 1,
                method: "echo".to_string(),
                input: json!({ "text": "hello" }),
            },
        ))
        .expect("typed service invoke");
    assert_eq!(echo, json!({ "rendered": "service:hello" }));

    let status = runtime
        .block_on(Plugin::service_invoke(
            &plugin,
            PluginServiceInvokeInput {
                service: "test.echo".to_string(),
                api_version: 1,
                method: "status".to_string(),
                input: json!({}),
            },
        ))
        .expect("async no-input service invoke");
    assert_eq!(status, json!({ "rendered": "ready" }));
}

#[test]
fn command_macro_dispatch_parses_inline_arg_generated_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline".to_string(),
                slash: Some("/manifest-inline".to_string()),
                raw: "/manifest-inline Ada".to_string(),
                input: json!({ "name": " Ada " }),
            },
        ))
        .expect("inline command invoke should succeed");

    assert_operation_summary(output, "hello Ada");
}

#[test]
fn command_macro_dispatch_parses_inline_arg_aliases() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.renamed".to_string(),
                slash: Some("/manifest-renamed".to_string()),
                raw: "/manifest-renamed README.md".to_string(),
                input: json!({ "path": " README.md " }),
            },
        ))
        .expect("renamed inline command invoke should succeed");

    assert_operation_summary(output, "README.md");
}

#[test]
fn command_macro_dispatch_parses_inline_arg_nested_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_nested".to_string(),
                slash: Some("/manifest-inline-nested".to_string()),
                raw: "/manifest-inline-nested query_text=cargo".to_string(),
                input: json!({
                    "payload": {},
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline nested command invoke should succeed");

    assert_operation_summary(output, "README.md:cargo");
}

#[test]
fn command_macro_dispatch_parses_inline_arg_flatten_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_flatten".to_string(),
                slash: Some("/manifest-inline-flatten".to_string()),
                raw: "/manifest-inline-flatten query_text=cargo".to_string(),
                input: json!({
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline flatten command invoke should succeed");

    assert_operation_summary(output, "README.md:cargo");
}

#[test]
fn command_macro_dispatch_applies_inline_arg_default_expr() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.default".to_string(),
                slash: Some("/manifest-default".to_string()),
                raw: "/manifest-default".to_string(),
                input: json!({}),
            },
        ))
        .expect("default inline command invoke should succeed");

    assert_operation_summary(output, "3");
}

#[test]
fn command_macro_dispatch_rejects_values_outside_choices() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_choice".to_string(),
                slash: Some("/manifest-inline-choice".to_string()),
                raw: "/manifest-inline-choice npm".to_string(),
                input: json!({ "tool": "npm" }),
            },
        ))
        .expect_err("inline choice command should reject unsupported values");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `tool` must be one of ["cargo","git"]"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_format() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_format".to_string(),
                slash: Some("/manifest-inline-format".to_string()),
                raw: "/manifest-inline-format not-a-uri".to_string(),
                input: json!({ "endpoint": "not a uri" }),
            },
        ))
        .expect_err("inline format command should reject invalid values");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `endpoint` must match format `uri`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_exclusive_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_exclusive_number".to_string(),
                slash: Some("/manifest-inline-exclusive-number".to_string()),
                raw: "/manifest-inline-exclusive-number 2".to_string(),
                input: json!({ "count": 2 }),
            },
        ))
        .expect_err("inline strict numeric bounds should reject equal lower values");
    assert!(
        min_error
            .diagnostic_message()
            .contains(r#"field `count` must be greater than 2"#)
    );

    let max_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_exclusive_number".to_string(),
                slash: Some("/manifest-inline-exclusive-number".to_string()),
                raw: "/manifest-inline-exclusive-number 5".to_string(),
                input: json!({ "count": 5 }),
            },
        ))
        .expect_err("inline strict numeric bounds should reject equal upper values");
    assert!(
        max_error
            .diagnostic_message()
            .contains(r#"field `count` must be less than 5"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_pattern() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_pattern".to_string(),
                slash: Some("/manifest-inline-pattern".to_string()),
                raw: "/manifest-inline-pattern Cargo".to_string(),
                input: json!({ "slug": "Cargo" }),
            },
        ))
        .expect_err("inline pattern command should reject unsupported values");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `slug` must match pattern `^[a-z0-9-]+$`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_below_min_chars() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_pattern".to_string(),
                slash: Some("/manifest-inline-pattern".to_string()),
                raw: "/manifest-inline-pattern go".to_string(),
                input: json!({ "slug": "go" }),
            },
        ))
        .expect_err("inline pattern command should reject short values");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `slug` must be at least 3 characters"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_object_property_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_object".to_string(),
                slash: Some("/manifest-inline-object".to_string()),
                raw: "/manifest-inline-object {}".to_string(),
                input: json!({ "labels": {} }),
            },
        ))
        .expect_err("inline object command should reject empty objects");
    assert!(
        min_error
            .diagnostic_message()
            .contains(r#"field `labels` requires at least 1 property"#)
    );

    let max_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_object".to_string(),
                slash: Some("/manifest-inline-object".to_string()),
                raw: "/manifest-inline-object a=1 b=2 c=3".to_string(),
                input: json!({
                    "labels": { "a": "1", "b": "2", "c": "3" }
                }),
            },
        ))
        .expect_err("inline object command should reject oversized objects");
    assert!(
        max_error
            .diagnostic_message()
            .contains(r#"field `labels` accepts at most 2 properties"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_format() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_format".to_string(),
                slash: Some("/manifest-inline-item-format".to_string()),
                raw: "/manifest-inline-item-format not-a-uuid".to_string(),
                input: json!({ "ids": ["not-a-uuid"] }),
            },
        ))
        .expect_err("inline item format command should reject invalid values");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `ids[]` must match format `uuid`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_constraints() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_pattern".to_string(),
                slash: Some("/manifest-inline-item-pattern".to_string()),
                raw: "/manifest-inline-item-pattern go".to_string(),
                input: json!({ "tags": ["go"] }),
            },
        ))
        .expect_err("inline item constraints should reject short values");
    assert!(
        min_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must be at least 3 characters"#)
    );

    let pattern_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_pattern".to_string(),
                slash: Some("/manifest-inline-item-pattern".to_string()),
                raw: "/manifest-inline-item-pattern Cargo".to_string(),
                input: json!({ "tags": ["Cargo"] }),
            },
        ))
        .expect_err("inline item constraints should reject invalid patterns");
    assert!(
        pattern_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must match pattern `^[a-z0-9-]+$`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_choices() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_choice".to_string(),
                slash: Some("/manifest-inline-item-choice".to_string()),
                raw: "/manifest-inline-item-choice npm".to_string(),
                input: json!({ "tools": ["npm"] }),
            },
        ))
        .expect_err("inline item choices should reject unsupported values");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#)
    );
}

#[test]
fn command_macro_dispatch_normalizes_inline_item_values() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_normalize".to_string(),
                slash: Some("/manifest-inline-item-normalize".to_string()),
                raw: "/manifest-inline-item-normalize cargo.rs git.rs".to_string(),
                input: json!({ "tags": [" cargo.rs ", " git.rs "] }),
            },
        ))
        .expect("inline item normalization command should succeed");

    assert_operation_summary(output, "cargo,git");
}

#[test]
fn command_macro_dispatch_rejects_empty_normalized_inline_item_values() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_normalize".to_string(),
                slash: Some("/manifest-inline-item-normalize".to_string()),
                raw: "/manifest-inline-item-normalize .rs".to_string(),
                input: json!({ "tags": [" .rs "] }),
            },
        ))
        .expect_err("inline item normalization command should reject empty normalized items");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty"#)
    );
}

#[test]
fn command_macro_dispatch_handles_item_non_empty_if_present() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let missing_output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_non_empty_if_present".to_string(),
                slash: Some("/manifest-inline-item-non-empty-if-present".to_string()),
                raw: "/manifest-inline-item-non-empty-if-present".to_string(),
                input: json!({}),
            },
        ))
        .expect("inline optional item command should allow missing values");

    assert_operation_summary(missing_output, "");

    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_non_empty_if_present".to_string(),
                slash: Some("/manifest-inline-item-non-empty-if-present".to_string()),
                raw: "/manifest-inline-item-non-empty-if-present \"\"".to_string(),
                input: json!({ "tags": [""] }),
            },
        ))
        .expect_err("inline optional item command should reject present empty items");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty when present"#)
    );
}

#[test]
fn command_macro_dispatch_applies_direct_array_string_constraints() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_auto_item_pattern".to_string(),
                slash: Some("/manifest-inline-auto-item-pattern".to_string()),
                raw: "/manifest-inline-auto-item-pattern cargo.rs".to_string(),
                input: json!({ "tags": [" cargo.rs "] }),
            },
        ))
        .expect("inline direct array string constraints should normalize items");

    assert_operation_summary(output, "cargo");

    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_auto_item_pattern".to_string(),
                slash: Some("/manifest-inline-auto-item-pattern".to_string()),
                raw: "/manifest-inline-auto-item-pattern Cargo.rs".to_string(),
                input: json!({ "tags": [" Cargo.rs "] }),
            },
        ))
        .expect_err("inline direct array string constraints should validate items");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `tags[]` must match pattern `^[a-z0-9-]+$`"#)
    );
}

#[test]
fn command_macro_dispatch_applies_direct_array_numeric_constraints() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_auto_item_number".to_string(),
                slash: Some("/manifest-inline-auto-item-number".to_string()),
                raw: "/manifest-inline-auto-item-number 2 4".to_string(),
                input: json!({ "counts": [2, 4] }),
            },
        ))
        .expect("inline direct array numeric constraints should accept matching items");

    assert_operation_summary(output, "2");

    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_auto_item_number".to_string(),
                slash: Some("/manifest-inline-auto-item-number".to_string()),
                raw: "/manifest-inline-auto-item-number 1".to_string(),
                input: json!({ "counts": [1] }),
            },
        ))
        .expect_err("inline direct array numeric constraints should validate items");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at least 2"#)
    );
}

#[test]
fn command_macro_dispatch_applies_direct_array_choices() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_auto_item_choice".to_string(),
                slash: Some("/manifest-inline-auto-item-choice".to_string()),
                raw: "/manifest-inline-auto-item-choice cargo".to_string(),
                input: json!({ "tools": ["cargo"] }),
            },
        ))
        .expect("inline direct array choices should accept matching items");

    assert_operation_summary(output, "cargo");

    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_auto_item_choice".to_string(),
                slash: Some("/manifest-inline-auto-item-choice".to_string()),
                raw: "/manifest-inline-auto-item-choice npm".to_string(),
                input: json!({ "tools": ["npm"] }),
            },
        ))
        .expect_err("inline direct array choices should validate items");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#)
    );
}

#[test]
fn tool_macro_dispatch_handles_enum_variant_local_normalization() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let query_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_normalize".to_string(),
                input: json!({ "query": " cargo " }),
            },
        ))
        .expect("typed tool input should normalize variant-local string fields");
    assert_eq!(query_output.output_text, "query:cargo");

    let tags_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_normalize".to_string(),
                input: json!({ "tags": [" cargo.rs ", " git.rs "] }),
            },
        ))
        .expect("typed tool input should normalize variant-local array items");
    assert_eq!(tags_output.output_text, "tags:cargo,git");

    let auto_tags_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_normalize".to_string(),
                input: json!({ "auto_tags": [" cargo.rs ", " git.rs "] }),
            },
        ))
        .expect("typed tool input should auto-target direct array variant constraints");
    assert_eq!(auto_tags_output.output_text, "auto_tags:cargo,git");
}

#[test]
fn command_macro_dispatch_handles_enum_variant_local_normalization() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let query_output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.variant_normalize".to_string(),
                slash: Some("/manifest-variant-normalize".to_string()),
                raw: "/manifest-variant-normalize cargo".to_string(),
                input: json!({ "query": " cargo " }),
            },
        ))
        .expect("typed command should normalize variant-local string fields");
    assert_operation_summary(query_output, "query:cargo");

    let tags_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.variant_normalize".to_string(),
                slash: Some("/manifest-variant-normalize".to_string()),
                raw: "/manifest-variant-normalize .rs".to_string(),
                input: json!({ "tags": [" .rs "] }),
            },
        ))
        .expect_err("typed command should reject empty normalized array items");
    assert!(
        tags_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty"#)
    );

    let renamed_tools_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.variant_normalize".to_string(),
                slash: Some("/manifest-variant-normalize".to_string()),
                raw: "/manifest-variant-normalize npm".to_string(),
                input: json!({
                    "action": "renamed_tools",
                    "tools": ["npm"]
                }),
            },
        ))
        .expect_err("typed command should validate direct array variant choices");
    assert!(
        renamed_tools_error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed tools command error: {renamed_tools_error}"
    );
}

#[test]
fn tool_and_command_dispatch_handle_enum_variant_renamed_fields() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let tool_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_renamed_fields".to_string(),
                input: json!({
                    "action": "query",
                    "filePath": " Cargo.toml "
                }),
            },
        ))
        .expect("typed tool input should normalize renamed variant fields");
    assert_eq!(tool_output.output_text, "query:Cargo.toml");

    let command_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.variant_renamed_fields".to_string(),
                slash: Some("/manifest-variant-renamed-fields".to_string()),
                raw: "/manifest-variant-renamed-fields filePath=Cargo.toml".to_string(),
                input: json!({
                    "action": "run",
                    "filePath": "Cargo.toml"
                }),
            },
        ))
        .expect_err("typed command should validate renamed variant relations");
    assert!(
        command_error
            .diagnostic_message()
            .contains(r#"field `filePath` requires `mode`"#),
        "unexpected renamed variant command error: {command_error}"
    );
}

#[test]
fn tool_and_command_dispatch_handle_enum_variant_field_args() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let tool_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_field_args".to_string(),
                input: json!({
                    "path": " Cargo.toml "
                }),
            },
        ))
        .expect("typed tool input should normalize aliased variant field args");
    assert_eq!(tool_output.output_text, "query:Cargo.toml");

    let command_output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.variant_field_args".to_string(),
                slash: Some("/manifest-variant-field-args".to_string()),
                raw: "/manifest-variant-field-args path=Cargo.toml".to_string(),
                input: json!({
                    "action": "run",
                    "path": "Cargo.toml"
                }),
            },
        ))
        .expect("typed command should apply alias normalization and defaults for variant fields");
    assert_operation_summary(command_output, "run:Cargo.toml:read");

    let command_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.variant_field_args".to_string(),
                slash: Some("/manifest-variant-field-args".to_string()),
                raw: "/manifest-variant-field-args tags=cargo tags=cargo".to_string(),
                input: json!({
                    "action": "tags",
                    "tags": [" cargo ", "cargo"]
                }),
            },
        ))
        .expect_err("typed command should validate renamed array field args");
    assert!(
        command_error
            .diagnostic_message()
            .contains(r#"field `tagValues[]` must not contain duplicate values"#),
        "unexpected variant field arg command error: {command_error}"
    );
}

#[test]
fn tool_and_command_dispatch_handle_enum_variant_inference() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let tool_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_inference".to_string(),
                input: json!({
                    "filePath": "marker",
                    "queryText": " cargo "
                }),
            },
        ))
        .expect("typed tool input should infer variants through renamed fields");
    assert_eq!(tool_output.output_text, "query::cargo");

    let command_output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.variant_inference".to_string(),
                slash: Some("/manifest-variant-inference".to_string()),
                raw: "/manifest-variant-inference filePath=marker queryText=cargo".to_string(),
                input: json!({
                    "filePath": "marker",
                    "queryText": " cargo "
                }),
            },
        ))
        .expect("typed command should infer variants through renamed fields");
    assert_operation_summary(command_output, "query::cargo");
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_number".to_string(),
                slash: Some("/manifest-inline-item-number".to_string()),
                raw: "/manifest-inline-item-number 1".to_string(),
                input: json!({ "counts": [1] }),
            },
        ))
        .expect_err("inline item numeric bounds should reject low values");
    assert!(
        min_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at least 2"#)
    );

    let max_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_number".to_string(),
                slash: Some("/manifest-inline-item-number".to_string()),
                raw: "/manifest-inline-item-number 5".to_string(),
                input: json!({ "counts": [5] }),
            },
        ))
        .expect_err("inline item numeric bounds should reject high values");
    assert!(
        max_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at most 4"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_exclusive_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_exclusive_number".to_string(),
                slash: Some("/manifest-inline-item-exclusive-number".to_string()),
                raw: "/manifest-inline-item-exclusive-number 2".to_string(),
                input: json!({ "counts": [2] }),
            },
        ))
        .expect_err("inline item strict numeric bounds should reject equal lower values");
    assert!(
        min_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be greater than 2"#)
    );

    let max_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_exclusive_number".to_string(),
                slash: Some("/manifest-inline-item-exclusive-number".to_string()),
                raw: "/manifest-inline-item-exclusive-number 5".to_string(),
                input: json!({ "counts": [5] }),
            },
        ))
        .expect_err("inline item strict numeric bounds should reject equal upper values");
    assert!(
        max_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be less than 5"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_object_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_object".to_string(),
                slash: Some("/manifest-inline-item-object".to_string()),
                raw: "/manifest-inline-item-object [{}]".to_string(),
                input: json!({ "entries": [{}] }),
            },
        ))
        .expect_err("inline item object bounds should reject empty objects");
    assert!(
        min_error
            .diagnostic_message()
            .contains(r#"field `entries[]` requires at least 1 property"#)
    );

    let max_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_item_object".to_string(),
                slash: Some("/manifest-inline-item-object".to_string()),
                raw: "/manifest-inline-item-object [{\"a\":\"1\",\"b\":\"2\",\"c\":\"3\"}]"
                    .to_string(),
                input: json!({
                    "entries": [{ "a": "1", "b": "2", "c": "3" }]
                }),
            },
        ))
        .expect_err("inline item object bounds should reject oversized objects");
    assert!(
        max_error
            .diagnostic_message()
            .contains(r#"field `entries[]` accepts at most 2 properties"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_inline_relation_rules() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let requires_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation path=README.md".to_string(),
                input: json!({
                    "path": "README.md",
                    "fallback": "default",
                    "file_path": "src/lib.rs",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce requires");
    assert!(
        requires_error
            .diagnostic_message()
            .contains(r#"field `path` requires `mode`"#)
    );

    let conflicts_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation slug=docs mode=read".to_string(),
                input: json!({
                    "mode": "read",
                    "slug": "docs",
                    "fallback": "default",
                    "file_path": "src/lib.rs",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce conflicts_with");
    assert!(
        conflicts_error
            .diagnostic_message()
            .contains(r#"field `slug` conflicts with `mode`"#)
    );

    let required_unless_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation".to_string(),
                input: json!({
                    "file_path": "src/lib.rs",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce required_unless_present");
    assert!(
        required_unless_error
            .diagnostic_message()
            .contains(r#"field `fallback` is required unless `mode` is present"#)
    );

    let forbid_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation file_path=../etc".to_string(),
                input: json!({
                    "fallback": "default",
                    "file_path": "../etc/passwd",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce forbid_substrings");
    assert!(
        forbid_error
            .diagnostic_message()
            .contains(r#"field `file_path` must not contain `..`"#)
    );

    let distinct_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation tags=cargo".to_string(),
                input: json!({
                    "fallback": "default",
                    "file_path": "src/lib.rs",
                    "tags": [" cargo ", "cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce distinct_trimmed");
    assert!(
        distinct_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain duplicate values"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_inline_group_rules() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let exactly_one_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_group".to_string(),
                slash: Some("/manifest-inline-group".to_string()),
                raw: "/manifest-inline-group".to_string(),
                input: json!({
                    "filePath": "README.md",
                    "stdinPayload": "payload",
                    "text": "hello"
                }),
            },
        ))
        .expect_err("inline group command should enforce exactly_one_of");
    assert!(
        exactly_one_error
            .diagnostic_message()
            .contains(r#"exactly one of `filePath` or `stdinPayload` is required"#)
    );

    let exactly_one_missing_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_group".to_string(),
                slash: Some("/manifest-inline-group".to_string()),
                raw: "/manifest-inline-group".to_string(),
                input: json!({
                    "text": "hello"
                }),
            },
        ))
        .expect_err("inline group command should reject missing exactly_one_of group");
    assert!(
        exactly_one_missing_error
            .diagnostic_message()
            .contains(r#"exactly one of `filePath` or `stdinPayload` is required"#)
    );

    let at_least_one_error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_group".to_string(),
                slash: Some("/manifest-inline-group".to_string()),
                raw: "/manifest-inline-group filePath=README.md".to_string(),
                input: json!({
                    "filePath": "README.md"
                }),
            },
        ))
        .expect_err("inline group command should enforce at_least_one_of");
    assert!(
        at_least_one_error
            .diagnostic_message()
            .contains(r#"at least one of `text` or `stdinPayload` is required"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.inline_number".to_string(),
                slash: Some("/manifest-inline-number".to_string()),
                raw: "/manifest-inline-number 1".to_string(),
                input: json!({ "count": 1 }),
            },
        ))
        .expect_err("inline numeric command should reject out-of-range values");

    assert!(
        error
            .diagnostic_message()
            .contains(r#"field `count` must be at least 2"#)
    );
}

#[test]
fn command_macro_dispatch_parses_top_level_primitive_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.bool".to_string(),
                slash: Some("/manifest-bool".to_string()),
                raw: "/manifest-bool true".to_string(),
                input: json!(true),
            },
        ))
        .expect("primitive command invoke should succeed");

    assert_operation_summary(output, "true");
}

#[test]
fn command_macro_dispatch_supports_typed_input_with_command_context() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.context".to_string(),
                slash: Some("/manifest-context".to_string()),
                raw: "/manifest-context Ada".to_string(),
                input: json!({ "name": "Ada" }),
            },
        ))
        .expect("context command invoke should succeed");

    assert_operation_summary(output, "Ada via /manifest-context");
}

#[test]
fn tool_macro_dispatch_parses_top_level_primitive_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "plain_string".to_string(),
                input: json!("Ada"),
            },
        ))
        .expect("plain string tool invoke should succeed");
    assert_eq!(output.output_text, "Ada");
}

#[test]
fn tool_command_macro_dispatch_routes_to_tool() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::operation_invoke(
            &plugin,
            PluginOperationInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                operation_id: "manifest.render".to_string(),
                slash: Some("/manifest-render".to_string()),
                raw: "/manifest-render hi".to_string(),
                input: json!({ "text": " hi " }),
            },
        ))
        .expect("tool command invoke should succeed");

    assert_eq!(output.status, PluginOperationStatus::Unavailable);
    assert!(output.summary.contains("executed by the host runtime"));
    assert!(output.effects.is_empty());
}

#[test]
fn hook_macro_allows_multiple_handlers_ordered_by_priority() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let fallback = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("render", json!({ "text": "normal" })),
        ))
        .expect("hook dispatch should succeed")
        .expect("fallback hook should return a patch");
    let high = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("render", json!({ "text": "priority" })),
        ))
        .expect("hook dispatch should succeed")
        .expect("high-priority hook should return a patch");

    assert_eq!(fallback.title_override.as_deref(), Some("fallback"));
    assert_eq!(high.title_override.as_deref(), Some("high"));
}

#[test]
fn hook_macro_filters_by_tool_and_command() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let doc = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("doc_render", json!({})),
        ))
        .expect("tool hook dispatch should succeed")
        .expect("doc hook should match");
    let dynamic = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("dynamic", json!({})),
        ))
        .expect("tool hook dispatch should succeed");
    let tagged = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input_with_tags("dynamic", vec![ToolTag::Filesystem], json!({})),
        ))
        .expect("tagged tool hook dispatch should succeed")
        .expect("tagged hook should match");
    let cargo = runtime
        .block_on(Plugin::command_execute_before(
            &plugin,
            command_before_input("cargo"),
        ))
        .expect("command hook dispatch should succeed");
    let git = runtime
        .block_on(Plugin::command_execute_before(
            &plugin,
            command_before_input("git"),
        ))
        .expect("command hook dispatch should succeed");

    assert_eq!(doc.title_override.as_deref(), Some("doc"));
    assert!(
        dynamic.is_none(),
        "unmatched tool filters should skip handlers"
    );
    assert_eq!(tagged.title_override.as_deref(), Some("write"));
    match cargo {
        Some(CommandBeforeResponse::Patch(patch)) => {
            assert_eq!(patch.args, Some(vec!["check".to_string()]));
        }
        other => panic!("expected cargo command patch, got {other:?}"),
    }
    assert!(
        git.is_none(),
        "unmatched command filters should skip handlers"
    );
}
fn assert_operation_summary(output: PluginOperationResult, expected: &str) {
    assert_eq!(output.status, PluginOperationStatus::Succeeded);
    assert_eq!(output.summary, expected);
    assert!(output.diagnostics.is_empty());
    assert!(output.effects.is_empty());
}

use super::ManifestPlugin;
use super::{command_before_input, tool_before_input, tool_before_input_with_tags};
use agena_plugin_sdk::prelude::*;
