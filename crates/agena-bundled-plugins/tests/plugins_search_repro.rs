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
use agena_plugin_host::sdk::host_api::{EventSubscription, HostClient, LogLevel, ToolDescriptor};
use agena_plugin_host::sdk::{EventEnvelope, EventFilter, Result as SdkResult, ToolInvokeOutput};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_runtime_tools::{
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    tool::ToolExecutor,
};

struct CatalogHostClient;

#[async_trait::async_trait]
impl HostClient for CatalogHostClient {
    async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

    async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
        Ok(())
    }

    async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
        Ok(EventSubscription {
            id: "unused".to_owned(),
        })
    }

    async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }

    async fn invoke_tool(
        &self,
        _tool: String,
        _input: serde_json::Value,
    ) -> SdkResult<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("unused"))
    }

    async fn list_tools(&self) -> SdkResult<Vec<ToolDescriptor>> {
        Ok(vec![
            ToolDescriptor {
                name: "monitor.start".to_owned(),
                plugin_id: Some("agena.monitor".to_owned()),
                summary: Some("Start a continuous background monitor.".to_owned()),
                help: Some("Start monitoring a command or WebSocket endpoint.".to_owned()),
                examples: Vec::new(),
                input_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": { "command": { "type": "string" } }
                })),
            },
            ToolDescriptor {
                name: "monitor.stop".to_owned(),
                plugin_id: Some("agena.monitor".to_owned()),
                summary: Some("Stop one background monitor.".to_owned()),
                help: None,
                examples: Vec::new(),
                input_schema: Some(serde_json::json!({
                    "type": "object",
                    "required": ["monitor_id"],
                    "properties": { "monitor_id": { "type": "string" } }
                })),
            },
        ])
    }
}

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
    plugins_config.list.insert(
        "agena.monitor".to_owned(),
        ConfiguredPlugin::static_default(),
    );
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.tools".parse().expect("valid tool-api plugin key"),
                agena_bundled_plugins::tool::new_tool_api_plugin(),
            ),
            StaticPluginRegistration::new(
                "agena.monitor".parse().expect("valid monitor plugin key"),
                agena_bundled_plugins::tool::new_monitor_plugin(),
            ),
        ],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: Some(Arc::new(CatalogHostClient)),
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

async fn execute_gateway(
    executor: &ToolExecutor,
    function: agena_domain::ToolApiFunction,
    name: &str,
    input: serde_json::Value,
    call_id: i64,
) -> agena_runtime_tools::tool::ToolInvocationExecution {
    let invocation = plugins_invocation(function, name, input.clone(), input);
    let prepared = executor
        .prepare_invocation(&invocation, 7, call_id)
        .await
        .unwrap_or_else(|error| panic!("prepare {name}: {error}"));
    let (prepared_invocation, prepared_shell) = executor
        .prepare_shell_invocation(&prepared.invocation, 7, call_id)
        .await
        .unwrap_or_else(|error| panic!("prepare shell for {name}: {error}"));
    executor
        .execute_invocation_detailed_with_prepared_shell(
            &prepared_invocation,
            7,
            call_id,
            prepared_shell,
        )
        .await
        .unwrap_or_else(|error| panic!("execute {name}: {error}"))
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tools_search_and_help_accept_batches_through_the_real_gateway() {
    let (executor, workspace_root) = build_tool_api_executor().await;

    let search_input = serde_json::json!({
        "query": ["continuous monitor", "stop background monitor"],
        "plugin": ["agena.monitor", "monitor"],
        "limit": 1_000
    });
    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Search,
        "tools_search",
        search_input,
        54,
    )
    .await;
    assert!(
        execution
            .view
            .output_text
            .contains("Matching tools for \"continuous monitor\"")
    );
    assert!(
        execution
            .view
            .output_text
            .contains("Matching tools for \"stop background monitor\"")
    );
    assert!(execution.view.output_text.contains("monitor.start"));
    assert!(execution.view.output_text.contains("monitor.stop"));

    let help_input = serde_json::json!({
        "tool": ["monitor.start", "monitor.stop"]
    });
    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Help,
        "tools_help",
        help_input,
        55,
    )
    .await;
    assert!(execution.view.output_text.contains("Tool: monitor.start"));
    assert!(execution.view.output_text.contains("Tool: monitor.stop"));
    assert!(execution.view.output_text.contains("\n\n---\n\n"));

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::List,
        "tools_list",
        serde_json::json!({
            "plugin": ["monitor", "agena.missing"],
            "limit": 1_000
        }),
        56,
    )
    .await;
    assert!(execution.view.output_text.contains("monitor.start"));
    assert!(execution.view.output_text.contains("monitor.stop"));

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::PluginsList,
        "plugins_list",
        serde_json::json!({
            "plugin": ["agena.tools", "monitor"]
        }),
        57,
    )
    .await;
    assert!(execution.view.output_text.contains("agena.tools"));
    assert!(execution.view.output_text.contains("agena.monitor"));

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::PluginsSearch,
        "plugins_search",
        serde_json::json!({
            "query": ["tools", "monitor"],
            "plugin": ["agena.tools", "monitor"]
        }),
        58,
    )
    .await;
    assert!(execution.view.output_text.contains("agena.tools"));
    assert!(execution.view.output_text.contains("agena.monitor"));

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Tags,
        "tools_tags",
        serde_json::json!({
            "plugin": ["monitor", "agena.missing"]
        }),
        59,
    )
    .await;
    assert!(execution.view.output_text.contains("execute"));

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::PluginsTags,
        "plugins_tags",
        serde_json::json!({
            "plugin": ["agena.tools", "monitor"]
        }),
        60,
    )
    .await;
    assert!(execution.view.output_text.contains("Available plugin tags"));

    let _ = std::fs::remove_dir_all(workspace_root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tools_search_filters_opposite_and_unrelated_capabilities_before_pagination() {
    let (executor, workspace_root) = build_tool_api_executor().await;

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Search,
        "tools_search",
        serde_json::json!({
            "query": "stop background monitor",
            "limit": 10_000
        }),
        61,
    )
    .await;
    assert!(execution.view.output_text.contains("monitor.stop"));
    assert!(
        !execution.view.output_text.contains("monitor.start"),
        "an opposite action must not survive relevance filtering: {}",
        execution.view.output_text
    );
    assert!(
        execution.view.output_text.contains("returned 1 of 1"),
        "limit must cap relevant matches instead of filling with noise: {}",
        execution.view.output_text
    );

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Search,
        "tools_search",
        serde_json::json!({
            "query": "send customer email",
            "limit": 10_000
        }),
        62,
    )
    .await;
    assert!(
        execution.view.output_text.contains("returned 0 of 0"),
        "an unrelated query must produce an honest empty result: {}",
        execution.view.output_text
    );
    assert!(!execution.view.output_text.contains("monitor.start"));
    assert!(!execution.view.output_text.contains("monitor.stop"));

    let _ = std::fs::remove_dir_all(workspace_root);
}
