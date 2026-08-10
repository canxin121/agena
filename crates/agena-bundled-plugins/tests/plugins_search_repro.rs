//! Deterministic reproduction harness for the production `plugins_search`
//! hang (sessions 5/6/7 froze exactly on `plugins_search` + `plugins_tags`
//! with all threads parked and the run task awaiting a no-deadline future).
//!
//! This drives the *exact* production dispatch route for these tools:
//! ToolApiPlugin (`agena.tools`) as an in-proc static plugin, resolved
//! through the ToolExecutor's non-payload path (plugins_search/tags are not
//! in `ToolPayloadInput::from_executor_backed_invocation`'s builtin set) into
//! `invoke_tool_cancellable` -> in-proc transport dispatch -> the plugin's
//! `list_plugins` host callback.

use std::{collections::HashMap, sync::Arc};

use agena_domain::StructuredObject;
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_runtime_tools::{
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    tool::ToolExecutor,
};

async fn build_tool_api_executor() -> (ToolExecutor, std::path::PathBuf) {
    let workspace_root = std::env::temp_dir().join(format!(
        "agena-plugins-search-repro-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&workspace_root).expect("create repro workspace");

    let mut plugins_config = PluginsConfig::default();
    plugins_config
        .list
        .insert("agena.tools".to_owned(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.tools".parse().expect("valid tool-api plugin key"),
            agena_bundled_plugins::tool::new_tool_api_plugin(),
        )],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build tool-api plugin host");
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins,
        None,
        None,
        None,
    );
    (executor, workspace_root)
}

fn plugins_invocation(
    function: agena_domain::ToolApiFunction,
    name: &str,
    arguments: serde_json::Value,
    input: serde_json::Value,
) -> agena_domain::ToolInvocation {
    agena_domain::ToolInvocation {
        tool_api_call: Some(agena_domain::ToolApiCall {
            function,
            arguments: StructuredObject::try_from(arguments).expect("valid plugins envelope"),
        }),
        name: name.to_owned(),
        plugin_name: None,
        input: StructuredObject::try_from(input).expect("valid plugins input"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plugins_search_and_tags_resolve_through_the_real_tool_api_plugin() {
    let (executor, workspace_root) = build_tool_api_executor().await;
    for (name, function, arguments, input) in [
        (
            "plugins_search",
            agena_domain::ToolApiFunction::PluginsSearch,
            serde_json::json!({ "query": "context status" }),
            serde_json::json!({ "query": "context status" }),
        ),
        (
            "plugins_tags",
            agena_domain::ToolApiFunction::PluginsTags,
            serde_json::json!({}),
            serde_json::json!({}),
        ),
    ] {
        let invocation = plugins_invocation(function, name, arguments, input);
        let prepared = executor
            .prepare_invocation(&invocation, 7, 52)
            .await
            .unwrap_or_else(|error| panic!("prepare {name}: {error}"));
        let (prepared_invocation, prepared_shell) = executor
            .prepare_shell_invocation(&prepared.invocation, 7, 52)
            .await
            .unwrap_or_else(|error| panic!("prepare shell {name}: {error}"));
        let execution = executor
            .execute_invocation_detailed_with_prepared_shell(
                &prepared_invocation,
                7,
                52,
                prepared_shell,
            )
            .await
            .unwrap_or_else(|error| panic!("execute {name}: {error}"));
        assert!(
            !execution.view.output_text.is_empty(),
            "{name} produced empty output: {execution:?}"
        );
        tracing::info!(
            tool = name,
            output = %execution.view.output_text,
            "plugins family tool resolved"
        );
    }
    let _ = std::fs::remove_dir_all(workspace_root);
}

/// The plugin list must be readable even when no app-provided HostClient was
/// installed — the in-proc host installs its own HostHandle-backed client, and
/// `list_plugins` reads the shared plugin registry synchronously.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tool_api_host_callback_has_an_installed_client() {
    let (executor, workspace_root) = build_tool_api_executor().await;
    let invocation = plugins_invocation(
        agena_domain::ToolApiFunction::PluginsList,
        "plugins_list",
        serde_json::json!({}),
        serde_json::json!({}),
    );
    let prepared = executor
        .prepare_invocation(&invocation, 7, 53)
        .await
        .expect("prepare plugins_list");
    let (prepared_invocation, prepared_shell) = executor
        .prepare_shell_invocation(&prepared.invocation, 7, 53)
        .await
        .expect("prepare shell plugins_list");
    let execution = executor
        .execute_invocation_detailed_with_prepared_shell(
            &prepared_invocation,
            7,
            53,
            prepared_shell,
        )
        .await
        .expect("execute plugins_list");
    assert!(execution.view.output_text.contains("agena.tools"));
    let _ = std::fs::remove_dir_all(workspace_root);
}

#[allow(dead_code)]
fn _arc_used() -> Arc<()> {
    Arc::new(())
}
