//! The engine-side effect gate is gone: a plugin that performs its own I/O
//! is no longer preflighted from a declaration. What remains here is the
//! boundary the host can still enforce — a path the policy refuses is refused
//! for the plugin's own query too, so a plugin that asks before it touches
//! anything gets the same answer a host tool would.
use agena_domain::{PermissionDecision, PermissionMode, StructuredObject, ToolInvocation};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_runtime_tools::{
    authorization::ExecutionPrincipal,
    permission::{NetworkPermissionPolicy, PermissionPolicy, ToolPermissionPolicy},
    tool::ToolExecutor,
};
use serde_json::{Value, json};
use std::collections::HashMap;

async fn fixture(
    read: PermissionMode,
    write: PermissionMode,
    network: PermissionMode,
) -> (tempfile::TempDir, ToolExecutor) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().canonicalize().unwrap();
    let mut config = PluginsConfig::default();
    for name in ["agena.chatgpt", "agena.claude", "agena.gemini", "agena.web"] {
        config
            .list
            .insert(name.into(), ConfiguredPlugin::static_default());
    }
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.chatgpt".parse().unwrap(),
                agena_bundled_plugins::tool::new_chatgpt_plugin(),
            ),
            StaticPluginRegistration::new(
                "agena.claude".parse().unwrap(),
                agena_bundled_plugins::tool::new_claude_plugin(),
            ),
            StaticPluginRegistration::new(
                "agena.gemini".parse().unwrap(),
                agena_bundled_plugins::tool::new_gemini_plugin(),
            ),
            StaticPluginRegistration::new(
                "agena.web".parse().unwrap(),
                agena_bundled_plugins::tool::new_web_plugin(),
            ),
        ],
        config,
        workspace_root: workspace.clone(),
        agena_version: "audit".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .unwrap();
    let mut principal = ExecutionPrincipal::new(
        PermissionPolicy::new(read, write),
        ToolPermissionPolicy::allow_all(),
    );
    principal.network_policy = NetworkPermissionPolicy::new(network);
    (
        directory,
        ToolExecutor::new(workspace, principal, plugins, None, None, None),
    )
}

/// The decision a host tool invocation on `name` would receive. Only the
/// tool-level check remains; the host resolves path and network arguments of
/// its own executor-backed builtins, and a plugin-handled tool such as these
/// cloud-media tools is checked at the tool-name level here.
async fn decision_for(
    executor: &ToolExecutor,
    name: &str,
    value: Value,
) -> Vec<PermissionDecision> {
    let call = ToolInvocation::new(name, StructuredObject::try_from(value).unwrap());
    let prepared = executor.prepare_invocation(&call, 41, 1).await.unwrap();
    let checks = executor
        .collect_permission_checks_for_invocation_in_session(&prepared.invocation, Some(41))
        .await
        .unwrap();
    checks.into_iter().map(|check| check.decision).collect()
}

#[tokio::test]
async fn cloud_media_tools_still_reach_the_tool_gate() {
    let (_dir, executor) = fixture(
        PermissionMode::Deny,
        PermissionMode::Allow,
        PermissionMode::Allow,
    )
    .await;
    for provider in ["chatgpt", "claude", "gemini"] {
        for operation in [
            "image_understanding",
            "document_understanding",
            "file_upload",
            "file_status",
            "file_delete",
        ] {
            let input = match operation {
                "image_understanding" | "document_understanding" => {
                    json!({"inputs":[{"source":"local","path":"fixture.png"}],"prompt":"fixture"})
                }
                "file_upload" => json!({"path":"fixture.png"}),
                _ => json!({"handle":"media_00000000000000000000000000000000"}),
            };
            let name = format!("{provider}.cloud_{operation}");
            assert!(
                !decision_for(&executor, &name, input).await.is_empty(),
                "missing tool decision for {name}"
            );
        }
    }
}
