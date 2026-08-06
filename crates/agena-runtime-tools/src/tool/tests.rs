use std::{collections::HashMap, process::Command, sync::Arc};

use super::{
    StructuredObject, TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES, ToolError, ToolExecutor,
    ToolInvocation, ToolOutput, ToolPayloadOutput, bounded_model_output_preview,
    canonicalize_path_for_execution, compact_tool_output_payload_for_model, line_count,
};
use crate::{
    authorization::ExecutionPrincipal,
    message::{EnterSnapshotToolInput, ExitSnapshotToolInput},
    permission::{PermissionPolicy, ToolPermissionPolicy},
    snapshot_registry,
};
use agena_domain::PermissionMode;
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
    ToolPresentationConfig,
};
use agena_tool::SnapshotBackend;

#[derive(Default)]
struct ChokePointPlugin;

#[derive(Default)]
struct ToolApiFixture;

#[derive(Default)]
struct ExecutionAccessFixture;

#[derive(Default)]
struct ExecutorBackedShellAdapter;

#[derive(Default)]
struct ExecutorBackedFsAdapter;

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn empty_test_plugin_host(workspace_root: &std::path::Path) -> Arc<PluginHost> {
    PluginHost::new(PluginHostBuildConfig {
        static_plugins: Vec::new(),
        config: PluginsConfig::default(),
        workspace_root: workspace_root.to_path_buf(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build empty plugin host")
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "shell",
    version = "test",
    summary = "Definition-only shell adapter regression fixture."
)]
impl ExecutorBackedShellAdapter {
    #[tool(name = "run", summary = "Run a shell command.", mutating, shell)]
    async fn run(&self, _input: &crate::message::ShellCommandInput) -> String {
        "plugin adapter must not execute".to_owned()
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "fs",
    version = "test",
    summary = "Definition-only filesystem adapter regression fixture."
)]
impl ExecutorBackedFsAdapter {
    #[tool(name = "read", summary = "Read a file.", read_only)]
    async fn read(&self, _input: &crate::message::ReadToolInput) -> String {
        "plugin adapter must not execute".to_owned()
    }

    #[tool(name = "grep", summary = "Search file contents with regex.", read_only)]
    async fn grep(&self, _input: &crate::message::GrepToolInput) -> String {
        "plugin adapter must not execute".to_owned()
    }
}

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

    fn capability_denied_tool_names(&self) -> &std::collections::BTreeSet<String> {
        static EMPTY: std::sync::OnceLock<std::collections::BTreeSet<String>> =
            std::sync::OnceLock::new();
        EMPTY.get_or_init(Default::default)
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
async fn compact_builtin_targets_execute_through_the_orchestrator() {
    let workspace_root = std::env::temp_dir().join(format!(
        "agena-direct-builtin-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&workspace_root).expect("create direct builtin workspace");
    std::fs::write(workspace_root.join("fixture.txt"), "direct fs dispatch\n")
        .expect("write direct read fixture");

    let mut plugins_config = PluginsConfig::default();
    plugins_config
        .list
        .insert("agena.shell".to_owned(), ConfiguredPlugin::static_default());
    plugins_config
        .list
        .insert("agena.fs".to_owned(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.shell".parse().expect("valid shell plugin key"),
                ExecutorBackedShellAdapter,
            ),
            StaticPluginRegistration::new(
                "agena.fs".parse().expect("valid fs plugin key"),
                ExecutorBackedFsAdapter,
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
    .expect("build direct builtin plugin host");
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
        ToolPresentationConfig::default(),
    );

    let shell_input = StructuredObject::try_from(serde_json::json!({
        "shell": "bash",
        "command": "python3 --version",
        "description": "Check Python version",
        "reads": ["/usr/bin"],
        "writes": [],
        "network": []
    }))
    .expect("valid shell input");
    let shell_invocation = ToolInvocation {
        tool_api_call: Some(agena_domain::ToolApiCall {
            function: agena_domain::ToolApiFunction::Call,
            arguments: StructuredObject::try_from(serde_json::json!({
                "tool": "shell.run",
                "input": serde_json::Value::from(shell_input.clone())
            }))
            .expect("valid tools_call envelope"),
        }),
        name: "shell.run".to_owned(),
        plugin_name: None,
        input: shell_input,
    };
    let prepared = executor
        .prepare_invocation(&shell_invocation, 1, 1)
        .expect("prepare compact shell invocation");
    let (prepared_shell_invocation, prepared_shell) = executor
        .prepare_shell_invocation(&prepared.invocation, 1, 1)
        .expect("prepare compact shell command");
    let shell_execution = executor
        .execute_invocation_detailed_with_prepared_shell(
            &prepared_shell_invocation,
            1,
            1,
            prepared_shell,
        )
        .expect("execute compact shell target");
    assert!(shell_execution.view.output_text.contains("Python"));
    assert!(
        !shell_execution
            .view
            .output_text
            .contains("plugin adapter must not execute")
    );

    let read_invocation = ToolInvocation::new(
        "fs.read",
        StructuredObject::try_from(serde_json::json!({"path": "fixture.txt"}))
            .expect("valid read input"),
    );
    let prepared_read = executor
        .prepare_invocation(&read_invocation, 1, 2)
        .expect("prepare compact read invocation");
    let read_execution = executor
        .execute_invocation_detailed(&prepared_read.invocation, 1, 2)
        .expect("execute compact read target");
    assert!(
        read_execution
            .view
            .output_text
            .contains("direct fs dispatch")
    );
    assert!(
        !read_execution
            .view
            .output_text
            .contains("plugin adapter must not execute")
    );

    std::fs::remove_dir_all(workspace_root).expect("remove direct builtin workspace");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grep_targets_a_single_file_or_a_directory() {
    let workspace_root =
        std::env::temp_dir().join(format!("agena-grep-test-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(workspace_root.join("nested")).expect("create grep workspace");
    std::fs::write(
        workspace_root.join("fixture.txt"),
        "alpha\nbeta\nalpha again\n",
    )
    .expect("write grep fixture");
    std::fs::write(workspace_root.join("nested/other.txt"), "alpha only\n")
        .expect("write nested grep fixture");

    let mut plugins_config = PluginsConfig::default();
    plugins_config
        .list
        .insert("agena.fs".to_owned(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.fs".parse().expect("valid fs plugin key"),
            ExecutorBackedFsAdapter,
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
    .expect("build grep plugin host");
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
        ToolPresentationConfig::default(),
    );

    let file_invocation = ToolInvocation::new(
        "fs.grep",
        StructuredObject::try_from(serde_json::json!({
            "pattern": "alpha",
            "path": "fixture.txt"
        }))
        .expect("valid grep file input"),
    );
    let prepared_file = executor
        .prepare_invocation(&file_invocation, 1, 1)
        .expect("prepare grep file");
    let file_execution = executor
        .execute_invocation_detailed(&prepared_file.invocation, 1, 1)
        .expect("execute grep file");
    assert!(
        file_execution
            .view
            .output_text
            .contains("fixture.txt:1: alpha")
    );
    assert!(
        file_execution
            .view
            .output_text
            .contains("fixture.txt:3: alpha again")
    );
    assert!(!file_execution.view.output_text.contains("nested/other.txt"));
    assert!(
        !file_execution
            .view
            .output_text
            .contains("plugin adapter must not execute")
    );

    let dir_invocation = ToolInvocation::new(
        "fs.grep",
        StructuredObject::try_from(serde_json::json!({
            "pattern": "alpha",
            "path": "nested"
        }))
        .expect("valid grep dir input"),
    );
    let prepared_dir = executor
        .prepare_invocation(&dir_invocation, 1, 2)
        .expect("prepare grep dir");
    let dir_execution = executor
        .execute_invocation_detailed(&prepared_dir.invocation, 1, 2)
        .expect("execute grep dir");
    assert!(
        dir_execution
            .view
            .output_text
            .contains("nested/other.txt:1: alpha only")
    );
    assert!(!dir_execution.view.output_text.contains("fixture.txt"));

    std::fs::remove_dir_all(workspace_root).expect("remove grep workspace");
}

#[tokio::test]
async fn snapshot_internal_dispatch_does_not_depend_on_public_tool_registration() {
    let fixture_root = std::env::temp_dir().join(format!(
        "agena-snapshot-internal-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let workspace_root = fixture_root.join("workspace");
    let snapshot_path = fixture_root.join("snapshot");
    std::fs::create_dir_all(&workspace_root).expect("create snapshot fixture workspace");
    run_git(&workspace_root, &["init"]);
    run_git(
        &workspace_root,
        &["config", "user.email", "agena@example.invalid"],
    );
    run_git(&workspace_root, &["config", "user.name", "Agena Test"]);
    std::fs::write(workspace_root.join("fixture.txt"), "snapshot fixture\n")
        .expect("write snapshot fixture");
    run_git(&workspace_root, &["add", "fixture.txt"]);
    run_git(&workspace_root, &["commit", "-m", "initial"]);
    run_git(
        &workspace_root,
        &[
            "worktree",
            "add",
            "-b",
            "agena/internal-dispatch-test",
            snapshot_path.to_string_lossy().as_ref(),
        ],
    );

    let registry = snapshot_registry();
    let plugins = empty_test_plugin_host(&workspace_root).await;
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins,
        Some(Arc::clone(&registry)),
        None,
        None,
        ToolPresentationConfig::default(),
    );

    let entered = executor
        .enter_snapshot_internal(
            &EnterSnapshotToolInput {
                name: None,
                path: Some(snapshot_path.to_string_lossy().into_owned()),
            },
            77,
        )
        .expect("typed internal snapshot enter");
    assert!(matches!(
        entered.output,
        ToolPayloadOutput::EnterSnapshot {
            ref path,
            ref branch,
            backend: Some(ref backend),
            ..
        } if path == snapshot_path.to_string_lossy().as_ref()
            && branch == "agena/internal-dispatch-test"
            && backend == "git"
    ));
    assert_eq!(
        registry.read().get(&77).map(|session| session.backend),
        Some(SnapshotBackend::Git)
    );

    let exited = executor
        .exit_snapshot_internal(
            &ExitSnapshotToolInput {
                action: "keep".to_owned(),
                discard_changes: false,
            },
            77,
        )
        .expect("typed internal snapshot exit");
    assert!(matches!(
        exited.output,
        ToolPayloadOutput::ExitSnapshot { ref action, ref path }
            if action == "keep" && path == snapshot_path.to_string_lossy().as_ref()
    ));
    assert!(!registry.read().contains_key(&77));

    std::fs::remove_dir_all(&fixture_root).expect("remove snapshot fixture");
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
    let search_description = bindings
        .iter()
        .find(|binding| binding.function() == agena_domain::ToolApiFunction::Search)
        .expect("tools_search binding")
        .definition()
        .description;
    assert!(search_description.contains("search instead of choosing a suggestion"));
    let call_description = bindings
        .iter()
        .find(|binding| binding.function() == agena_domain::ToolApiFunction::Call)
        .expect("tools_call binding")
        .definition()
        .description;
    assert!(call_description.contains("Never invent the tool name"));
    assert!(call_description.contains("complete argument object"));
    assert!(call_description.contains("never a Tool API function name"));
    for binding in bindings
        .iter()
        .filter(|binding| binding.function() != agena_domain::ToolApiFunction::Call)
    {
        let mut invocation =
            ToolInvocation::new(binding.function_name(), StructuredObject::default());
        invocation.tool_api_call = Some(agena_domain::ToolApiCall {
            function: binding.function(),
            arguments: invocation.input.clone(),
        });
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
    let invocation = ToolInvocation {
        tool_api_call: Some(agena_domain::ToolApiCall {
            function: agena_domain::ToolApiFunction::Call,
            arguments: StructuredObject::try_from(serde_json::json!({
                "tool": execution_tool.canonical_name(),
                "input": {}
            }))
            .expect("tools_call provider envelope"),
        }),
        name: execution_tool.canonical_name().to_string(),
        plugin_name: None,
        input: StructuredObject::default(),
    };
    let checks = executor
        .collect_permission_checks_for_invocation(&invocation)
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
        .collect_permission_checks_for_invocation_in_session(
            &ToolInvocation::new("test.access.mutate", StructuredObject::default()),
            Some(1),
        )
        .expect_err("read-only access must reject a mutating live tool");
    assert!(
        matches!(&error, ToolError::CapabilityUnavailable(_)),
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

#[tokio::test]
async fn gateway_tools_call_without_a_target_is_rejected_as_invalid_input() {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    plugins_config.list.insert(
        "agena.tools".to_string(),
        ConfiguredPlugin::static_default(),
    );
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.tools".parse().expect("valid Tool API plugin key"),
            ToolApiFixture,
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

    // A `tools_call` that still names the gateway function itself (no `tool`
    // target was ever resolved) must be rejected as invalid input — never
    // dispatched as a fabricated execution-tool name.
    for arguments in [
        serde_json::json!({}),
        serde_json::json!({"input": {}}),
        serde_json::json!({"tool": ""}),
    ] {
        let invocation = ToolInvocation {
            tool_api_call: Some(agena_domain::ToolApiCall {
                function: agena_domain::ToolApiFunction::Call,
                arguments: StructuredObject::try_from(arguments)
                    .expect("tools_call provider envelope"),
            }),
            name: "tools_call".to_owned(),
            plugin_name: None,
            input: StructuredObject::default(),
        };

        let error = executor
            .prepare_invocation(&invocation, 1, 1)
            .expect_err("gateway without a target must fail before execution");
        assert!(
            matches!(error, ToolError::InvalidInput { .. }),
            "expected invalid-input rejection, got: {error}"
        );
        assert_eq!(error.field_issues().len(), 1);
        assert_eq!(error.field_issues()[0].field(), "tool");
    }
}

#[tokio::test]
async fn gateway_tools_call_surfaces_the_arguments_shape_diagnostic() {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    plugins_config.list.insert(
        "agena.tools".to_string(),
        ConfiguredPlugin::static_default(),
    );
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.tools".parse().expect("valid Tool API plugin key"),
            ToolApiFixture,
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

    // A diagnostic stamped by the session processor (string-encoded or
    // malformed arguments) must be surfaced verbatim instead of the
    // generic missing-`tool` message, so the model learns the real shape
    // error rather than blindly re-adding a `tool` field.
    let diagnostic = "tools_call arguments did not parse as valid JSON (serde json error: \
                      invalid escape at line 1 column 99). Arguments must be one JSON object \
                      with a string `tool` field and an `input` object.";
    let invocation = ToolInvocation {
        tool_api_call: Some(agena_domain::ToolApiCall {
            function: agena_domain::ToolApiFunction::Call,
            arguments: StructuredObject::try_from(serde_json::json!({
                agena_domain::TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD: diagnostic,
            }))
            .expect("diagnostic envelope"),
        }),
        name: "tools_call".to_owned(),
        plugin_name: None,
        input: StructuredObject::default(),
    };

    let error = executor
        .prepare_invocation(&invocation, 1, 1)
        .expect_err("gateway with a shape diagnostic must fail before execution");
    assert!(
        matches!(error, ToolError::InvalidInput { .. }),
        "expected invalid-input rejection, got: {error}"
    );
    assert_eq!(
        error.actionable_message().as_deref(),
        Some(diagnostic),
        "shape diagnostic must replace the generic message"
    );
}
