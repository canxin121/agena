//! Streaming tool invocation. The plugin's default `tool_invoke_stream`
//! impl emulates a single chunk; this test verifies the host-side
//! `invoke_tool_stream` consumer round-trips that chunk.

use std::collections::BTreeMap;

use agena_plugin_host::{PluginEntry, PluginHostBuilder, PluginsConfig};
use agena_plugin_sdk::prelude::*;
use serde_json::json;

#[derive(Default)]
struct StreamingPlugin;

#[async_trait]
impl Plugin for StreamingPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("streamy", "0.1.0")
            .hooks(HookSubscription::TOOL_INVOKE | HookSubscription::TOOL_INVOKE_STREAM)
            .tool(
                PluginToolDecl::new(
                    "count",
                    json!({"type":"object","properties":{"n":{"type":"integer"}}}),
                )
                .description("Count to N."),
            )
            .build()
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let n = input.input.get("n").and_then(|v| v.as_i64()).unwrap_or(3);
        Ok(ToolInvokeOutput::text(format!("counted to {n}")))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_emulation_yields_one_chunk() {
    let mut list = BTreeMap::new();
    list.insert(
        "streamy".to_string(),
        PluginEntry::Static {
            options: json!(null),
            timeouts: Default::default(),
            disabled: false,
        },
    );
    let host = PluginHostBuilder::new(std::env::current_dir().unwrap(), "test")
        .with_config(PluginsConfig {
            enabled: true,
            timeouts: Default::default(),
            list,
            trusted_keys: Default::default(),
            default_quota: Default::default(),
            quotas: Default::default(),
            tool_presentation: Default::default(),
        })
        .register_static("streamy", StreamingPlugin)
        .build()
        .await
        .expect("build");

    let resolved = host.lookup_entry("count").expect("count exposed");
    let mut stream = host
        .invoke_tool_stream(
            &resolved,
            ToolInvokeInput {
                tool_name: "count".into(),
                session_id: 1,
                call_id: 1,
                workspace_root: ".".into(),
                input: json!({ "n": 5 }),
            },
        )
        .await
        .expect("stream");

    let mut chunks = Vec::new();
    while let Some(c) = stream.chunks.recv().await {
        chunks.push(c);
    }
    assert!(!chunks.is_empty(), "expected at least one chunk");
    assert!(chunks[0].text_delta.as_deref() == Some("counted to 5"));
    let end = stream
        .end
        .await
        .expect("stream end channel")
        .expect("stream result");
    assert_eq!(end.output_text, "counted to 5");
}
