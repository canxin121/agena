use std::{
    collections::HashMap,
    process::Command,
    sync::{Arc, RwLock},
};

use super::{
    StructuredObject, ToolError, ToolExecutor, ToolInvocation, ToolPayloadOutput,
    bounded_model_output_preview, canonicalize_path_for_execution, line_count,
};
use crate::{
    authorization::ExecutionPrincipal,
    part::{EnterSnapshotToolInput, ExitSnapshotToolInput},
    permission::{PermissionPolicy, ToolPermissionPolicy},
    snapshot_registry,
};
use agena_domain::PermissionMode;
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_tool::SnapshotBackend;

#[derive(Default)]
struct ChokePointPlugin;

#[derive(Default)]
struct ToolApiFixture;

#[derive(Default)]
struct ExecutionAccessFixture;

#[derive(Debug, Clone, Copy)]
enum RenderBehavior {
    Project,
    Delegate,
    Fail,
    EmptyHuman,
    JsonOnlyHuman,
}

struct RenderingFixture {
    behavior: RenderBehavior,
}

#[agena_plugin_host::sdk::async_trait]
impl agena_plugin_host::sdk::Plugin for RenderingFixture {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        let mut manifest = agena_plugin_host::sdk::PluginManifest::new("test", "renderer", "0.1.0");
        manifest.tools = vec![scoped_dynamic_tool_definition("render")];
        manifest
    }

    async fn tool_render(
        &self,
        input: agena_plugin_host::sdk::ToolRenderInput,
    ) -> agena_plugin_host::sdk::Result<Option<agena_plugin_host::sdk::ToolRenderOutput>> {
        assert_eq!(input.tool_name, "render");
        assert_eq!(input.input, serde_json::json!({"source": "raw"}));
        assert_eq!(input.output.text, "raw result");
        match self.behavior {
            RenderBehavior::Project => Ok(Some(agena_plugin_host::sdk::ToolRenderOutput {
                model: Some("plugin model projection".to_owned()),
                human: Some(agena_plugin_host::sdk::ToolHumanPresentation {
                    title: "Plugin title".to_owned(),
                    summary: "Plugin summary".to_owned(),
                    blocks: vec![agena_domain::ViewBlock::Markdown {
                        id: None,
                        text: "plugin human projection".to_owned(),
                    }],
                }),
            })),
            RenderBehavior::Delegate => Ok(None),
            RenderBehavior::Fail => Err(agena_plugin_host::sdk::PluginError::internal(
                "renderer failed",
            )),
            RenderBehavior::EmptyHuman => Ok(Some(agena_plugin_host::sdk::ToolRenderOutput {
                model: None,
                human: Some(agena_plugin_host::sdk::ToolHumanPresentation {
                    title: "Plugin fallback title".to_owned(),
                    summary: "Plugin fallback summary".to_owned(),
                    blocks: Vec::new(),
                }),
            })),
            RenderBehavior::JsonOnlyHuman => Ok(Some(agena_plugin_host::sdk::ToolRenderOutput {
                model: None,
                human: Some(agena_plugin_host::sdk::ToolHumanPresentation {
                    title: "Plugin JSON title".to_owned(),
                    summary: "Plugin JSON summary".to_owned(),
                    blocks: vec![agena_domain::ViewBlock::Json {
                        id: Some("opaque".to_owned()),
                        value: serde_json::json!({"machine_only": true}),
                    }],
                }),
            })),
        }
    }
}

#[derive(Default)]
struct ScopedDynamicToolFixture {
    host: RwLock<Option<Arc<dyn agena_plugin_host::sdk::HostClient>>>,
}

fn scoped_dynamic_tool_definition(name: &str) -> agena_plugin_host::sdk::ToolDefinition {
    agena_plugin_host::sdk::ToolDefinition {
        name: name.to_string(),
        contract: agena_plugin_host::sdk::ToolContract {
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            ..Default::default()
        },
        model: Default::default(),
        docs: agena_plugin_host::sdk::ToolDocs {
            summary: Some(format!("Scoped fixture tool {name}.")),
            ..Default::default()
        },
        runtime: Default::default(),
        permissions: agena_plugin_host::sdk::ToolPermissionContract {
            read_only: true,
            ..Default::default()
        },
        tags: Vec::new(),
    }
}

#[agena_plugin_host::sdk::async_trait]
impl agena_plugin_host::sdk::Plugin for ScopedDynamicToolFixture {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        let mut manifest = agena_plugin_host::sdk::PluginManifest::new("test", "scoped", "0.1.0");
        manifest.summary = Some("Session scoped dynamic tool fixture.".to_string());
        manifest.hooks = agena_plugin_host::sdk::HookSubscription::INIT;
        manifest.tools = vec![scoped_dynamic_tool_definition("seed")];
        manifest
    }

    async fn init(
        &self,
        _ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn agena_plugin_host::sdk::HostClient>,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| agena_plugin_host::sdk::PluginError::internal("host lock poisoned"))? =
            Some(host);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(
        &self,
        input: agena_plugin_host::sdk::ToolInvokeInput,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        match input.tool_name.as_str() {
            "seed" => {
                let host = self
                    .host
                    .read()
                    .map_err(|_| {
                        agena_plugin_host::sdk::PluginError::internal("host lock poisoned")
                    })?
                    .clone()
                    .ok_or_else(|| {
                        agena_plugin_host::sdk::PluginError::internal("plugin not initialized")
                    })?;
                host.register_tool(agena_plugin_host::sdk::host_api::HostToolRegisterRequest {
                    tool: scoped_dynamic_tool_definition("dynamic"),
                })
                .await?;
                Ok(agena_plugin_host::sdk::ToolInvokeOutput::text("seeded"))
            }
            "dynamic" => Ok(agena_plugin_host::sdk::ToolInvokeOutput::text("dynamic")),
            other => Err(agena_plugin_host::sdk::PluginError::not_implemented(
                format!("tool_invoke({other})"),
            )),
        }
    }
}

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

async fn rendering_executor(behavior: RenderBehavior) -> ToolExecutor {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let plugin_id = "test.renderer";
    let mut config = PluginsConfig::default();
    config
        .list
        .insert(plugin_id.to_owned(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            plugin_id.parse().expect("valid renderer plugin key"),
            RenderingFixture { behavior },
        )],
        config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build renderer plugin host");
    ToolExecutor::new(
        workspace_root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins,
        None,
        None,
        None,
    )
}

fn rendering_invocation() -> ToolInvocation {
    ToolInvocation::new(
        "test.renderer.render",
        StructuredObject::try_from(serde_json::json!({"source": "raw"})).expect("structured input"),
    )
}

#[tokio::test]
async fn owning_plugin_controls_both_runtime_tool_result_projections() {
    let executor = rendering_executor(RenderBehavior::Project).await;
    let raw = agena_domain::RawOutput::text("raw result");

    let projected = executor
        .render_tool_result(&rendering_invocation(), &raw)
        .await;

    assert_eq!(projected.model.as_deref(), Some("plugin model projection"));
    let human = projected.human.expect("human projection");
    assert_eq!(human.title, "Plugin title");
    assert_eq!(human.summary, "Plugin summary");
    assert_eq!(
        human.blocks,
        vec![agena_domain::ViewBlock::Markdown {
            id: None,
            text: "plugin human projection".to_owned(),
        }]
    );
    assert_eq!(raw, agena_domain::RawOutput::text("raw result"));
}

#[tokio::test]
async fn delegated_or_failed_plugin_render_uses_runtime_fallback() {
    for behavior in [RenderBehavior::Delegate, RenderBehavior::Fail] {
        let executor = rendering_executor(behavior).await;
        let projected = executor
            .render_tool_result(
                &rendering_invocation(),
                &agena_domain::RawOutput::text("raw result"),
            )
            .await;

        assert_eq!(projected.model.as_deref(), Some("raw result"));
        let human = projected.human.expect("runtime human fallback");
        assert_eq!(human.title, "test.renderer.render");
        assert_eq!(human.summary, "raw result");
    }
}

#[tokio::test]
async fn empty_or_json_only_plugin_human_render_uses_readable_fallback() {
    for (behavior, title, summary) in [
        (
            RenderBehavior::EmptyHuman,
            "Plugin fallback title",
            "Plugin fallback summary",
        ),
        (
            RenderBehavior::JsonOnlyHuman,
            "Plugin JSON title",
            "Plugin JSON summary",
        ),
    ] {
        let executor = rendering_executor(behavior).await;
        let projected = executor
            .render_tool_result(
                &rendering_invocation(),
                &agena_domain::RawOutput {
                    text: "raw result".to_owned(),
                    payload: Some(serde_json::json!({
                        "items": [{"name": "visible", "status": "ready"}]
                    })),
                    ..Default::default()
                },
            )
            .await;

        let human = projected.human.expect("runtime human fallback");
        assert_eq!(human.title, title);
        assert_eq!(human.summary, summary);
        assert!(
            human
                .blocks
                .iter()
                .any(|block| !matches!(block, agena_domain::ViewBlock::Json { .. })),
            "fallback should expose readable blocks: {:?}",
            human.blocks
        );
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "shell",
    version = "test",
    summary = "Definition-only shell adapter regression fixture."
)]
impl ExecutorBackedShellAdapter {
    #[tool(name = "run", summary = "Run a shell command.", mutating, shell)]
    async fn run(&self, _input: &crate::part::ShellCommandInput) -> String {
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
    async fn read(&self, _input: &crate::part::ReadToolInput) -> String {
        "plugin adapter must not execute".to_owned()
    }

    #[tool(name = "grep", summary = "Search file contents with regex.", read_only)]
    async fn grep(&self, _input: &crate::part::GrepToolInput) -> String {
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
    session_id: Option<i64>,
    access: agena_domain::ExecutionAccess,
}

impl crate::ToolSessionContext for TestSessionContext {
    fn session_id(&self) -> Option<i64> {
        self.session_id
    }

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
    async fn list(&self, _input: &crate::part::AskUserToolInput) -> String {
        String::new()
    }

    #[tool(name = "search", summary = "Search tools.")]
    async fn search(&self, _input: &crate::part::AskUserToolInput) -> String {
        String::new()
    }

    #[tool(name = "help", summary = "Describe a tool.")]
    async fn help(&self, _input: &crate::part::AskUserToolInput) -> String {
        String::new()
    }

    #[tool(name = "tags", summary = "List tool tags.")]
    async fn tags(&self, _input: &crate::part::AskUserToolInput) -> String {
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
        .await
        .expect("prepare compact shell invocation");
    let (prepared_shell_invocation, prepared_shell) = executor
        .prepare_shell_invocation(&prepared.invocation, 1, 1)
        .await
        .expect("prepare compact shell command");
    let shell_execution = executor
        .execute_invocation_detailed_with_prepared_shell(
            &prepared_shell_invocation,
            1,
            1,
            prepared_shell,
        )
        .await
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
        StructuredObject::try_from(serde_json::json!({"file_path": "fixture.txt"}))
            .expect("valid read input"),
    );
    let prepared_read = executor
        .prepare_invocation(&read_invocation, 1, 2)
        .await
        .expect("prepare compact read invocation");
    let read_execution = executor
        .execute_invocation_detailed(&prepared_read.invocation, 1, 2)
        .await
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
    std::fs::create_dir_all(workspace_root.join("target")).expect("create ignored target");
    std::fs::create_dir_all(workspace_root.join(".hidden")).expect("create hidden fixture");
    std::fs::write(workspace_root.join(".gitignore"), "target/\n")
        .expect("write grep ignore rules");
    std::fs::write(
        workspace_root.join("target/generated.txt"),
        "alpha generated\n",
    )
    .expect("write ignored grep fixture");
    std::fs::write(workspace_root.join(".hidden/private.txt"), "alpha hidden\n")
        .expect("write hidden grep fixture");
    std::fs::write(
        workspace_root.join("binary.dat"),
        b"alpha before binary\0alpha after binary\n",
    )
    .expect("write binary grep fixture");
    let oversized = std::fs::File::create(workspace_root.join("oversized.txt"))
        .expect("create oversized grep fixture");
    oversized
        .set_len(33 * 1024 * 1024)
        .expect("make sparse oversized grep fixture");

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
        .await
        .expect("prepare grep file");
    let file_execution = executor
        .execute_invocation_detailed(&prepared_file.invocation, 1, 1)
        .await
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
        .await
        .expect("prepare grep dir");
    let dir_execution = executor
        .execute_invocation_detailed(&prepared_dir.invocation, 1, 2)
        .await
        .expect("execute grep dir");
    assert!(
        dir_execution
            .view
            .output_text
            .contains("nested/other.txt:1: alpha only")
    );
    assert!(!dir_execution.view.output_text.contains("fixture.txt"));

    let workspace_invocation = ToolInvocation::new(
        "fs.grep",
        StructuredObject::try_from(serde_json::json!({
            "pattern": "alpha"
        }))
        .expect("valid workspace grep input"),
    );
    let prepared_workspace = executor
        .prepare_invocation(&workspace_invocation, 1, 3)
        .await
        .expect("prepare workspace grep");
    let workspace_execution = executor
        .execute_invocation_detailed(&prepared_workspace.invocation, 1, 3)
        .await
        .expect("execute workspace grep");
    assert!(workspace_execution.view.output_text.contains("fixture.txt"));
    assert!(
        workspace_execution
            .view
            .output_text
            .contains("nested/other.txt")
    );
    assert!(
        !workspace_execution
            .view
            .output_text
            .contains("target/generated.txt")
    );
    assert!(
        !workspace_execution
            .view
            .output_text
            .contains(".hidden/private.txt")
    );
    assert!(!workspace_execution.view.output_text.contains("binary.dat"));
    assert!(
        workspace_execution
            .view
            .output_text
            .contains("1 file(s) larger than 32 MiB skipped")
    );

    let ignored_invocation = ToolInvocation::new(
        "fs.grep",
        StructuredObject::try_from(serde_json::json!({
            "pattern": "alpha",
            "include_ignored": true
        }))
        .expect("valid ignored grep input"),
    );
    let prepared_ignored = executor
        .prepare_invocation(&ignored_invocation, 1, 4)
        .await
        .expect("prepare ignored grep");
    let ignored_execution = executor
        .execute_invocation_detailed(&prepared_ignored.invocation, 1, 4)
        .await
        .expect("execute ignored grep");
    assert!(
        ignored_execution
            .view
            .output_text
            .contains("target/generated.txt")
    );
    assert!(
        ignored_execution
            .view
            .output_text
            .contains(".hidden/private.txt")
    );

    let concurrent = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(
            executor.execute_invocation_detailed(&prepared_workspace.invocation, 1, 5),
            executor.execute_invocation_detailed(&prepared_workspace.invocation, 1, 6),
        )
    })
    .await
    .expect("concurrent grep calls must remain responsive");
    assert!(
        concurrent
            .0
            .expect("first concurrent grep")
            .view
            .output_text
            .contains("fixture.txt")
    );
    assert!(
        concurrent
            .1
            .expect("second concurrent grep")
            .view
            .output_text
            .contains("fixture.txt")
    );

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
                .collect_permission_checks_for_invocation_in_session(&invocation, None)
                .await
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
        .collect_permission_checks_for_invocation_in_session(&invocation, None)
        .await
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
    )
    .for_session_context_async(&TestSessionContext {
        session_id: Some(1),
        access: agena_domain::ExecutionAccess::ReadOnly,
    })
    .await;

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
        .await
        .expect_err("read-only access must reject a mutating live tool");
    assert!(
        matches!(&error, ToolError::CapabilityUnavailable(_)),
        "out-of-capability tools must be hidden at invocation time, got {error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_scoped_dynamic_tools_are_stable_per_turn_and_isolated_across_sessions() {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let plugin_id = "test.scoped";
    let mut plugins_config = PluginsConfig::default();
    plugins_config
        .list
        .insert(plugin_id.to_owned(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            plugin_id.parse().expect("valid scoped plugin key"),
            ScopedDynamicToolFixture::default(),
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
    .expect("build scoped dynamic tool host");
    let base = ToolExecutor::new(
        workspace_root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        Arc::clone(&plugins),
        None,
        None,
        None,
    );
    let session_a = TestSessionContext {
        session_id: Some(101),
        access: agena_domain::ExecutionAccess::Inherit,
    };
    let session_b = TestSessionContext {
        session_id: Some(202),
        access: agena_domain::ExecutionAccess::Inherit,
    };

    let executor_a_turn_1 = base.for_session_context_async(&session_a).await;
    assert!(
        executor_a_turn_1
            .available_execution_tools()
            .iter()
            .any(|tool| tool.canonical_name() == "test.scoped.seed")
    );
    assert!(
        executor_a_turn_1
            .available_execution_tools()
            .iter()
            .all(|tool| tool.canonical_name() != "test.scoped.dynamic")
    );

    executor_a_turn_1
        .execute_invocation_detailed(
            &ToolInvocation::new("test.scoped.seed", StructuredObject::default()),
            101,
            1,
        )
        .await
        .expect("seed registers a session-scoped tool");

    assert!(
        executor_a_turn_1
            .available_execution_tools()
            .iter()
            .all(|tool| tool.canonical_name() != "test.scoped.dynamic"),
        "the current turn must keep its immutable catalog snapshot"
    );

    let executor_a_turn_2 = base.for_session_context_async(&session_a).await;
    assert!(
        executor_a_turn_2
            .available_execution_tools()
            .iter()
            .any(|tool| tool.canonical_name() == "test.scoped.dynamic"),
        "a new turn in the owning session sees the scoped registration"
    );
    executor_a_turn_2
        .execute_invocation_detailed(
            &ToolInvocation::new("test.scoped.dynamic", StructuredObject::default()),
            101,
            2,
        )
        .await
        .expect("owning session executes scoped dynamic tool");

    let executor_b = base.for_session_context_async(&session_b).await;
    assert!(
        executor_b
            .available_execution_tools()
            .iter()
            .all(|tool| tool.canonical_name() != "test.scoped.dynamic")
    );
    let error = executor_b
        .execute_invocation_detailed(
            &ToolInvocation::new("test.scoped.dynamic", StructuredObject::default()),
            202,
            1,
        )
        .await
        .expect_err("another session must not resolve the scoped dynamic tool");
    assert!(matches!(error, ToolError::ToolUnavailable(_)));

    assert!(
        base.available_execution_tools()
            .iter()
            .all(|tool| tool.canonical_name() != "test.scoped.dynamic"),
        "global callers never see a session-scoped registration"
    );

    plugins
        .broadcast_session_end(agena_plugin_host::sdk::SessionEndInput {
            session_id: 101,
            reason: agena_plugin_host::sdk::SessionEndReason::Other,
        })
        .await;
    let executor_a_after_end = base.for_session_context_async(&session_a).await;
    assert!(
        executor_a_after_end
            .available_execution_tools()
            .iter()
            .all(|tool| tool.canonical_name() != "test.scoped.dynamic"),
        "session.end must tear down the session visibility/lifetime scope"
    );
    assert!(
        plugins
            .architecture_catalog()
            .tool_registrations
            .iter()
            .all(|entry| !matches!(
                &entry.layer,
                agena_plugin_host::ScopedRegistryLayer::Scope { scope }
                    if scope.as_str() == "session:101"
            )),
        "architecture inspect must stop reporting registrations after scope teardown"
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
            .await
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
        .await
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
