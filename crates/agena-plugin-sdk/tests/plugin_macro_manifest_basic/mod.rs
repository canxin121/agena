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
fn plugin_macro_covers_manifest_metadata_declared_by_the_sdk_contract() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    assert_eq!(manifest.authors, ["Agena Team"]);
    assert_eq!(
        manifest.transports,
        [TransportKind::Static, TransportKind::Http]
    );
    assert!(manifest.surface.is_empty());
}

#[test]
fn plugin_and_tool_documentation_translations_are_ui_only_and_locale_aware() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    assert_eq!(
        manifest.summary.as_deref(),
        Some("Manifest macro behavior test plugin.")
    );
    assert_eq!(
        manifest.summary_for_locale("zh-CN"),
        Some("清单宏行为测试插件。")
    );
    assert_eq!(manifest.help_for_locale("fr-FR"), manifest.help_text());
    assert_eq!(
        manifest.summary_for_locale("fr-FR"),
        Some("Plugin de test du manifeste.")
    );

    let tool = tool_by_name(&manifest, "render");
    assert_eq!(tool.docs.summary.as_deref(), Some("Render text."));
    assert_eq!(tool.docs.summary_for_locale("zh_CN"), Some("渲染文本。"));
    assert_eq!(
        tool.docs.help_for_locale("zh-CN"),
        Some("将输入文本作为渲染结果返回。")
    );
    assert_eq!(tool.docs.summary_for_locale("en-US"), Some("Render text."));
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

use super::ManifestPlugin;
use super::tool_by_name;

use agena_plugin_sdk::prelude::*;
