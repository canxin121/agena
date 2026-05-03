//! HTTP transport end-to-end. Spins up the SDK's http driver on
//! `127.0.0.1:0`, then drives it via the host's `HttpTransport`.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_plugin_host::transport::{PluginTransport, http::HttpTransport};
use agena_plugin_host::{PluginEntry, PluginHostBuilder, PluginsConfig};
use agena_plugin_sdk::host_api::NoopHostClient;
use agena_plugin_sdk::prelude::*;
use serde_json::json;

#[derive(Default)]
struct EchoHttpPlugin;

#[async_trait]
impl Plugin for EchoHttpPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("echo-http", "0.0.1")
            .hooks(HookSubscription::TOOL_INVOKE | HookSubscription::SHELL_ENV)
            .entry(
                PluginEntryDecl::new(
                    "echo",
                    json!({"type":"object","properties":{"text":{"type":"string"}}}),
                )
                .description("Echo via HTTP."),
            )
            .build()
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let text = input
            .input
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolInvokeOutput::text(format!("http-echo: {text}")))
    }

    async fn shell_env(&self, _: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(Some(ShellEnvPatch::set("AGENA_HTTP_PLUGIN", "1")))
    }
}

async fn spawn_test_server() -> String {
    let host: Arc<dyn HostClient> = Arc::new(NoopHostClient);
    let app = agena_plugin_sdk::drivers::http::router(EchoHttpPlugin, host);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Give axum a moment to start accepting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    format!("http://{local}/rpc")
}

#[tokio::test(flavor = "multi_thread")]
async fn http_transport_round_trip_via_low_level_dispatch() {
    let url = spawn_test_server().await;
    let transport = HttpTransport::new(
        url.parse().unwrap(),
        agena_plugin_host::HttpAuth::None,
        &|_| None,
    );

    // meta/init
    let init = transport
        .dispatch(
            agena_plugin_sdk::rpc::method::META_INIT,
            json!({
                "agena_version": "test",
                "workspace_root": ".",
                "plugin_id": "echo-http",
                "options": {},
                "protocol_version": 1
            }),
        )
        .await
        .expect("init ok");
    assert!(init.get("manifest").is_some());

    // tool.invoke
    let out = transport
        .dispatch(
            agena_plugin_sdk::rpc::method::HOOK_TOOL_INVOKE,
            json!({
                "tool_name": "echo",
                "session_id": 1,
                "call_id": 1,
                "workspace_root": ".",
                "input": { "text": "hi" }
            }),
        )
        .await
        .expect("invoke ok");
    assert_eq!(
        out.get("output_text").and_then(|v| v.as_str()),
        Some("http-echo: hi")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn http_transport_round_trip_via_plugin_host() {
    let url = spawn_test_server().await;

    let mut list = BTreeMap::new();
    list.insert(
        "echo-http".to_string(),
        PluginEntry::Http {
            url: url.parse().unwrap(),
            auth: agena_plugin_host::HttpAuth::None,
            options: serde_json::Value::Null,
            timeouts: Default::default(),
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
        })
        .build()
        .await
        .expect("plugin host should build");

    assert_eq!(host.plugins().len(), 1);
    let resolved = host.lookup_entry("echo").expect("tool exposed");
    let out = host
        .invoke_tool(
            &resolved.handle,
            ToolInvokeInput {
                tool_name: "echo".into(),
                session_id: 7,
                call_id: 11,
                workspace_root: ".".into(),
                input: json!({ "text": "world" }),
            },
        )
        .expect("invoke");
    assert_eq!(out.output_text, "http-echo: world");
}
