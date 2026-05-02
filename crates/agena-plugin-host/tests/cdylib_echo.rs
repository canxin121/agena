//! Loads the cdylib `agena-echo-plugin` example via `CdylibTransport` and
//! verifies the SDK's `export_cdylib!` macro produces an FFI surface the
//! host can drive end-to-end.
//!
//! Skipped automatically if the artifact has not been built (CI builds it
//! before running this test).

use std::collections::BTreeMap;
use std::path::PathBuf;

use agena_plugin_host::{PluginEntry, PluginHostBuilder, PluginsConfig};
use agena_plugin_sdk::ToolInvokeInput;
use serde_json::json;

fn echo_plugin_path() -> Option<PathBuf> {
    // examples/echo_plugin/target/{debug,release}/libagena_echo_plugin.{so,dylib,dll}
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .to_path_buf();
    let base = workspace.join("examples/echo_plugin/target/debug");
    for name in [
        "libagena_echo_plugin.so",
        "libagena_echo_plugin.dylib",
        "agena_echo_plugin.dll",
    ] {
        let candidate = base.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn cdylib_echo_plugin_loads_and_invokes() {
    let Some(path) = echo_plugin_path() else {
        eprintln!("skip: examples/echo_plugin has not been built");
        return;
    };

    let mut list = BTreeMap::new();
    list.insert(
        "echo".to_string(),
        PluginEntry::Cdylib {
            path,
            options: json!({ "uppercase": true }),
            timeouts: Default::default(),
            sha256: None,
            signature: None,
        },
    );
    let host = PluginHostBuilder::new(std::env::current_dir().unwrap(), "test")
        .with_config(PluginsConfig {
            enabled: true,
            timeouts: Default::default(),
            list,
            trusted_keys: Default::default(),
        })
        .build()
        .await
        .expect("plugin host should build");

    assert_eq!(host.plugins().len(), 1, "one plugin should be loaded");

    let resolved = host.lookup_entry("echo").expect("echo tool exposed");
    let out = host
        .invoke_tool(
            &resolved.handle,
            ToolInvokeInput {
                tool_name: "echo".into(),
                session_id: 1,
                call_id: 1,
                workspace_root: ".".into(),
                input: json!({ "text": "hello" }),
            },
        )
        .expect("invoke");

    // `uppercase = true` was set in options, so the plugin should uppercase.
    assert_eq!(out.output_text, "HELLO");
}
