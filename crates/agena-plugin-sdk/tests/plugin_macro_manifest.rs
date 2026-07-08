use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct ManifestInput {
    #[arg(trim, non_empty)]
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ManifestOutput {
    rendered: String,
}

#[derive(Default)]
struct ManifestPlugin;

#[agena_plugin(
    namespace = "test",
    name = "manifest",
    version = "0.0.0",
    summary = "Manifest macro behavior test plugin."
)]
impl ManifestPlugin {
    #[tool(
        summary = "Render text.",
        read_only,
        stream = render_stream,
        permission(paths = render_paths),
        concurrency_safe
    )]
    fn render(&self, input: &ManifestInput) -> Result<ManifestOutput> {
        Ok(ManifestOutput {
            rendered: input.text.clone(),
        })
    }

    fn render_stream(&self, sink: ToolStreamSink, input: &ManifestInput) -> Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::text(
            sink.stream_id().to_string(),
            input.text.clone(),
        ))
    }

    fn render_paths(&self, input: &ManifestInput) -> Vec<PathRequest> {
        vec![PathRequest::read(input.text.clone())]
    }

    /// Render docs summary.
    ///
    /// Render docs help.
    #[tool(read_only)]
    fn doc_render(&self) -> String {
        "doc".to_string()
    }

    #[tool(summary = "Dynamic output.", read_only)]
    fn dynamic(&self) -> ToolInvokeOutput {
        ToolInvokeOutput::text("dynamic")
    }

    #[tool(summary = "Explicit output.", output(ManifestOutput), read_only)]
    fn explicit(&self) -> ManifestOutput {
        ManifestOutput {
            rendered: "explicit".to_string(),
        }
    }
}

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

fn tool_by_name<'a>(manifest: &'a PluginManifest, name: &str) -> &'a ToolDefinition {
    manifest
        .tools
        .iter()
        .find(|tool| tool.name == name)
        .unwrap_or_else(|| panic!("{name} tool should be generated"))
}
