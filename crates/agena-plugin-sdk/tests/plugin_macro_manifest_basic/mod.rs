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
fn service_macro_merges_typed_methods_into_one_versioned_export() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let export = manifest
        .services
        .exports
        .iter()
        .find(|export| export.id == "test.echo" && export.api_version == 1)
        .expect("typed service export");
    assert_eq!(
        export
            .methods
            .iter()
            .map(|method| method.id.as_str())
            .collect::<Vec<_>>(),
        ["echo", "status"]
    );
    manifest
        .services
        .validate()
        .expect("generated service declarations contract");
    export.methods[0]
        .input
        .validate_value(&json!({ "text": "hello" }))
        .expect("typed service request contract");
    export.methods[0]
        .output
        .validate_value(&json!({ "rendered": "service:hello" }))
        .expect("typed service response contract");
    export.methods[1]
        .input
        .validate_value(&json!({}))
        .expect("no-input service method uses a closed empty object");
}

#[test]
fn plugin_macro_compiles_typed_settings_then_applies_presentation_metadata() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let settings = manifest.settings.expect("typed settings contract");
    assert_eq!(settings.root.title, "Manifest Settings");
    assert_eq!(
        settings.root.description,
        "Settings metadata stays presentation-only."
    );
    let SettingsNodeKind::Object { fields } = settings.root.kind else {
        panic!("manifest settings should be an object");
    };
    assert_eq!(fields[0].path, "/enabled");
    assert_eq!(fields[0].title, "Enabled Override");
    assert_eq!(fields[0].description, "Decorated field label.");
    assert_eq!(fields[0].default, Some(json!(false)));
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

    // Operation publication is verified against the closed contract in
    // plugin_macro_manifest_operations; keep these tests focused on
    // ToolInput parsing/schema behavior rather than a second schema view.
    for operation in &manifest.operations {
        operation.validate().expect("generated operation contract");
    }
}
use super::ManifestPlugin;
use super::{ManifestCommandInput, SemanticInput, tool_by_name};
use agena_plugin_sdk::prelude::*;
