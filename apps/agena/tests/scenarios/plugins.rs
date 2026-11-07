//! A real SDK HTTP plugin hosted by this test process and loaded by the product.
//! No plugin hook or tool method is invoked directly by the assertions.
mod model_hooks;

use std::sync::{
    OnceLock,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

use super::fixture;
use agena_domain::session_kernel::ExecutionAttemptSettlement;
use agena_plugin_sdk::prelude::*;
use agena_testkit::{EditOptions, Reply, RequestMatch, SettingsLayer, TestResult};
use anyhow::{Context, ensure};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct EchoSettings {
    uppercase: bool,
}

struct Observations {
    initializations: AtomicUsize,
    callbacks: AtomicUsize,
    before_tools: AtomicUsize,
    after_tools: AtomicUsize,
    post_runs: AtomicUsize,
    session_ends: AtomicUsize,
    last_end_post_runs: AtomicUsize,
    shutdowns: AtomicUsize,
    gate_next_post: AtomicBool,
    post_entered: Semaphore,
    post_release: Semaphore,
}

impl Default for Observations {
    fn default() -> Self {
        Self {
            initializations: AtomicUsize::new(0),
            callbacks: AtomicUsize::new(0),
            before_tools: AtomicUsize::new(0),
            after_tools: AtomicUsize::new(0),
            post_runs: AtomicUsize::new(0),
            session_ends: AtomicUsize::new(0),
            last_end_post_runs: AtomicUsize::new(0),
            shutdowns: AtomicUsize::new(0),
            gate_next_post: AtomicBool::new(false),
            post_entered: Semaphore::new(0),
            post_release: Semaphore::new(0),
        }
    }
}

struct EchoPlugin {
    settings: PluginSettings<EchoSettings>,
    host: OnceLock<Arc<dyn HostClient>>,
    seen: Arc<Observations>,
}

#[agena_plugin(
    namespace = "testkit",
    name = "echo",
    version = env!("CARGO_PKG_VERSION"),
    summary = "System-test echo plugin with real HTTP host callbacks.",
    settings = EchoSettings,
    settings_default = default,
    settings_field = settings
)]
impl EchoPlugin {
    #[hook(
        init,
        summary = "Initialize test plugin",
        help = "Retains the real HTTP callback client and records initialization."
    )]
    async fn initialize(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        if ctx.host_callback_url.is_none() || ctx.host_callback_token.is_none() {
            return Err(PluginError::internal(
                "the real host callback was not installed",
            ));
        }
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("initialized twice"))?;
        self.seen.initializations.fetch_add(1, Ordering::SeqCst);
        Ok(InitOutcome::ack(self.manifest()))
    }

    #[tool(
        name = "echo",
        summary = "Echo text using current settings.",
        tags(read_only)
    )]
    async fn echo(&self, #[arg(trim, non_empty)] text: String) -> Result<ToolInvokeOutput> {
        let value = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("missing host"))?
            .read_config(Some("config.ui.testkit_callback_marker".to_owned()))
            .await?;
        self.seen.callbacks.fetch_add(1, Ordering::SeqCst);
        let text = if self.settings.expect("initialized settings").uppercase {
            text.to_uppercase()
        } else {
            text
        };
        Ok(ToolInvokeOutput::text(format!("{text}\ncallback={value}")))
    }

    #[hook(tool.before, tool = "echo", summary = "Prepare echo", help = "Adds the before-hook marker.")]
    async fn before(&self, input: ToolBeforeInput) -> Option<ToolBeforePatch> {
        self.seen.before_tools.fetch_add(1, Ordering::SeqCst);
        let mut patched = input.input;
        if let Some(text) = patched
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            patched["text"] = json!(format!("[before] {text}"));
        }
        Some(ToolBeforePatch {
            input: Some(patched),
            ..Default::default()
        })
    }

    #[hook(tool.after, tool = "echo", summary = "Annotate echo", help = "Adds the after-hook marker.")]
    async fn after(&self, input: ToolAfterInput) -> Option<ToolAfterPatch> {
        self.seen.after_tools.fetch_add(1, Ordering::SeqCst);
        Some(ToolAfterPatch {
            output_text: Some(format!("{}\n[after]", input.output_text)),
            ..Default::default()
        })
    }

    #[hook(run.post, summary = "Observe settlement", help = "Can hold settlement for an explicit assertion.")]
    async fn post_run(&self, _input: PostRunInput) {
        if self.seen.gate_next_post.swap(false, Ordering::SeqCst) {
            self.seen.post_entered.add_permits(1);
            if let Ok(permit) = self.seen.post_release.acquire().await {
                permit.forget();
            }
        }
        self.seen.post_runs.fetch_add(1, Ordering::SeqCst);
    }

    #[hook(session.end, summary = "Observe session end", help = "Records the real session lifecycle callback.")]
    async fn session_end(&self, _input: SessionEndInput) {
        self.seen
            .last_end_post_runs
            .store(self.seen.post_runs.load(Ordering::SeqCst), Ordering::SeqCst);
        self.seen.session_ends.fetch_add(1, Ordering::SeqCst);
    }

    #[hook(
        shutdown,
        summary = "Observe shutdown",
        help = "Records awaited plugin shutdown before the fixture returns."
    )]
    async fn stop(&self) -> Result<()> {
        self.seen.shutdowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// A failed assertion must also release a held hook before case cleanup drains
// the real runtime. This is cleanup ownership, not a product hook invocation.
struct ReleaseOnDrop(Arc<Observations>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.post_release.add_permits(1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_plugin_tools_settings_callbacks_and_lifecycle_use_production_paths() -> TestResult {
    let seen = Arc::new(Observations::default());
    let setup_seen = seen.clone();
    let body_seen = seen.clone();
    let (ends_before_shutdown, shutdowns_before_shutdown) = fixture()
        .configure_global(|config| {
            config["ui"]["testkit_callback_marker"] = json!("callback-proof-marker");
        })
        .before_start(move |case| async move {
            let factory_seen = setup_seen;
            let router = agena_plugin_sdk::drivers::http::router(
                move || EchoPlugin {
                    settings: PluginSettings::new(),
                    host: OnceLock::new(),
                    seen: factory_seen.clone(),
                },
                Arc::new(NoopHostClient),
            );
            let service = case.serve_http(router).await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["testkit.echo"] = json!({
                "enabled": true,
                "package": {"kind": "http", "url": service.url("/rpc")},
                "settings": {"uppercase": false}
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .expect_plugin("testkit.echo")
        .run(move |case| async move {
            let model = case.model();
            let first = model
                .expect(RequestMatch::user_text("plugin-first-marker"))
                .reply(Reply::tool_call(
                    "call_plugin_first",
                    "testkit.echo.echo",
                    json!({"text": "mixedCase"}),
                ));
            model
                .expect(RequestMatch::tool_output("call_plugin_first"))
                .after(&first)
                .check_tool_output_contains("call_plugin_first", "[before] mixedCase")
                .check_tool_output_contains("call_plugin_first", "[after]")
                .check_tool_output_contains("call_plugin_first", "callback-proof-marker")
                .reply(Reply::text("plugin-first-complete"));

            let user = case.user().new_session().await?;
            body_seen.gate_next_post.store(true, Ordering::SeqCst);
            let _release_on_failure = ReleaseOnDrop(body_seen.clone());
            let run = user.send("plugin-first-marker").await?;
            let permit =
                tokio::time::timeout(Duration::from_secs(15), body_seen.post_entered.acquire())
                    .await??;
            permit.forget();
            let terminal = run.wait_terminal().await?;
            terminal.assert_succeeded()?;
            ensure!(
                terminal.status.settlement == ExecutionAttemptSettlement::Terminal,
                "{:?}",
                terminal.status
            );
            ensure!(
                body_seen.post_runs.load(Ordering::SeqCst) == 0,
                "held hook completed early"
            );
            body_seen.post_release.add_permits(1);
            run.wait_settled()
                .await?
                .assert_assistant_text("plugin-first-complete")?;
            ensure!(body_seen.post_runs.load(Ordering::SeqCst) == 1);

            case.settings()
                .set(
                    SettingsLayer::Global,
                    "plugins.list.\"testkit.echo\".settings.uppercase",
                    json!(true),
                    EditOptions::default(),
                )
                .await?
                .require_success()?;
            let second = model
                .expect(RequestMatch::user_text("plugin-second-marker"))
                .reply(Reply::tool_call(
                    "call_plugin_second",
                    "testkit.echo.echo",
                    json!({"text": "mixedCase"}),
                ));
            model
                .expect(RequestMatch::tool_output("call_plugin_second"))
                .after(&second)
                .check_tool_output_contains("call_plugin_second", "[BEFORE] MIXEDCASE")
                .check_tool_output_contains("call_plugin_second", "[after]")
                .reply(Reply::text("plugin-second-complete"));
            let result = user
                .send("plugin-second-marker")
                .await?
                .wait_settled()
                .await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("plugin-second-complete")?;
            ensure!(
                body_seen.initializations.load(Ordering::SeqCst) >= 2,
                "settings reload did not initialize a fresh plugin"
            );
            ensure!(body_seen.callbacks.load(Ordering::SeqCst) == 2);
            ensure!(body_seen.before_tools.load(Ordering::SeqCst) == 2);
            ensure!(body_seen.after_tools.load(Ordering::SeqCst) == 2);
            ensure!(body_seen.post_runs.load(Ordering::SeqCst) == 2);
            // Shutdown broadcasts session.end for active executions, not all
            // historical sessions. Leave a real request in flight so cleanup
            // must cancel it, drain run.post, then emit end before plugin stop.
            let shutdown_step = model
                .expect(RequestMatch::user_text("plugin-shutdown-marker"))
                .reply(Reply::text("held-until-shutdown").held());
            user.send("plugin-shutdown-marker").await?;
            shutdown_step.observed().await?;
            shutdown_step.expect_disconnection()?;
            Ok((
                body_seen.session_ends.load(Ordering::SeqCst),
                body_seen.shutdowns.load(Ordering::SeqCst),
            ))
        })
        .await?;
    ensure!(
        seen.session_ends.load(Ordering::SeqCst) > ends_before_shutdown,
        "case returned before session.end"
    );
    ensure!(
        seen.shutdowns.load(Ordering::SeqCst) > shutdowns_before_shutdown,
        "case returned before plugin shutdown"
    );
    ensure!(
        seen.last_end_post_runs.load(Ordering::SeqCst) == 3,
        "session.end ran before the interrupted execution's run.post settled"
    );
    Ok(())
}

struct ReloadObservations {
    gate_next_config: AtomicBool,
    config_entered: Semaphore,
    config_release: Semaphore,
    config_completed: AtomicBool,
    invocations: AtomicUsize,
    callbacks: AtomicUsize,
}

impl Default for ReloadObservations {
    fn default() -> Self {
        Self {
            gate_next_config: AtomicBool::new(false),
            config_entered: Semaphore::new(0),
            config_release: Semaphore::new(0),
            config_completed: AtomicBool::new(false),
            invocations: AtomicUsize::new(0),
            callbacks: AtomicUsize::new(0),
        }
    }
}

struct ReloadProbe {
    host: OnceLock<Arc<dyn HostClient>>,
    seen: Arc<ReloadObservations>,
}

#[agena_plugin(
    namespace = "testkit",
    name = "reload_probe",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Exercises real callbacks across an unpublished runtime candidate."
)]
impl ReloadProbe {
    #[hook(
        init,
        summary = "Initialize reload probe",
        help = "Retains the SDK callback client belonging to this plugin instance."
    )]
    async fn initialize(
        &self,
        _ctx: InitContext,
        host: Arc<dyn HostClient>,
    ) -> Result<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("reload probe initialized twice"))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    #[hook(
        config.resolved,
        summary = "Hold a reload candidate",
        help = "Keeps candidate configuration pending until the test releases its gate."
    )]
    async fn configured(&self, _input: ConfigInput) -> Result<Option<ConfigPatch>> {
        if self.seen.gate_next_config.swap(false, Ordering::SeqCst) {
            self.seen.config_entered.add_permits(1);
            let permit = self
                .seen
                .config_release
                .acquire()
                .await
                .map_err(|_| PluginError::internal("reload candidate gate was closed"))?;
            permit.forget();
            self.seen.config_completed.store(true, Ordering::SeqCst);
        }
        Ok(None)
    }

    #[tool(
        name = "echo",
        summary = "Echo text after a real Host configuration callback.",
        tags(read_only)
    )]
    async fn echo(&self, #[arg(trim, non_empty)] text: String) -> Result<ToolInvokeOutput> {
        self.seen.invocations.fetch_add(1, Ordering::SeqCst);
        let value = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("reload probe has no Host client"))?
            .read_config(Some("config.ui.testkit_callback_marker".to_owned()))
            .await?;
        self.seen.callbacks.fetch_add(1, Ordering::SeqCst);
        Ok(ToolInvokeOutput::text(format!("{text}\ncallback={value}")))
    }
}

// Always release the product hook before TestCase begins its owned cleanup,
// including assertion failure or cancellation of the test body.
struct ReleaseConfigOnDrop(Arc<ReloadObservations>);

impl Drop for ReleaseConfigOnDrop {
    fn drop(&mut self) {
        self.0.gate_next_config.store(false, Ordering::SeqCst);
        self.0.config_release.add_permits(1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_plugin_callbacks_remain_valid_while_reload_candidate_is_unpublished() -> TestResult {
    use agena_api::resource::RuntimeBackgroundTaskStatus;

    let seen = Arc::new(ReloadObservations::default());
    let setup_seen = seen.clone();
    fixture()
        .configure_global(|config| {
            config["ui"]["testkit_callback_marker"] = json!("reload-callback-proof");
            // Config hooks default to two seconds. The gate, not that timeout,
            // must control publication while a complete model/tool round trip runs.
            config["plugins"]["host"]["timeouts"]["fast"] = json!("45s");
        })
        .before_start(move |case| async move {
            let router = agena_plugin_sdk::drivers::http::router(
                move || ReloadProbe {
                    host: OnceLock::new(),
                    seen: setup_seen.clone(),
                },
                Arc::new(NoopHostClient),
            );
            let service = case.serve_http(router).await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["testkit.reload_probe"] = json!({
                "enabled": true,
                "package": {"kind": "http", "url": service.url("/rpc")}
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .expect_plugin("testkit.reload_probe")
        .run(move |case| async move {
            let _release_on_failure = ReleaseConfigOnDrop(seen.clone());
            let settings = case.settings();
            // Complete an initial exact reload before arming the gate. The
            // next explicit reload leaves the plugin entry unchanged and
            // exercises the production reuse decision.
            settings.reload_and_wait().await?;

            let model = case.model();
            let old_reply = model
                .expect(RequestMatch::user_text("old-run-before-candidate"))
                .reply(
                    Reply::tool_call(
                        "call_reload_old",
                        "testkit.reload_probe.echo",
                        json!({"text": "old-run-during-candidate"}),
                    )
                    .held(),
                );
            model
                .expect(RequestMatch::tool_output("call_reload_old"))
                .after(&old_reply)
                .check(|request| {
                    request.require_tool_output("call_reload_old")?;
                    let output: Value = serde_json::from_str(
                        &request
                            .tool_output_text("call_reload_old")
                            .context("old run did not receive its tool result")?,
                    )?;
                    ensure!(
                        output["phase"] == "succeeded",
                        "old runtime tool failed during candidate construction: {output}"
                    );
                    Ok(())
                })
                .check_tool_output_contains("call_reload_old", "old-run-during-candidate")
                .check_tool_output_contains("call_reload_old", "reload-callback-proof")
                .reply(Reply::text("old-run-completed-before-publication"));

            let user = case.user().new_session().await?;
            let old_run = user.send("old-run-before-candidate").await?;
            old_reply.observed().await?;
            ensure!(!old_run.status().await?.phase.is_terminal());

            seen.gate_next_config.store(true, Ordering::SeqCst);
            let reload = settings.start_reload().await?;
            tokio::time::timeout(Duration::from_secs(15), seen.config_entered.acquire())
                .await
                .context("reload did not reach the real config hook")??
                .forget();
            ensure!(!seen.config_completed.load(Ordering::SeqCst));

            // Runtime dispatches config.resolved after plugin construction and
            // before publishing the candidate. This public task is still
            // running while the already accepted old execution resumes.
            let client = case.client().await;
            let held = client.runtime_status().await?;
            let held_task = held
                .background_tasks
                .iter()
                .find(|task| task.id == reload.task_id())
                .context("the held reload receipt disappeared")?;
            ensure!(held_task.status == RuntimeBackgroundTaskStatus::Running);

            old_reply.release();
            let old_result = old_run.wait_settled().await?;
            old_result.assert_succeeded()?;
            old_result.assert_assistant_text("old-run-completed-before-publication")?;
            ensure!(seen.invocations.load(Ordering::SeqCst) == 1);
            ensure!(seen.callbacks.load(Ordering::SeqCst) == 1);
            ensure!(
                !seen.config_completed.load(Ordering::SeqCst),
                "the config gate was released before the old attempt settled"
            );
            let still_held = client.runtime_status().await?;
            ensure!(
                still_held.generation == held.generation,
                "candidate publication overtook its held config hook"
            );
            ensure!(
                still_held
                    .background_tasks
                    .iter()
                    .find(|task| task.id == reload.task_id())
                    .is_some_and(|task| task.status == RuntimeBackgroundTaskStatus::Running),
                "reload finished while its config hook was still held"
            );

            seen.config_release.add_permits(1);
            let published = reload.wait().await?;
            ensure!(seen.config_completed.load(Ordering::SeqCst));
            ensure!(
                published.generation > held.generation,
                "releasing the candidate did not publish a fresh runtime"
            );
            let new_result = user
                .call_tool(
                    "testkit.reload_probe.echo",
                    json!({"text": "new-run-after-publication"}),
                )
                .await?;
            new_result.assert_succeeded()?;
            new_result.assert_text_contains("new-run-after-publication")?;
            new_result.assert_text_contains("reload-callback-proof")?;
            ensure!(seen.invocations.load(Ordering::SeqCst) == 2);
            ensure!(seen.callbacks.load(Ordering::SeqCst) == 2);
            Ok(())
        })
        .await
}
