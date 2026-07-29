use std::{collections::HashMap, sync::Arc};

use super::{
    StructuredObject, TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES, ToolError, ToolExecutor,
    ToolInvocation, ToolOutput, bounded_model_output_preview, canonicalize_path_for_execution,
    compact_tool_output_payload_for_model, line_count,
};
use crate::{
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
};
use agena_domain::PermissionMode;
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
    ToolPresentationConfig,
};

#[derive(Default)]
struct ChokePointPlugin;

#[derive(Default)]
struct ToolApiFixture;

#[derive(Default)]
struct ExecutionAccessFixture;

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "test",
    name = "access",
    version = "0.1.0",
    summary = "Execution access regression fixture."
)]
impl ExecutionAccessFixture {
    #[tool(name = "inspect", summary = "Inspect state.", read_only)]
    async fn inspect(&self) -> String {
        "inspected".to_owned()
    }

    #[tool(name = "mutate", summary = "Mutate state.", mutating)]
    async fn mutate(&self) -> String {
        "mutated".to_owned()
    }
}

struct TestSessionContext {
    access: agena_domain::ExecutionAccess,
}

impl crate::ToolSessionContext for TestSessionContext {
    fn effective_workspace_root(&self) -> Option<&std::path::Path> {
        None
    }

    fn effective_permission(&self) -> &crate::authorization::PermissionConfig {
        static EMPTY: std::sync::OnceLock<crate::authorization::PermissionConfig> =
            std::sync::OnceLock::new();
        EMPTY.get_or_init(crate::authorization::PermissionConfig::default)
    }

    fn permission_ceiling(&self) -> &crate::authorization::PermissionConfig {
        self.effective_permission()
    }

    fn execution_access(&self) -> agena_domain::ExecutionAccess {
        self.access
    }

    fn selected_model(&self) -> Option<&str> {
        None
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "tools",
    version = "test",
    summary = "Tool API registry fixture."
)]
impl ToolApiFixture {
    #[tool(name = "list", summary = "List tools.")]
    async fn list(&self, _input: &crate::message::AskUserToolInput) -> String {
        String::new()
    }

    #[tool(name = "search", summary = "Search tools.")]
    async fn search(&self, _input: &crate::message::AskUserToolInput) -> String {
        String::new()
    }

    #[tool(name = "help", summary = "Describe a tool.")]
    async fn help(&self, _input: &crate::message::AskUserToolInput) -> String {
        String::new()
    }

    #[tool(name = "tags", summary = "List tool tags.")]
    async fn tags(&self, _input: &crate::message::AskUserToolInput) -> String {
        String::new()
    }

    #[tool(name = "call", summary = "Call a tool.")]
    async fn call(&self, _input: &crate::message::AskUserToolInput) -> String {
        String::new()
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "test",
    name = "choke",
    version = "0.1.0",
    summary = "Permission choke-point regression fixture."
)]
impl ChokePointPlugin {
    #[tool(name = "run", summary = "Run the regression fixture.")]
    async fn run(&self) -> String {
        "should not execute when denied".to_string()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detailed_execution_enforces_permissions_without_caller_preflight() {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    plugins_config
        .list
        .insert("test.choke".to_string(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "test.choke".parse().expect("valid test plugin key"),
            ChokePointPlugin,
        )],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_string(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build test plugin host");
    let executor = ToolExecutor::new(
        workspace_root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::new(PermissionMode::Ask),
        ),
        Arc::clone(&plugins),
        None,
        None,
        None,
        ToolPresentationConfig::default(),
    );
    let tool_name = plugins
        .registered_tools()
        .into_iter()
        .next()
        .expect("test plugin registers one tool")
        .canonical_name()
        .to_string();
    let invocation = ToolInvocation::new(tool_name, StructuredObject::default());

    let error = executor
        .execute_invocation_detailed(&invocation, 1, 1)
        .expect_err("the final execution boundary must reject unapproved tools");

    assert!(
        matches!(error, ToolError::PermissionAsk(_)),
        "expected final-boundary permission prompt, got: {error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_five_gateway_functions_are_provider_visible() {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    plugins_config.list.insert(
        "agena.tools".to_string(),
        ConfiguredPlugin::static_default(),
    );
    plugins_config
        .list
        .insert("test.choke".to_string(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.tools".parse().expect("valid Tool API plugin key"),
                ToolApiFixture,
            ),
            StaticPluginRegistration::new(
                "test.choke".parse().expect("valid test plugin key"),
                ChokePointPlugin,
            ),
        ],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_string(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build Tool API test plugin host");
    let executor = ToolExecutor::new(
        workspace_root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::new(PermissionMode::Ask),
        ),
        Arc::clone(&plugins),
        None,
        None,
        None,
        ToolPresentationConfig::default(),
    );

    let bindings = executor.available_tool_api_bindings();
    assert_eq!(bindings.len(), 5);
    assert!(
        bindings
            .iter()
            .all(|binding| binding.execution_tool_name().is_none())
    );
    for binding in bindings {
        let mut invocation =
            ToolInvocation::new(binding.function_name(), StructuredObject::default());
        invocation.tool_api_function = Some(binding.function());
        assert!(
            executor
                .collect_permission_checks_for_invocation(&invocation)
                .expect("collect Tool API permission checks")
                .is_empty(),
            "provider function {} must not enter the permission system",
            binding.function_name()
        );
    }

    let execution_tool = plugins
        .lookup_tool("test.choke.run")
        .expect("execution tool is registered");
    let checks = executor
        .collect_permission_checks_for_invocation(&ToolInvocation::new(
            execution_tool.canonical_name(),
            StructuredObject::default(),
        ))
        .expect("collect execution-tool permission checks");
    assert_eq!(checks.len(), 1);
    assert!(matches!(
        checks[0].decision,
        agena_domain::PermissionDecision::Ask { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_only_access_filters_live_tools_and_preserves_gateway_discovery() {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    plugins_config
        .list
        .insert("agena.tools".to_owned(), ConfiguredPlugin::static_default());
    plugins_config
        .list
        .insert("test.access".to_owned(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.tools".parse().expect("valid Tool API plugin key"),
                ToolApiFixture,
            ),
            StaticPluginRegistration::new(
                "test.access".parse().expect("valid access plugin key"),
                ExecutionAccessFixture,
            ),
        ],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build access test plugin host");
    let executor = ToolExecutor::new(
        workspace_root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins,
        None,
        None,
        None,
        ToolPresentationConfig::default(),
    )
    .for_session_context(&TestSessionContext {
        access: agena_domain::ExecutionAccess::ReadOnly,
    });

    let execution_tools = executor
        .available_execution_tools()
        .into_iter()
        .map(|tool| tool.canonical_name())
        .collect::<Vec<_>>();
    assert_eq!(execution_tools, ["test.access.inspect"]);
    assert_eq!(executor.available_tool_api_bindings().len(), 5);
    assert_eq!(
        executor
            .principal()
            .authorize_tool_name("test.access.mutate"),
        agena_domain::PermissionDecision::Allow,
        "capability filtering must not rewrite the independent permission policy"
    );

    let error = executor
        .execute_invocation_detailed(
            &ToolInvocation::new("test.access.mutate", StructuredObject::default()),
            1,
            1,
        )
        .expect_err("read-only access must reject a mutating live tool");
    assert!(
        matches!(
            &error,
            ToolError::UnknownTool { .. } | ToolError::UnknownToolHint { .. }
        ),
        "out-of-capability tools must be hidden at invocation time, got {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn canonical_path_resolution_follows_existing_symlink_parents() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("agena-path-test-{}", uuid::Uuid::new_v4()));
    let external = root.join("external");
    std::fs::create_dir_all(&external).expect("create test directories");
    symlink(&external, root.join("workspace-link")).expect("create test symlink");

    let resolved = canonicalize_path_for_execution(&root.join("workspace-link/new.txt"));
    let canonical_external = external
        .canonicalize()
        .expect("canonicalize expected symlink target");
    assert_eq!(resolved, canonical_external.join("new.txt"));

    std::fs::remove_dir_all(root).expect("remove test directories");
}

#[test]
fn bounded_model_output_preview_keeps_the_head_and_tail() {
    let output = (0..20)
        .map(|index| format!("line-{index:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let preview = bounded_model_output_preview(&output, "[full output: saved.txt]", 9, 120);

    assert!(preview.contains("line-00"));
    assert!(preview.contains("line-19"));
    assert!(preview.contains("[full output: saved.txt]"));
    assert!(line_count(&preview) <= 9);
    assert!(preview.len() <= 120);
}

#[test]
fn compact_tool_payload_preserves_patch_changes_without_the_full_diff() {
    let payload = serde_json::json!({
        "changes": [{ "path": "src/lib.rs", "kind": "updated" }],
        "diff": "x".repeat(TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES * 2),
    });
    let mut output = ToolOutput::from_json_payload(Some(&payload)).expect("tool output");

    compact_tool_output_payload_for_model(&mut output, ".agena/tool-results/full.txt", 50_000)
        .expect("compact payload");

    let compacted = output.to_json_payload().expect("compacted payload");
    let serialized = serde_json::to_string(&compacted).expect("serialize compact payload");
    assert!(serialized.len() <= TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES);
    assert_eq!(
        compacted
            .pointer("/changes/0/path")
            .and_then(serde_json::Value::as_str),
        Some("src/lib.rs")
    );
    assert!(
        compacted
            .pointer("/diff")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|diff| diff.contains("full output persisted"))
    );
}
