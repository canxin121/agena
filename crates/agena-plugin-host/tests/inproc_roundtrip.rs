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

    async fn check_path_permission(
        &self,
        _req: HostPathPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse> {
        Ok(HostPermissionCheckResponse::allowed())
    }

    async fn check_network_permission(
        &self,
        _req: HostNetworkPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse> {
        Ok(HostPermissionCheckResponse::allowed())
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
            summary: None,
            help: None,
            input_schema: None,
            description_mode: None,
            tags: vec![ToolTag::ReadOnly],
            deferred: false,
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
        let mut entry = PluginToolDecl::new("ping", json!({ "type": "object" }));
        for capability in &self.capabilities {
            entry = entry.host_capability(*capability);
        }
        PluginManifest::builder("capability-plugin", "0.1.0")
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(entry)
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
            disabled: false,
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
            .tool(
                PluginToolDecl::new(
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
        .register_static("test", TestPlugin)
        .build()
        .await
        .expect("plugin host should build");

    assert_eq!(host.plugins().len(), 1);
    let resolved = host.lookup_entry("ping").expect("ping exposed");
    assert_eq!(resolved.original_name, "ping");

    // tool_invoke
    let out = host
        .invoke_tool(
            &resolved,
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

    // tool hooks carry the active tool name into callback context.
    let before = host
        .dispatch_tool_before(ToolBeforeInput {
            tool_name: "ping".into(),
            plugin_name: "test".into(),
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
            plugin_name: "test".into(),
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
            .tool(
                PluginToolDecl::new("loud", json!({})).host_capability(HostCapability::ReadConfig),
            )
            .tool(PluginToolDecl::new("quiet", json!({})))
            .build()
    }

    async fn tool_invoke(&self, _input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("noop"))
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
            disabled: false,
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
            disabled: false,
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
            .tool(
                PluginToolDecl::new("ui", json!({})).host_capability(HostCapability::PermissionUi),
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
        Ok(Some(PermissionAskDecision::Advise(PermissionAdvice {
            decision: PermissionDecision::Allow,
            reason: "ui handler approved".to_string(),
            risk: PermissionRiskLevel::Low,
            requested_scope: None,
        })))
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
            disabled: false,
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
