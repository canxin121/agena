#[test]
fn tool_macro_manifest_infers_output_and_streaming() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "render");

    assert!(manifest.hooks.contains(HookSubscription::TOOL_INVOKE));
    assert!(
        manifest
            .hooks
            .contains(HookSubscription::TOOL_INVOKE_STREAM)
    );
    assert_eq!(tool.runtime.streaming, ToolStreamingMode::Streaming);
    assert!(tool.runtime.concurrency_safe);
    assert_ne!(tool.contract.output_schema, Value::Null);
    assert!(
        tool.contract
            .output_schema
            .pointer("/properties/rendered")
            .is_some(),
        "typed output schema should be inferred from Result<ManifestOutput>"
    );
}

#[test]
fn tool_macro_manifest_uses_doc_comments_and_dynamic_output() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let doc_tool = tool_by_name(&manifest, "doc_render");
    let dynamic_tool = tool_by_name(&manifest, "dynamic");
    let explicit_tool = tool_by_name(&manifest, "explicit");

    assert_eq!(
        doc_tool.docs.summary.as_deref(),
        Some("Render docs summary.")
    );
    assert!(
        doc_tool
            .docs
            .help
            .as_deref()
            .is_some_and(|help| help.contains("Render docs help."))
    );
    assert_eq!(dynamic_tool.contract.output_schema, Value::Null);
    assert!(
        explicit_tool
            .contract
            .output_schema
            .pointer("/properties/rendered")
            .is_some(),
        "explicit output(Type) should still generate a typed schema"
    );
}

#[test]
fn tool_input_field_semantics_generate_declarative_permissions() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "semantic");

    assert_eq!(SemanticInput::input_paths().len(), 6);
    assert_eq!(SemanticInput::input_paths()[0].jsonpath, "$.path");
    assert_eq!(SemanticInput::input_paths()[0].kind, PathKind::Write);
    assert_eq!(SemanticInput::input_paths()[0].fallback, None);
    assert!(!SemanticInput::input_paths()[0].optional);
    assert_eq!(SemanticInput::input_paths()[1].jsonpath, "$.config");
    assert_eq!(SemanticInput::input_paths()[1].kind, PathKind::Read);
    assert!(SemanticInput::input_paths()[1].optional);
    assert_eq!(SemanticInput::input_paths()[2].jsonpath, "$.sources[*]");
    assert_eq!(SemanticInput::input_paths()[2].kind, PathKind::Read);
    assert!(!SemanticInput::input_paths()[2].optional);
    assert_eq!(SemanticInput::input_paths()[3].jsonpath, "$.defaulted_path");
    assert_eq!(SemanticInput::input_paths()[3].kind, PathKind::Read);
    assert!(SemanticInput::input_paths()[3].optional);
    assert_eq!(SemanticInput::input_paths()[4].jsonpath, "$.workspace_path");
    assert_eq!(
        SemanticInput::input_paths()[4].fallback.as_deref(),
        Some("")
    );
    assert!(SemanticInput::input_paths()[4].optional);
    assert_eq!(
        SemanticInput::input_paths()[5].jsonpath,
        "$.nested.paths[*]"
    );
    assert!(SemanticInput::input_paths()[5].optional);
    assert_eq!(SemanticInput::input_networks().len(), 1);
    assert_eq!(SemanticInput::input_networks()[0].jsonpath, "$.endpoint");
    assert_eq!(tool.permissions.input_paths, SemanticInput::input_paths());
    assert_eq!(
        tool.permissions.input_networks,
        SemanticInput::input_networks()
    );

    let schema = SemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/path/x-agena-path"),
        Some(&json!("write"))
    );
    assert_eq!(
        schema.pointer("/properties/path/x-agena-picker"),
        Some(&json!("file"))
    );
    assert_eq!(
        schema.pointer("/properties/path/examples"),
        Some(&json!(["out.txt"]))
    );
    assert_eq!(
        schema.pointer("/properties/path/description"),
        Some(&json!("Destination path."))
    );
    assert_eq!(
        schema.pointer("/properties/path/x-agena-order"),
        Some(&json!("000000"))
    );
    assert_eq!(
        schema.pointer("/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );
    assert_eq!(
        schema.pointer("/properties/endpoint/x-agena-order"),
        Some(&json!("000001"))
    );
    assert_eq!(
        schema.pointer("/properties/token/x-agena-secret"),
        Some(&json!(true))
    );
    assert_eq!(
        SemanticInput::input_example(),
        Some(json!({ "path": "out.txt", "endpoint": "https://example.com" }))
    );
    assert_eq!(
        SemanticInput::input_usage().as_deref(),
        Some("path=out.txt endpoint=https://example.com sources=[\"<item>\"] token=<token>")
    );
    assert_eq!(
        ManifestCommandInput::input_example(),
        Some(json!({ "name": "<name>" }))
    );
    assert_eq!(
        ManifestCommandInput::input_usage().as_deref(),
        Some("<name>")
    );

    let semantic_command = command_by_id(&manifest, "semantic");
    assert_eq!(
        semantic_command.usage.as_deref(),
        Some(
            "/semantic path=out.txt endpoint=https://example.com sources=[\"<item>\"] token=<token>"
        )
    );

    let inline = tool_by_name(&manifest, "inline_semantic");
    assert_eq!(inline.permissions.input_paths.len(), 1);
    assert_eq!(inline.permissions.input_paths[0].jsonpath, "$.path");
    assert_eq!(inline.permissions.input_paths[0].kind, PathKind::Read);
    assert_eq!(inline.permissions.input_networks.len(), 1);
    assert_eq!(inline.permissions.input_networks[0].jsonpath, "$.host");
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/host/x-agena-network"),
        Some(&json!("private"))
    );
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/path/description"),
        Some(&json!("Path to inspect."))
    );
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/path/x-agena-order"),
        Some(&json!("000000"))
    );
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/host/x-agena-order"),
        Some(&json!("000001"))
    );

    let inline_command = command_by_id(&manifest, "inline_semantic");
    assert_eq!(
        inline_command.usage.as_deref(),
        Some("/inline-semantic path=README.md host=localhost")
    );

    let inline_auto_tool = tool_by_name(&manifest, "inline_auto");
    assert_eq!(
        inline_auto_tool
            .contract
            .input_schema
            .pointer("/properties/path/x-agena-order"),
        Some(&json!("000000"))
    );
    assert_eq!(
        inline_auto_tool
            .contract
            .input_schema
            .pointer("/properties/count/x-agena-order"),
        Some(&json!("000001"))
    );
    let inline_auto_command = command_by_id(&manifest, "inline_auto");
    assert_eq!(
        inline_auto_command.usage.as_deref(),
        Some("/inline-auto path=<path> count=1")
    );

    let inline_count_tool = tool_by_name(&manifest, "inline_count");
    assert_eq!(
        inline_count_tool
            .contract
            .input_schema
            .pointer("/properties/count/examples"),
        Some(&json!([3]))
    );
    let inline_count_command = command_by_id(&manifest, "inline_count");
    assert_eq!(
        inline_count_command.usage.as_deref(),
        Some("/inline-count 3")
    );

    let inline_rename_tool = tool_by_name(&manifest, "inline_rename");
    assert_eq!(inline_rename_tool.permissions.input_paths.len(), 2);
    assert_eq!(
        inline_rename_tool.permissions.input_paths[0].jsonpath,
        "$.filePath"
    );
    assert!(inline_rename_tool.permissions.input_paths[0].optional);
    assert_eq!(
        inline_rename_tool.permissions.input_paths[1].jsonpath,
        "$.path"
    );
    assert!(inline_rename_tool.permissions.input_paths[1].optional);
    assert_eq!(
        inline_rename_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["path"]))
    );
    let inline_rename_command = command_by_id(&manifest, "inline_rename");
    assert_eq!(
        inline_rename_command.usage.as_deref(),
        Some("/inline-rename <filePath>")
    );

    let inline_default_tool = tool_by_name(&manifest, "inline_default");
    assert_eq!(
        inline_default_tool
            .contract
            .input_schema
            .pointer("/properties/count/default"),
        Some(&json!(3))
    );
    let inline_default_command = command_by_id(&manifest, "inline_default");
    assert_eq!(
        inline_default_command.usage.as_deref(),
        Some("/inline-default 3")
    );

    let inline_nested_tool = tool_by_name(&manifest, "inline_nested");
    let nested_paths = inline_nested_tool
        .permissions
        .input_paths
        .iter()
        .map(|spec| spec.jsonpath.as_str())
        .collect::<Vec<_>>();
    assert_eq!(nested_paths.len(), 6);
    assert!(nested_paths.contains(&"$.payload.file_path"));
    assert!(nested_paths.contains(&"$.payload.filePath"));
    assert!(nested_paths.contains(&"$.payload.path"));
    assert!(nested_paths.contains(&"$.body.file_path"));
    assert!(nested_paths.contains(&"$.body.filePath"));
    assert!(nested_paths.contains(&"$.body.path"));
    assert_eq!(
        inline_nested_tool
            .contract
            .input_schema
            .pointer("/properties/payload/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        inline_nested_tool
            .contract
            .input_schema
            .pointer("/properties/payload/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        inline_nested_tool
            .contract
            .input_schema
            .pointer("/properties/payload/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );

    let inline_flatten_tool = tool_by_name(&manifest, "inline_flatten");
    let flatten_paths = inline_flatten_tool
        .permissions
        .input_paths
        .iter()
        .map(|spec| spec.jsonpath.as_str())
        .collect::<Vec<_>>();
    assert_eq!(flatten_paths.len(), 3);
    assert!(flatten_paths.contains(&"$.file_path"));
    assert!(flatten_paths.contains(&"$.filePath"));
    assert!(flatten_paths.contains(&"$.path"));
    assert_eq!(
        inline_flatten_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        inline_flatten_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        inline_flatten_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );

    let plain_string_tool = tool_by_name(&manifest, "plain_string");
    assert_eq!(
        plain_string_tool.contract.input_schema.pointer("/type"),
        Some(&json!("string"))
    );
    let plain_string_command = command_by_id(&manifest, "plain_string");
    assert_eq!(
        plain_string_command.usage.as_deref(),
        Some("/plain-string <value>")
    );

    let bool_command = command_by_id(&manifest, "manifest.bool");
    assert_eq!(
        bool_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/type")),
        Some(&json!("boolean"))
    );
    assert_eq!(bool_command.usage.as_deref(), Some("/manifest-bool false"));
}
use super::ManifestPlugin;
use super::{ManifestCommandInput, SemanticInput, command_by_id, tool_by_name};
use agena_plugin_sdk::prelude::*;
