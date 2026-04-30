//! End-to-end test: build a `PluginHost` containing a static plugin, fire
//! every relevant hook, assert the patches chain.

use std::collections::BTreeMap;

use agena_plugin_host::{PluginEntry, PluginHostBuilder, PluginsConfig};
use agena_plugin_sdk::prelude::*;

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
                ToolDecl::new(
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

    async fn tool_execute_before(
        &self,
        _: ToolBeforeInput,
    ) -> Result<Option<ToolBeforePatch>> {
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

    async fn chat_params(
        &self,
        _: ChatParamsInput,
    ) -> Result<Option<ChatParamsPatch>> {
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
    let resolved = host.lookup_tool("ping").expect("ping exposed");
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
