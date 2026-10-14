//! End-to-end harness for the `agena.tools` discovery family through the real
//! Tool API plugin: `plugins_list` (paged plugin inventory) -> `tools_list`
//! (paged per-plugin tool inventory) -> `tools_help` -> `tools_tags`.
//!
//! There is no search function: capability discovery walks the plugin
//! inventory and then the owning plugin's tool inventory, page by page.
//!
//! This drives the *exact* production dispatch route for these tools:
//! ToolApiPlugin (`agena.tools`) as an in-proc static plugin, resolved
//! through the ToolExecutor's non-payload path (the plugins_* family is not
//! in `ToolPayloadInput::from_executor_backed_invocation`'s builtin set) into
//! `invoke_tool_cancellable` -> in-proc transport dispatch -> the plugin's
//! `list_plugins`/`list_tools` host callbacks.

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
async fn plugins_list_reports_tool_counts_instead_of_tool_names() {
    let (executor, workspace_root) = build_tool_api_executor().await;
    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::PluginsList,
        "plugins_list",
        serde_json::json!({ "plugin": ["agena.tools", "agena.monitor"] }),
        52,
    )
    .await;
    let text = execution.view.output_text.as_str();
    assert!(text.contains("Available plugins: returned"), "{text}");
    assert!(text.contains("agena.tools"), "{text}");
    assert!(text.contains("agena.monitor"), "{text}");
    assert!(text.contains("tool(s)"), "{text}");
    assert!(
        !text.contains("monitor.start") && !text.contains("monitor.stop"),
        "plugins_list must report a tool count, not inline tool names: {text}"
    );
    assert!(
        text.contains("List a plugin's tools with `tools_list`"),
        "the plugin page must point at the per-plugin tool listing: {text}"
    );
    let _ = std::fs::remove_dir_all(workspace_root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plugins_list_pagination_continues_the_plugin_inventory() {
    let (executor, workspace_root) = build_tool_api_executor().await;
    let first = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::PluginsList,
        "plugins_list",
        serde_json::json!({ "offset": 0, "limit": 1 }),
        53,
    )
    .await;
    let first_text = first.view.output_text.as_str();
    assert!(first_text.contains("returned 1 of "), "{first_text}");
    assert!(
        first_text.contains("More available: yes. Continue with `plugins_list` using `offset: 1`."),
        "a partial plugin page must say how to continue: {first_text}"
    );

    let second = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::PluginsList,
        "plugins_list",
        serde_json::json!({ "offset": 1, "limit": 1 }),
        54,
    )
    .await;
    let second_text = second.view.output_text.as_str();
    assert!(
        second_text.contains("starting at offset 1."),
        "{second_text}"
    );
    assert!(second_text.contains("More available: no."), "{second_text}");
    let _ = std::fs::remove_dir_all(workspace_root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tools_list_pages_one_plugin_tool_inventory() {
    let (executor, workspace_root) = build_tool_api_executor().await;
    let complete = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::List,
        "tools_list",
        serde_json::json!({ "plugin": ["monitor", "agena.missing"], "limit": 1_000 }),
        55,
    )
    .await;
    let complete_text = complete.view.output_text.as_str();
    assert!(complete_text.contains("monitor.start"), "{complete_text}");
    assert!(complete_text.contains("monitor.stop"), "{complete_text}");
    assert!(complete_text.contains("(agena.monitor)"), "{complete_text}");
    assert!(
        complete_text.contains("More available: no."),
        "{complete_text}"
    );

    let page = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::List,
        "tools_list",
        serde_json::json!({ "plugin": ["monitor"], "offset": 0, "limit": 1 }),
        56,
    )
    .await;
    let page_text = page.view.output_text.as_str();
    assert!(page_text.contains("returned 1 of 2"), "{page_text}");
    assert!(
        page_text.contains("Continue with `tools_list` using `offset: 1`."),
        "a partial tool page must say how to continue: {page_text}"
    );
    let _ = std::fs::remove_dir_all(workspace_root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tools_help_and_tags_accept_batches_through_the_real_gateway() {
    let (executor, workspace_root) = build_tool_api_executor().await;

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Help,
        "tools_help",
        serde_json::json!({ "tool": ["monitor.start", "monitor.stop"] }),
        57,
    )
    .await;
    assert!(execution.view.output_text.contains("Tool: monitor.start"));
    assert!(execution.view.output_text.contains("Tool: monitor.stop"));
    assert!(execution.view.output_text.contains("\n\n---\n\n"));

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Tags,
        "tools_tags",
        serde_json::json!({ "plugin": ["monitor", "agena.missing"] }),
        58,
    )
    .await;
    assert!(execution.view.output_text.contains("Available tool tags"));

    let execution = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::PluginsTags,
        "plugins_tags",
        serde_json::json!({ "plugin": ["agena.tools", "monitor"] }),
        59,
    )
    .await;
    assert!(execution.view.output_text.contains("Available plugin tags"));

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
        .prepare_invocation(&invocation, 7, 60)
        .await
        .expect("prepare plugins_list");
    let (prepared_invocation, prepared_shell) = executor
        .prepare_shell_invocation(&prepared.invocation, 7, 60)
        .await
        .expect("prepare shell plugins_list");
    let execution = executor
        .execute_invocation_detailed_with_prepared_shell(
            &prepared_invocation,
            7,
            60,
            prepared_shell,
        )
        .await
        .expect("execute plugins_list");
    assert!(execution.view.output_text.contains("agena.tools"));
    let _ = std::fs::remove_dir_all(workspace_root);
}

#[tokio::test]
async fn mixed_batch_help_retains_valid_contracts_when_one_name_is_missing() {
    let (executor, root) = build_tool_api_executor().await;
    let output = execute_gateway(
        &executor,
        agena_domain::ToolApiFunction::Help,
        "tools_help",
        serde_json::json!({"tool":["monitor.start","missing.nonexistent"]}),
        101,
    )
    .await;
    assert!(output.view.output_text.contains("Tool: monitor.start"));
    assert!(
        output
            .view
            .output_text
            .contains("Tool: missing.nonexistent")
    );
    assert!(output.view.output_text.contains("Help unavailable"));
    std::fs::remove_dir_all(root).unwrap();
}
