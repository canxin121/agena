//! No provider request or browser process is launched: exercise the same
//! dynamic effect inspection used by the runtime permission gate.
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
fn tools() -> Vec<(String, Value)> {
    let groups = [
        (
            "chatgpt",
            "apply_patch code_interpreter computer computer_use_preview custom file_search function image_edit image_generation local_shell mcp namespace programmatic_tool_calling shell tool_search web_search web_search_preview",
        ),
        (
            "claude",
            "advisor bash code_execution computer mcp_toolset memory text_editor tool_search_bm25 tool_search_regex web_fetch web_search",
        ),
        (
            "gemini",
            "code_execution computer_use file_search function google_maps google_search image_edit image_generation mcp_server retrieval url_context",
        ),
    ];
    groups
        .iter()
        .flat_map(|(family, names)| {
            names.split_whitespace().map(move |name| {
                let mut input = json!({"prompt":"fixture","model":"audit-model"});
                if name == "image_edit" {
                    input["images"] = json!(["fixture.png"]);
                }
                (format!("{family}.{name}"), input)
            })
        })
        .collect()
}
async fn has_denial(executor: &ToolExecutor, name: &str, value: Value) -> bool {
    let call = ToolInvocation::new(name, StructuredObject::try_from(value).unwrap());
    let prepared = executor.prepare_invocation(&call, 41, 1).await.unwrap();
    let checks = executor
        .collect_permission_checks_for_invocation_in_session(&prepared.invocation, Some(41))
        .await
        .unwrap();
    checks
        .iter()
        .any(|check| matches!(check.decision, PermissionDecision::Deny { .. }))
}
#[tokio::test]
async fn every_provider_tool_declares_its_actual_network_target() {
    let (_dir, executor) = fixture(
        PermissionMode::Allow,
        PermissionMode::Allow,
        PermissionMode::Deny,
    )
    .await;
    let tools = tools();
    assert_eq!(tools.len(), 39);
    for (name, input) in tools {
        assert!(
            has_denial(&executor, &name, input).await,
            "missing network denial for {name}"
        );
    }
}
#[tokio::test]
async fn every_provider_tool_declares_receipt_and_image_writes() {
    let (_dir, executor) = fixture(
        PermissionMode::Allow,
        PermissionMode::Deny,
        PermissionMode::Allow,
    )
    .await;
    for (name, input) in tools() {
        assert!(
            has_denial(&executor, &name, input).await,
            "missing artifact write denial for {name}"
        );
    }
}
#[tokio::test]
async fn browser_artifact_paths_are_checked_before_starting_a_browser() {
    let (_dir, executor) = fixture(
        PermissionMode::Allow,
        PermissionMode::Deny,
        PermissionMode::Allow,
    )
    .await;
    for (name, input) in [
        (
            "web.browser_screenshot",
            json!({"session_id":"nonexistent","path":"forbidden.png"}),
        ),
        (
            "web.browser_screenshot",
            json!({"session_id":"nonexistent"}),
        ),
        (
            "web.browser_download",
            json!({"session_id":"nonexistent","url":"https://example.invalid/file"}),
        ),
    ] {
        assert!(
            has_denial(&executor, name, input).await,
            "missing write denial for {name}"
        );
    }
    assert!(!executor.workspace_root().join(".agena/artifacts").exists());
}
#[tokio::test]
async fn browser_initial_navigation_declares_network_access() {
    let (_dir, executor) = fixture(
        PermissionMode::Allow,
        PermissionMode::Allow,
        PermissionMode::Deny,
    )
    .await;
    assert!(
        has_denial(
            &executor,
            "web.browser_open",
            json!({"url":"https://example.invalid/"})
        )
        .await
    );
}
