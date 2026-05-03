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
        if let Some(entry_name) = agena_plugin_sdk::host_api::current_host_callback_context()
            .and_then(|context| context.entry_name)
        {
            meta.insert("before_entry".into(), entry_name);
        }
        Ok(Some(ToolBeforePatch {
            metadata: meta,
            ..Default::default()
        }))
    }

    async fn tool_execute_after(&self, _: ToolAfterInput) -> Result<Option<ToolAfterPatch>> {
        let mut meta = BTreeMap::new();
        if let Some(entry_name) = agena_plugin_sdk::host_api::current_host_callback_context()
            .and_then(|context| context.entry_name)
        {
            meta.insert("after_entry".into(), entry_name);
        }
        Ok(Some(ToolAfterPatch {
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
            default_quota: Default::default(),
            quotas: Default::default(),
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

    // tool hooks carry the active entry name into callback context.
    let before = host
        .dispatch_tool_before(ToolBeforeInput {
            tool_name: "ping".into(),
            source: EntrySource::Plugin {
                plugin: "test".into(),
            },
            session_id: 1,
            call_id: 1,
            workspace_root: ".".into(),
            input: json!({}),
            title_override: None,
            metadata: Default::default(),
        })
        .expect("tool_before");
    assert_eq!(
        before.metadata.get("before_entry").map(String::as_str),
        Some("ping")
    );

    let after = host
        .dispatch_tool_after(ToolAfterInput {
            tool_name: "ping".into(),
            source: EntrySource::Plugin {
                plugin: "test".into(),
            },
            session_id: 1,
            call_id: 1,
            workspace_root: ".".into(),
            title: "done".into(),
            output_text: "ok".into(),
            payload: None,
            metadata: Default::default(),
        })
        .expect("tool_after");
    assert_eq!(
        after.metadata.get("after_entry").map(String::as_str),
        Some("ping")
    );

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

#[tokio::test(flavor = "multi_thread")]
async fn lsp_calls_require_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_LSP_LIST_SERVERS,
            json!({}),
        )
        .await
        .expect_err("lsp.list_servers should require LspRegistry");
    assert!(err.message.contains("LspRegistry"));
}

#[tokio::test(flavor = "multi_thread")]
async fn lsp_capability_unlocks_dispatch() {
    let host = host_with_capability_plugin(vec![HostCapability::LspRegistry]).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_LSP_LIST_SERVERS,
            json!({}),
        )
        .await
        .expect_err("default lsp_list_servers should reach trait fallback");
    assert!(!err.message.contains("LspRegistry"));
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_worktree_scheduler_calls_require_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let plan_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_PLAN_LIST,
            json!({}),
        )
        .await
        .expect_err("plan.list should require PlanRegistry");
    assert!(plan_err.message.contains("PlanRegistry"));

    let worktree_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_WORKTREE_LIST,
            json!({}),
        )
        .await
        .expect_err("worktree.list should require WorktreeRegistry");
    assert!(worktree_err.message.contains("WorktreeRegistry"));

    let scheduler_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_SCHEDULER_LIST,
            json!({}),
        )
        .await
        .expect_err("scheduler.list should require Scheduler");
    assert!(scheduler_err.message.contains("Scheduler"));
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_worktree_scheduler_capability_unlocks_dispatch() {
    let host = host_with_capability_plugin(vec![
        HostCapability::PlanRegistry,
        HostCapability::WorktreeRegistry,
        HostCapability::Scheduler,
    ])
    .await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    for method in [
        agena_plugin_sdk::rpc::method::HOST_PLAN_LIST,
        agena_plugin_sdk::rpc::method::HOST_WORKTREE_LIST,
        agena_plugin_sdk::rpc::method::HOST_SCHEDULER_LIST,
    ] {
        let err = host
            .host_handle()
            .handle_call_for_plugin("capability-plugin", method, json!({}))
            .await
            .expect_err("default trait method should be HostUnavailable, not capability error");
        assert!(!err.message.contains("PlanRegistry"));
        assert!(!err.message.contains("WorktreeRegistry"));
        assert!(!err.message.contains("Scheduler"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn command_and_agent_calls_require_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let cmd_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_COMMAND_LIST,
            json!({}),
        )
        .await
        .expect_err("command.list should require CommandRegistry");
    assert!(cmd_err.message.contains("CommandRegistry"));

    let agent_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_AGENT_LIST,
            json!({}),
        )
        .await
        .expect_err("agent.list should require AgentRegistry");
    assert!(agent_err.message.contains("AgentRegistry"));
}

#[tokio::test(flavor = "multi_thread")]
async fn hook_and_mcp_calls_require_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let hook_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_HOOK_LIST,
            json!({}),
        )
        .await
        .expect_err("hook.list should require HookRegistry");
    assert!(hook_err.message.contains("HookRegistry"));

    let mcp_err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_MCP_LIST_SERVERS,
            json!({}),
        )
        .await
        .expect_err("mcp.list_servers should require McpRegistry");
    assert!(mcp_err.message.contains("McpRegistry"));
}

#[tokio::test(flavor = "multi_thread")]
async fn hook_capability_returns_loaded_plugin_capabilities() {
    let host = host_with_capability_plugin(vec![HostCapability::HookRegistry]).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let response = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_HOOK_LIST,
            json!({}),
        )
        .await
        .expect("hook.list should be allowed");
    let entries = response
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        entries
            .iter()
            .any(|entry| entry.get("plugin_id").and_then(|v| v.as_str())
                == Some("capability-plugin"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_statusline_round_trip_under_capability() {
    let host = host_with_capability_plugin(vec![HostCapability::Statusline]).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    host.host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_UI_STATUSLINE_CONTRIBUTE,
            json!({
                "request": {
                    "segment_id": "branch",
                    "content": "main",
                    "priority": 10,
                }
            }),
        )
        .await
        .expect("statusline.contribute should succeed");

    let listed = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_UI_STATUSLINE_LIST,
            json!({}),
        )
        .await
        .expect("statusline.list should succeed");
    assert_eq!(
        listed
            .get("segments")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|seg| seg.get("content"))
            .and_then(|v| v.as_str()),
        Some("main")
    );

    let removed = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_UI_STATUSLINE_REMOVE,
            json!({ "request": { "segment_id": "branch" } }),
        )
        .await
        .expect("statusline.remove should succeed");
    assert_eq!(removed.get("removed").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_theme_calls_require_capability() {
    let host = host_with_capability_plugin(Vec::new()).await;
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_UI_THEME_LIST,
            json!({}),
        )
        .await
        .expect_err("theme.list should require Theme capability");
    assert!(err.message.contains("Theme"));
}

/// A plugin shipping two entries: only entry `loud` declares `ReadConfig`;
/// entry `quiet` declares nothing. With per-entry scoping the same host
/// call must succeed under `loud` and fail under `quiet`, regardless of
/// the plugin-level union.
#[derive(Clone)]
struct TwoEntryPlugin;

#[async_trait]
impl Plugin for TwoEntryPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("two-entry", "0.1.0")
            .hooks(HookSubscription::TOOL_INVOKE)
            .entry(
                PluginEntryDecl::new("loud", json!({})).host_capability(HostCapability::ReadConfig),
            )
            .entry(PluginEntryDecl::new("quiet", json!({})))
            .build()
    }

    async fn tool_invoke(&self, _input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("ok"))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn per_entry_capability_scope_isolates_entries() {
    use agena_plugin_sdk::host_api::{HostCallbackContext, with_host_callback_context};

    let mut list = BTreeMap::new();
    list.insert(
        "two-entry".to_string(),
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
        .register_static("two-entry", TwoEntryPlugin)
        .build()
        .await
        .expect("host builds");
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    // Under entry `loud`: ReadConfig is declared, call must succeed.
    let value = with_host_callback_context(
        HostCallbackContext {
            entry_name: Some("loud".into()),
            ..Default::default()
        },
        host.host_handle().handle_call_for_plugin(
            "two-entry",
            agena_plugin_sdk::rpc::method::HOST_CONFIG_READ,
            json!({ "path": null }),
        ),
    )
    .await
    .expect("loud entry has ReadConfig");
    assert_eq!(value, json!({ "ok": true }));

    // Under entry `quiet`: no caps declared; per-entry deny must apply,
    // not fall through to the plugin-level union.
    let err = with_host_callback_context(
        HostCallbackContext {
            entry_name: Some("quiet".into()),
            ..Default::default()
        },
        host.host_handle().handle_call_for_plugin(
            "two-entry",
            agena_plugin_sdk::rpc::method::HOST_CONFIG_READ,
            json!({ "path": null }),
        ),
    )
    .await
    .expect_err("quiet entry must be denied");
    assert_eq!(err.code, PluginErrorCode::HostUnavailable);
    assert!(err.message.contains("entry `quiet`"));
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_burst_then_throttles() {
    use agena_plugin_host::quota::QuotaConfig;

    let mut list = BTreeMap::new();
    list.insert(
        "capability-plugin".to_string(),
        PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
        },
    );
    let mut quotas = BTreeMap::new();
    quotas.insert(
        "capability-plugin".to_string(),
        QuotaConfig {
            rate_per_sec: 1,
            burst: 2,
            max_concurrent: 0,
        },
    );
    let host = PluginHostBuilder::new(std::env::current_dir().unwrap(), "test")
        .with_config(PluginsConfig {
            list,
            quotas,
            ..Default::default()
        })
        .register_static(
            "capability-plugin",
            CapabilityPlugin::new([HostCapability::ReadConfig]),
        )
        .build()
        .await
        .expect("host builds");
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    for _ in 0..2 {
        host.host_handle()
            .handle_call_for_plugin(
                "capability-plugin",
                agena_plugin_sdk::rpc::method::HOST_CONFIG_READ,
                json!({ "path": null }),
            )
            .await
            .expect("within burst");
    }
    let err = host
        .host_handle()
        .handle_call_for_plugin(
            "capability-plugin",
            agena_plugin_sdk::rpc::method::HOST_CONFIG_READ,
            json!({ "path": null }),
        )
        .await
        .expect_err("third call must throttle");
    assert_eq!(err.code, PluginErrorCode::Generic);
    assert!(err.message.contains("rate exceeded"));
}

/// A plugin that registers itself as the permission handler. The host
/// dispatches HOST_PERMISSION_ASK to its `permission_ask` hook when the
/// handler is set.
#[derive(Clone)]
struct PermissionUiPlugin;

#[async_trait]
impl Plugin for PermissionUiPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("perm-ui", "0.1.0")
            .hooks(HookSubscription::TOOL_INVOKE | HookSubscription::PERMISSION_ASK)
            .entry(
                PluginEntryDecl::new("ui", json!({})).host_capability(HostCapability::PermissionUi),
            )
            .build()
    }

    async fn tool_invoke(&self, _input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("noop"))
    }

    async fn permission_ask(
        &self,
        _input: PermissionAskInput,
    ) -> Result<Option<PermissionAskDecision>> {
        Ok(Some(PermissionAskDecision::Decide(
            PermissionDecision::Allow,
        )))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_ui_handler_dispatch_routes_to_plugin() {
    let mut list = BTreeMap::new();
    list.insert(
        "perm-ui".to_string(),
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
        .register_static("perm-ui", PermissionUiPlugin)
        .build()
        .await
        .expect("host builds");
    host.host_handle()
        .install_client(Arc::new(FakeHostClient))
        .await;

    // Register the handler.
    let _ = host
        .host_handle()
        .handle_call_for_plugin(
            "perm-ui",
            agena_plugin_sdk::rpc::method::HOST_UI_PERMISSION_SET_HANDLER,
            json!({}),
        )
        .await
        .expect("set handler");
    assert_eq!(
        host.host_handle().permission_handler().await.as_deref(),
        Some("perm-ui")
    );

    // A plain HOST_PERMISSION_ASK call should now hit the plugin's
    // PLUGIN_PERMISSION_RENDER and bring back our canned decision.
    let result = host
        .host_handle()
        .handle_call_for_plugin(
            "some-other-plugin",
            agena_plugin_sdk::rpc::method::HOST_PERMISSION_ASK,
            json!({
                "session_id": 1,
                "action": "fs.read",
                "subject": {},
                "default_decision": "prompt"
            }),
        )
        .await
        .expect("ask routed");
    assert_eq!(
        result.as_str(),
        Some("allow"),
        "expected handler decision 'allow', got {result}"
    );

    // Clearing reverts to the FakeHostClient (Prompt).
    host.host_handle()
        .handle_call_for_plugin(
            "perm-ui",
            agena_plugin_sdk::rpc::method::HOST_UI_PERMISSION_CLEAR_HANDLER,
            json!({}),
        )
        .await
        .expect("clear handler");
    assert!(host.host_handle().permission_handler().await.is_none());
}
