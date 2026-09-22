use agena_domain::{StructuredObject, ToolInvocation};
use agena_plugin_host::sdk::{PluginError, Result as SdkResult, ToolAfterInput, ToolAfterPatch};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_runtime_tools::{
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    tool::ToolExecutor,
};
use serde_json::json;
use std::collections::HashMap;

struct BrokenAfterHook;
#[agena_plugin_host::sdk::agena_plugin(
    namespace = "audit",
    name = "broken_after",
    version = "0.1.0",
    summary = "Post-execution failure fixture"
)]
impl BrokenAfterHook {
    #[hook(tool.after)]
    async fn after(&self, _input: ToolAfterInput) -> SdkResult<Option<ToolAfterPatch>> {
        Err(PluginError::internal(
            "injected post-commit rendering failure",
        ))
    }
}
#[tokio::test]
async fn successful_write_survives_an_after_hook_failure_with_explicit_warning() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().canonicalize().unwrap();
    let mut config = PluginsConfig::default();
    for name in ["agena.fs", "audit.broken_after"] {
        config
            .list
            .insert(name.into(), ConfiguredPlugin::static_default());
    }
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.fs".parse().unwrap(),
                agena_bundled_plugins::tool::new_fs_plugin(),
            ),
            StaticPluginRegistration::new("audit.broken_after".parse().unwrap(), BrokenAfterHook),
        ],
        config,
        workspace_root: workspace.clone(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        workspace.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins,
        None,
        None,
        None,
    );
    let invocation = ToolInvocation::new(
        "fs.write",
        StructuredObject::try_from(json!({"path":"committed.txt","content":"exactly once"}))
            .unwrap(),
    );
    let prepared = executor
        .prepare_invocation(&invocation, 41, 1)
        .await
        .unwrap();
    let result = executor
        .execute_invocation_detailed(&prepared.invocation, 41, 1)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.join("committed.txt")).unwrap(),
        "exactly once"
    );
    assert_eq!(
        result
            .view
            .metadata
            .get("postprocessing_state")
            .map(String::as_str),
        Some("failed")
    );
    assert!(result.view.output_text.contains("do not repeat"));
    assert!(!result.output.payload.is_empty());
}
