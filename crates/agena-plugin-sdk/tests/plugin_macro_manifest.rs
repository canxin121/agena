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

    fn render_paths(&self, _input: &ManifestInput) -> Vec<PathRequest> {
        Vec::new()
    }
}

#[test]
fn tool_macro_manifest_infers_output_and_streaming() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = manifest
        .tools
        .iter()
        .find(|tool| tool.name == "render")
        .expect("render tool should be generated");

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
