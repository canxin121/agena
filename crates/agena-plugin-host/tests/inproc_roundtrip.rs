//! End-to-end test: build a `PluginHost` containing a static plugin, fire
//! every relevant hook, assert the patches chain.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_plugin_host::{PluginEntry, PluginHostBuilder, PluginsConfig};
use agena_plugin_sdk::host_api::ToolDescriptor;
use agena_plugin_sdk::prelude::*;

struct FakeHostClient;

#[async_trait]
impl HostClient for FakeHostClient {
    async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

    async fn publish_event(&self, _env: EventEnvelope) -> Result<()> {
        Ok(())
    }

    async fn subscribe_events(&self, _filter: EventFilter) -> Result<EventSubscription> {
        Ok(EventSubscription { id: "sub".into() })
    }

    async fn ask_permission(&self, _req: PermissionAskInput) -> Result<PermissionDecision> {
        Ok(PermissionDecision::Prompt)
    }

    async fn read_config(&self, _path: Option<String>) -> Result<serde_json::Value> {
        Ok(json!({ "ok": true }))
    }

    async fn invoke_tool(
        &self,
        _tool: String,
        _input: serde_json::Value,
    ) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("invoked"))
    }

    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>> {
        Ok(vec![ToolDescriptor {
            name: "demo".into(),
            description: None,
            search_terms: Vec::new(),
            behavior: None,
            deferred: false,
            read_only: true,
            plugin_id: Some("demo".into()),
        }])
    }
}

#[derive(Clone)]
struct CapabilityPlugin {
    capabilities: Vec<HostCapability>,
}

impl CapabilityPlugin {
    fn new(capabilities: impl IntoIterator<Item = HostCapability>) -> Self {
        Self {
            capabilities: capabilities.into_iter().collect(),
        }
    }
}

#[async_trait]
impl Plugin for CapabilityPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut entry = PluginEntryDecl::new("ping", json!({ "type": "object" }));
        for capability in &self.capabilities {
            entry = entry.host_capability(*capability);
        }
        PluginManifest::builder("capability-plugin", "0.1.0")
            .hooks(HookSubscription::TOOL_INVOKE)
            .entry(entry)
            .build()
    }

    async fn tool_invoke(&self, _input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("pong"))
    }
}

async fn host_with_capability_plugin(
    capabilities: Vec<HostCapability>,
) -> Arc<agena_plugin_host::PluginHost> {
    let mut list = BTreeMap::new();
    list.insert(
        "capability-plugin".to_string(),
        PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
        },
    );
    PluginHostBuilder::new(std::env::current_dir().unwrap(), "test")
        .with_config(PluginsConfig {
            list,
            ..Default::default()
        })
        .register_static("capability-plugin", CapabilityPlugin::new(capabilities))
        .build()
        .await
        .expect("host builds")
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_requires_declared_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_CONFIG_READ,
            json!({ "path": null }),
        )
        .await
        .expect_err("read_config should require ReadConfig capability");

    assert_eq!(err.code, PluginErrorCode::HostUnavailable);
    assert!(err.message.contains("ReadConfig"));
    assert!(
        err.message
            .contains(agena_plugin_sdk::rpc::method::HOST_CONFIG_READ)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_allows_declared_capability() {
    let host = host_with_capability_plugin(vec![HostCapability::ReadConfig]).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let value = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_CONFIG_READ,
            json!({ "path": null }),
        )
        .await
        .expect("read_config should be allowed");

    assert_eq!(value, json!({ "ok": true }));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_tools_requires_capability_but_log_does_not() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    host.host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_LOG,
            json!({ "level": "info", "message": "hello" }),
        )
        .await
        .expect("log should remain available without capabilities");

    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_TOOL_LIST,
            json!({}),
        )
        .await
        .expect_err("list_tools should require ListTools capability");

    assert!(err.message.contains("ListTools"));
}

#[derive(Default)]
struct TestPlugin;

#[async_trait]
impl Plugin for TestPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("test", "0.1.0")
            .hooks(
                HookSubscription::TOOL_INVOKE
                    | HookSubscription::TOOL_BEFORE
                    | HookSubscription::TOOL_AFTER
                    | HookSubscription::SHELL_ENV
                    | HookSubscription::CHAT_PARAMS,
            )
            .entry(
                PluginEntryDecl::new(
                    "ping",
                    json!({"type":"object","properties":{"text":{"type":"string"}}}),
                )
                .description("returns 'pong: <text>'"),
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
        Ok(ToolInvokeOutput::text(format!("pong: {text}")))
    }

    async fn tool_execute_before(&self, _: ToolBeforeInput) -> Result<Option<ToolBeforePatch>> {
        let mut meta = BTreeMap::new();
        meta.insert("touched".into(), "yes".into());
        Ok(Some(ToolBeforePatch {
            metadata: meta,
            ..Default::default()
        }))
    }

    async fn shell_env(&self, _: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(Some(ShellEnvPatch::set("AGENA_TEST", "1")))
    }

    async fn chat_params(&self, _: ChatParamsInput) -> Result<Option<ChatParamsPatch>> {
        Ok(Some(ChatParamsPatch {
            params: Some(json!({ "temperature": 0.5 })),
        }))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn static_plugin_round_trips_every_hook() {
    let mut list = BTreeMap::new();
    list.insert(
        "test".to_string(),
        PluginEntry::Static {
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
        })
        .register_static("test", TestPlugin)
        .build()
        .await
        .expect("plugin host should build");

    assert_eq!(host.plugins().len(), 1);
    let resolved = host.lookup_entry("ping").expect("ping exposed");
    assert_eq!(resolved.handle.original_name, "ping");

    // tool_invoke
    let out = host
        .invoke_tool(
            &resolved.handle,
            ToolInvokeInput {
                tool_name: "ping".into(),
                session_id: 1,
                call_id: 1,
                workspace_root: ".".into(),
                input: json!({ "text": "hi" }),
            },
        )
        .expect("invoke");
    assert_eq!(out.output_text, "pong: hi");

    // shell_env
    let patch = host
        .dispatch_shell_env(ShellEnvInput {
            cwd: ".".into(),
            session_id: None,
            call_id: None,
        })
        .expect("shell_env");
    assert_eq!(patch.set.get("AGENA_TEST").map(String::as_str), Some("1"));

    // chat_params (async)
    let updated = host
        .dispatch_chat_params(ChatParamsInput {
            provider: "openai".into(),
            model: "gpt".into(),
            params: json!({}),
        })
        .await
        .expect("chat_params");
    assert_eq!(updated.params.get("temperature"), Some(&json!(0.5)));
}

#[derive(Default)]
struct CompactionSummaryPlugin;

#[async_trait]
impl Plugin for CompactionSummaryPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("summary-replacer", "0.1.0")
            .hooks(HookSubscription::SESSION_COMPACTING)
            .build()
    }

    async fn session_compacting(
        &self,
        _input: SessionCompactingInput,
    ) -> Result<Option<SessionCompactingPatch>> {
        Ok(Some(SessionCompactingPatch {
            summary: Some("plugin-supplied summary".into()),
            ..Default::default()
        }))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_compacting_patch_can_replace_summary() {
    let mut list = BTreeMap::new();
    list.insert(
        "summary-replacer".to_string(),
        PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
        },
    );
    let host = PluginHostBuilder::new(std::env::current_dir().unwrap(), "test")
        .with_config(PluginsConfig {
            list,
            ..Default::default()
        })
        .register_static("summary-replacer", CompactionSummaryPlugin)
        .build()
        .await
        .expect("host builds");

    let outcome = host
        .dispatch_session_compacting(SessionCompactingInput {
            session_id: 7,
            messages: vec![ChatMessage {
                role: "user".into(),
                content: json!({"text": "hi"}),
            }],
            strategy: "summarize".into(),
        })
        .await
        .expect("dispatch");
    assert_eq!(outcome.summary.as_deref(), Some("plugin-supplied summary"));
    assert_eq!(outcome.messages.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn entry_register_requires_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_ENTRY_REGISTER,
            json!({
                "request": {
                    "entry": PluginEntryDecl::new("dynamic", json!({"type": "object"})),
                },
            }),
        )
        .await
        .expect_err("entry.register should require EntryRegistry capability");
    assert!(err.message.contains("EntryRegistry"));
}

#[tokio::test(flavor = "multi_thread")]
async fn entry_register_then_lookup_resolves() {
    let host = host_with_capability_plugin(vec![HostCapability::EntryRegistry]).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let response = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_ENTRY_REGISTER,
            json!({
                "request": {
                    "entry": PluginEntryDecl::new("dynamic-entry", json!({"type": "object"})),
                },
            }),
        )
        .await
        .expect("entry.register should succeed");
    assert!(response.get("generation").and_then(|v| v.as_u64()).unwrap() > 0);

    let resolved = host
        .lookup_entry("dynamic-entry")
        .expect("dynamic entry should resolve via lookup");
    assert_eq!(resolved.handle.plugin_id, "capability-plugin");
    assert_eq!(resolved.handle.original_name, "dynamic-entry");

    let removed = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_ENTRY_REMOVE,
            json!({
                "request": {
                    "name": "dynamic-entry",
                    "exposed": false,
                },
            }),
        )
        .await
        .expect("entry.remove should succeed");
    assert!(
        removed
            .get("exposed_name")
            .and_then(|v| v.as_str())
            .is_some()
    );
    assert!(host.lookup_entry("dynamic-entry").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_and_secret_calls_require_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let storage_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_STORAGE_GET,
            json!({ "request": { "namespace": "ns", "key": "k" } }),
        )
        .await
        .expect_err("storage.get should require PluginStorage");
    assert!(storage_err.message.contains("PluginStorage"));

    let secret_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_SECRET_LIST,
            json!({}),
        )
        .await
        .expect_err("secret.list should require PluginSecrets");
    assert!(secret_err.message.contains("PluginSecrets"));
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_and_secret_capability_unlocks_dispatch() {
    let host = host_with_capability_plugin(vec![
        HostCapability::PluginStorage,
        HostCapability::PluginSecrets,
    ])
    .await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    // FakeHostClient does not implement storage_get; the default trait method
    // returns HostUnavailable, which proves the capability gate let the call
    // through to the underlying host implementation.
    let storage_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_STORAGE_GET,
            json!({ "request": { "namespace": "ns", "key": "k" } }),
        )
        .await
        .expect_err("storage.get should reach default trait method");
    assert!(!storage_err.message.contains("PluginStorage"));

    let secret_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_SECRET_LIST,
            json!({}),
        )
        .await
        .expect_err("secret.list should reach default trait method");
    assert!(!secret_err.message.contains("PluginSecrets"));
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_status_calls_require_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_PLUGIN_STATUS_LIST,
            json!({}),
        )
        .await
        .expect_err("plugin.status.list should require PluginStatus");
    assert!(err.message.contains("PluginStatus"));
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_status_capability_returns_loaded_plugin() {
    let host = host_with_capability_plugin(vec![HostCapability::PluginStatus]).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let response = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_PLUGIN_STATUS_LIST,
            json!({}),
        )
        .await
        .expect("plugin.status.list should succeed");

    let entries = response
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        entries
            .iter()
            .any(|entry| entry.get("plugin_id").and_then(|v| v.as_str())
                == Some("capability-plugin")
                && entry.get("kind").and_then(|v| v.as_str()) == Some("static")
                && entry.get("state").and_then(|v| v.as_str()) == Some("running"))
    );

    let single = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_PLUGIN_STATUS_GET,
            json!({ "request": { "plugin_id": "capability-plugin" } }),
        )
        .await
        .expect("plugin.status.get should succeed");
    assert_eq!(
        single
            .get("status")
            .and_then(|v| v.get("state"))
            .and_then(|v| v.as_str()),
        Some("running")
    );

    let missing = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_PLUGIN_STATUS_GET,
            json!({ "request": { "plugin_id": "no-such-plugin" } }),
        )
        .await
        .expect("plugin.status.get on missing plugin should succeed");
    assert!(missing.get("status").is_none_or(|v| v.is_null()));
}
