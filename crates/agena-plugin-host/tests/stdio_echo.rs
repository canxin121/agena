//! End-to-end stdio transport test. Spawns the `agena-echo-plugin-stdio`
//! example binary, runs `meta/init` + `tool.invoke` + `shell.env` against it,
//! and asserts patches round-trip correctly.

use std::collections::BTreeMap;
use std::path::PathBuf;

use agena_plugin_host::{PluginEntry, PluginHostBuilder, PluginsConfig};
use agena_plugin_sdk::ToolInvokeInput;
use serde_json::json;

fn stdio_binary_path() -> Option<PathBuf> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .to_path_buf();
    let base = workspace.join("examples/echo_plugin_stdio/target/debug");
    for name in ["agena-echo-plugin-stdio", "agena-echo-plugin-stdio.exe"] {
        let candidate = base.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_echo_plugin_loads_and_invokes() {
    let Some(path) = stdio_binary_path() else {
        eprintln!("skip: examples/echo_plugin_stdio has not been built");
        return;
    };

    let mut list = BTreeMap::new();
    list.insert(
        "echo-stdio".to_string(),
        PluginEntry::Stdio {
            command: path.to_string_lossy().to_string(),
            args: Vec::new(),
            env: Default::default(),
            cwd: None,
            restart: Default::default(),
            options: serde_json::Value::Null,
            timeouts: Default::default(),
            sha256: None,
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

    assert_eq!(host.plugins().len(), 1, "stdio plugin should load");

    let resolved = host.lookup_entry("echo").expect("tool exposed");
    let out = host
        .invoke_tool(
            &resolved.handle,
            ToolInvokeInput {
                tool_name: "echo".into(),
                session_id: 7,
                call_id: 11,
                workspace_root: ".".into(),
                input: json!({ "text": "hi" }),
            },
        )
        .expect("invoke");
    assert_eq!(out.output_text, "stdio-echo: hi");

    // shell_env round-trip via stdio
    let patch = host
        .dispatch_shell_env(agena_plugin_sdk::ShellEnvInput {
            cwd: ".".into(),
            session_id: None,
            call_id: None,
        })
        .expect("shell_env");
    assert_eq!(
        patch.set.get("AGENA_STDIO_PLUGIN").map(String::as_str),
        Some("1")
    );

    // chat_params patch via async dispatch
    let updated = host
        .dispatch_chat_params(agena_plugin_sdk::ChatParamsInput {
            provider: "openai".into(),
            model: "gpt".into(),
            params: json!({}),
        })
        .await
        .expect("chat_params");
    assert!(updated.params.get("stop").is_some());

    host.shutdown().await;
}
