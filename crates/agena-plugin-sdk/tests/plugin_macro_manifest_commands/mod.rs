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
fn typed_command_declaration_is_generated_from_method_signature() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.greet");

    assert_eq!(command.title, "Manifest Greet");
    assert_eq!(
        command.docs.summary.as_deref(),
        Some("Greet from a typed command.")
    );
    assert_eq!(command.group, "command_palette");
    assert_eq!(command.category.as_deref(), Some("Test"));
    assert_eq!(command.slash.as_deref(), Some("/manifest-greet"));
    assert_eq!(command.aliases, vec!["hello-manifest"]);
    assert_eq!(
        command.docs.usage.as_deref(),
        Some("/manifest-greet {\"name\":\"Ada\"}")
    );
    assert_eq!(
        command.target,
        CommandTarget::Method {
            handler: "greet_command".to_string(),
        }
    );
    let SettingsNodeKind::Object { fields } = &command.input.root.kind else {
        panic!("typed command input should use an object root")
    };
    let name = fields
        .iter()
        .find(|field| field.id == "name")
        .expect("name field");
    assert!(matches!(name.kind, SettingsNodeKind::Text));
    assert!(name.required);
    assert_eq!(name.constraints.min_length, Some(1));
    command
        .input
        .validate_value(&json!({"name":"Ada"}))
        .expect("valid command input");
    assert!(command.input.validate_value(&json!({"name":""})).is_err());
}

#[test]
fn inline_command_arguments_generate_the_same_closed_contract() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline");

    let SettingsNodeKind::Object { fields } = &command.input.root.kind else {
        panic!("inline command input should use an object root")
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].id, "name");
    assert_eq!(fields[0].title, "Name");
    assert_eq!(fields[0].description, "Name to greet.");
    assert_eq!(fields[0].constraints.min_length, Some(1));
    assert_eq!(
        command
            .input
            .parse_shorthand("Ada")
            .expect("shared shorthand parser"),
        json!({"name":"Ada"})
    );
}

#[test]
fn renamed_defaulted_and_nested_command_arguments_remain_typed() {
    let manifest = Plugin::manifest(&ManifestPlugin);

    let renamed = command_by_id(&manifest, "manifest.renamed");
    let SettingsNodeKind::Object { fields } = &renamed.input.root.kind else {
        panic!("renamed input should be an object")
    };
    assert_eq!(fields[0].id, "filePath");

    let defaulted = command_by_id(&manifest, "manifest.default");
    assert_eq!(
        defaulted.input.default_value().expect("command default"),
        json!({"count":3})
    );

    let nested = command_by_id(&manifest, "manifest.inline_nested");
    let SettingsNodeKind::Object { fields } = &nested.input.root.kind else {
        panic!("nested input should be an object")
    };
    assert!(fields.iter().any(|field| {
        field.id == "payload" && matches!(field.kind, SettingsNodeKind::Object { .. })
    }));
    assert!(fields.iter().any(|field| field.id == "query_text"));
}

#[test]
fn command_contract_handles_choices_numbers_patterns_and_objects() {
    let manifest = Plugin::manifest(&ManifestPlugin);

    let choice = command_by_id(&manifest, "manifest.inline_choice");
    let SettingsNodeKind::Object { fields } = &choice.input.root.kind else {
        panic!("choice input should be an object")
    };
    assert!(matches!(fields[0].kind, SettingsNodeKind::Choice { .. }));

    let pattern = command_by_id(&manifest, "manifest.inline_pattern");
    let SettingsNodeKind::Object { fields } = &pattern.input.root.kind else {
        panic!("pattern input should be an object")
    };
    assert!(fields[0].constraints.pattern.is_some());

    let number = command_by_id(&manifest, "manifest.inline_number");
    let SettingsNodeKind::Object { fields } = &number.input.root.kind else {
        panic!("number input should be an object")
    };
    assert!(matches!(fields[0].kind, SettingsNodeKind::Integer));
    assert!(fields[0].constraints.minimum.is_some());

    let object = command_by_id(&manifest, "manifest.inline_object");
    let SettingsNodeKind::Object { fields } = &object.input.root.kind else {
        panic!("object input should be an object")
    };
    assert!(matches!(fields[0].kind, SettingsNodeKind::Record { .. }));
}

#[test]
fn tool_declared_command_targets_the_normal_tool_execution_path() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = manifest
        .commands
        .iter()
        .find(|command| {
            matches!(
                &command.target,
                CommandTarget::Tool { tool } if tool == "path_choice"
            )
        })
        .expect("path_choice tool command");

    assert!(command.discoverability.catalog);
    command
        .input
        .validate_value(&json!({"mode":"fast"}))
        .expect("tool-backed command uses the same input contract");
}

use super::{ManifestPlugin, command_by_id, schema_relation_labels, tool_by_name};
use super::{
    PathChoiceInput, VariantFieldArgInput, VariantInferenceInput, VariantNormalizeInput,
    VariantRenamedFieldInput,
};
use agena_plugin_sdk::prelude::*;

/// A command's documentation is derived from its input contract at build time,
/// never hand-written. These typed commands declare no explicit `usage`, so the
/// macro must produce exactly what the SDK derives from the same schema — a
/// drift here would publish a usage line or an example the input contract does
/// not accept.
#[test]
fn typed_command_docs_match_the_schema_derived_usage_and_examples() {
    fn assert_derived_docs<T: ToolInput>(manifest: &PluginManifest, id: &str) {
        let command = command_by_id(manifest, id);
        let slash = command.slash.as_deref().expect("typed command has a slash");
        let schema = T::input_schema();
        let expected_usage = match T::input_usage() {
            Some(usage) if !usage.trim().is_empty() => format!("{slash} {usage}"),
            _ => slash.to_string(),
        };
        assert_eq!(
            command.docs.usage.as_deref(),
            Some(expected_usage.as_str()),
            "{id} usage must be the slash plus the schema-derived argument text"
        );
        assert_eq!(
            command.docs.examples,
            schema_example_texts(&schema),
            "{id} examples must be the schema-derived example invocations"
        );
    }

    let manifest = Plugin::manifest(&ManifestPlugin);
    assert_derived_docs::<VariantNormalizeInput>(&manifest, "manifest.variant_normalize");
    assert_derived_docs::<VariantRenamedFieldInput>(&manifest, "manifest.variant_renamed_fields");
    assert_derived_docs::<VariantFieldArgInput>(&manifest, "manifest.variant_field_args");
    assert_derived_docs::<VariantInferenceInput>(&manifest, "manifest.variant_inference");
    assert_derived_docs::<PathChoiceInput>(&manifest, "path_choice");
}

#[test]
fn every_slashed_command_documents_itself_under_its_own_slash() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    for command in &manifest.commands {
        let Some(slash) = command.slash.as_deref() else {
            continue;
        };
        let usage = command
            .docs
            .usage
            .as_deref()
            .unwrap_or_else(|| panic!("{} publishes a slash without a usage line", command.id));
        assert!(
            usage == slash || usage.starts_with(&format!("{slash} ")),
            "{} usage {usage:?} must start with its own slash {slash:?}",
            command.id
        );
    }
}
