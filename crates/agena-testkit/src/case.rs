//! A Rust-test-owned Agena product process. The async run boundary is the
//! recommended API because it awaits cleanup after success, error, panic, and
//! timeout. Drop is only an emergency process-tree kill.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use agena_api::resource::ServerEndpointRecord;
use agena_client::AgenaClient;
use anyhow::{Context, Result, anyhow, bail, ensure};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::{
    sync::{Mutex, RwLock},
    task::{AbortHandle, JoinHandle},
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{
        CasePaths, ConfigFile, ConfigSnapshot, SettingsDriver, SettingsLayer, local_model_config,
        owned_path, write_bytes,
    },
    http::{TestHttpClient, validate_server_url},
    model::{ModelHandle, ModelOptions, ModelProtocol, ScriptedModel},
    process::{OwnedProcess, ProcessExit, ProcessSpec, environment_password, isolated_environment},
};

/// Entry point for an explicitly selected product binary. The testkit never
/// searches for a running user server or falls back to the normal port.
pub struct TestAgena;

impl TestAgena {
    pub fn builder(binary: impl Into<PathBuf>) -> TestAgenaBuilder {
        TestAgenaBuilder::new(binary.into())
    }
}

type ConfigEdit = Box<dyn FnOnce(&mut serde_json::Value, &CasePaths, &ModelHandle) + Send>;
type BeforeStart =
    Box<dyn FnOnce(TestCase) -> futures_util::future::BoxFuture<'static, Result<()>> + Send>;

pub struct TestAgenaBuilder {
    binary: PathBuf,
    preset: bool,
    global: Option<ConfigFile>,
    workspace: ConfigFile,
    edits: Vec<ConfigEdit>,
    before_start: Vec<BeforeStart>,
    environment: BTreeMap<OsString, Option<OsString>>,
    environment_paths: Vec<(OsString, PathBuf)>,
    raw_args: Vec<OsString>,
    seed_files: Vec<(PathBuf, Vec<u8>)>,
    cwd: PathBuf,
    password: Option<String>,
    startup_timeout: Duration,
    case_timeout: Duration,
    shutdown_timeout: Duration,
    require_session_runtime: bool,
    allow_failed_plugins: bool,
    expected_plugins: Vec<String>,
    keep_artifacts: bool,
    model_options: ModelOptions,
    selected_protocol: Option<ModelProtocol>,
    scheduler_clock_start: Option<chrono::DateTime<chrono::Utc>>,
}

impl TestAgenaBuilder {
    fn new(binary: PathBuf) -> Self {
        Self {
            binary,
            preset: true,
            global: None,
            workspace: ConfigFile::Missing,
            edits: Vec::new(),
            before_start: Vec::new(),
            environment: BTreeMap::new(),
            environment_paths: Vec::new(),
            raw_args: Vec::new(),
            seed_files: Vec::new(),
            cwd: PathBuf::from("workspace"),
            password: Some(format!("testkit-{}", uuid::Uuid::new_v4())),
            startup_timeout: Duration::from_secs(30),
            case_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(8),
            require_session_runtime: true,
            allow_failed_plugins: false,
            expected_plugins: Vec::new(),
            keep_artifacts: false,
            model_options: ModelOptions::default(),
            selected_protocol: None,
            scheduler_clock_start: None,
        }
    }

    /// No configuration document is injected. Process/home/environment
    /// isolation still applies. Explicit files and CLI inputs retain their
    /// production meaning, including production defaults and validation errors.
    pub fn raw(mut self) -> Self {
        self.preset = false;
        self.edits.clear();
        self.require_session_runtime = false;
        self
    }

    /// Replace the whole global source, including the local-model preset.
    pub fn global_config(mut self, source: impl Into<ConfigFile>) -> Self {
        self.global = Some(source.into());
        self.edits.clear();
        self
    }

    pub fn workspace_config(mut self, source: impl Into<ConfigFile>) -> Self {
        self.workspace = source.into();
        self
    }

    /// Configure an enabled plugin already compiled into the product. Emits a
    /// complete production entry, including the required static package source.
    /// Use the raw configuration APIs for other transports or invalid fixtures.
    pub fn builtin_plugin(self, id: impl Into<String>, settings: serde_json::Value) -> Self {
        let id = id.into();
        self.configure_global(move |config| {
            config["plugins"]["list"][&id] = serde_json::json!({
                "enabled": true,
                "package": {"kind": "static"},
                "settings": settings
            });
        })
    }

    /// Edit the selected fixture JSON before writing it. This edits a single
    /// source document; production layer merge behavior is left to Agena.
    pub fn configure_global(
        self,
        edit: impl FnOnce(&mut serde_json::Value) + Send + 'static,
    ) -> Self {
        self.configure_global_with(move |value, _, _| edit(value))
    }

    pub fn configure_global_with(
        mut self,
        edit: impl FnOnce(&mut serde_json::Value, &CasePaths, &ModelHandle) + Send + 'static,
    ) -> Self {
        self.edits.push(Box::new(edit));
        self
    }

    /// Prepare local dependencies after paths, configuration, seed files, and
    /// the scripted model exist, but before the product process starts.
    /// Hooks run once, in registration order, and do not run again on restart.
    ///
    /// Use paths/model, serve_http, spawn_service, and write_config here.
    /// Client, HTTP, endpoint, user, and settings drivers require a started
    /// product and must not be used in this hook. The entire hook sequence has
    /// the startup_timeout budget; product readiness has its own startup budget.
    /// Error, panic, and timeout all await case cleanup before leaving startup.
    pub fn before_start<F, Fut>(mut self, setup: F) -> Self
    where
        F: FnOnce(TestCase) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.before_start
            .push(Box::new(move |case| setup(case).boxed()));
        self
    }

    pub fn env(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        let name = name.into();
        self.environment_paths.retain(|(key, _)| key != &name);
        self.environment.insert(name, Some(value.into()));
        self
    }

    pub fn env_remove(mut self, name: impl Into<OsString>) -> Self {
        let name = name.into();
        self.environment_paths.retain(|(key, _)| key != &name);
        self.environment.insert(name, None);
        self
    }

    /// Resolve an environment value under this case's root, useful for secret
    /// files and prompt directories without a process-global environment edit.
    pub fn env_relative(mut self, name: impl Into<OsString>, path: impl Into<PathBuf>) -> Self {
        let name = name.into();
        self.environment.remove(&name);
        self.environment_paths.retain(|(key, _)| key != &name);
        self.environment_paths.push((name, path.into()));
        self
    }

    /// Opt into a file-backed virtual wall clock used only by the scheduler.
    /// Real HTTP deadlines, process shutdown and Tokio's monotonic clock are
    /// intentionally unaffected. No real-time wait is needed for a long job.
    pub fn scheduler_clock(mut self, start: chrono::DateTime<chrono::Utc>) -> Self {
        self.scheduler_clock_start = Some(start);
        self.env_relative(
            "AGENA_TEST_SCHEDULER_CLOCK_FILE",
            "control/scheduler-clock.txt",
        )
        .seed_file(
            "control/scheduler-clock.txt",
            start.timestamp_millis().to_string().into_bytes(),
        )
    }

    /// Enable a frozen, case-owned clock for scheduler UTC and explicit
    /// TestClock timers. HTTP, process and cleanup deadlines remain real.
    pub fn test_clock(self, start: chrono::DateTime<chrono::Utc>) -> Self {
        self.scheduler_clock(start)
    }

    /// Enable opt-in kernel transaction fault gates in a debug Agena child.
    /// Starting a case does not arm any failure until the test explicitly does.
    pub fn kernel_faults(self) -> Self {
        self.env_relative("AGENA_TEST_KERNEL_FAULT_FILE", "control/kernel-fault.json")
            .seed_file(
                "control/kernel-fault.json",
                br#"{"point":"","effect":"fail","remaining":0,"hits":0,"released":true}"#.to_vec(),
            )
    }

    /// Select a real non-OpenAI provider adapter and its own loopback wire
    /// protocol while preserving strict expected-request matching.
    pub fn native_protocol(self, protocol: crate::native::NativeProtocol) -> Self {
        self.model_protocol(protocol.into())
    }

    /// Use the same Reply and RequestMatch APIs with a real product adapter.
    /// Protocol defaults are applied during preparation, so later calls to
    /// model_options can tune limits without accidentally losing the protocol.
    /// BedrockConverse is a wire fixture only and is rejected before startup.
    pub fn model_protocol(mut self, protocol: ModelProtocol) -> Self {
        self.selected_protocol = Some(protocol);
        self.configure_global_with(move |config, _, model| {
            // An earlier selection may have been replaced by a supported one.
            // Preparation rejects an unsupported final selection before resources exist.
            let Some(adapter) = protocol.adapter() else {
                return;
            };
            let base = if matches!(
                protocol,
                ModelProtocol::Ollama | ModelProtocol::Gemini | ModelProtocol::BedrockAnthropic
            ) {
                model.base_url().trim_end_matches("/v1")
            } else {
                model.base_url()
            };
            config["providers"]["testkit"]["auth"] = match protocol {
                ModelProtocol::Ollama => serde_json::json!({"mode":"none"}),
                ModelProtocol::BedrockAnthropic => serde_json::json!({
                    "mode":"api", "subtype":"bedrock_sigv4", "base_url":base,
                    "region":"us-east-1", "access_key_id":"agena-test-access-key",
                    "secret_access_key":"agena-test-secret-key"
                }),
                _ => serde_json::json!({
                    "mode":"api", "subtype":"custom", "base_url":base,
                    "api_key":{"kind":"inline","value":"agena-testkit"}
                }),
            };
            config["providers"]["default_selection"]["adapter"] = serde_json::json!(adapter);
            config["providers"]["default_selection"]["model"] = serde_json::json!(model.model_id());
            let mut definition = serde_json::json!({
                "enabled":true,"model_discovery":"configured_only","models":{
                    (model.model_id()):{
                        "native_compaction":false,"context_window_tokens":131072,
                        "max_output_tokens":8192,
                        "agena_tools":{"mode":"provider_protocol"}
                    }
                }
            });
            if protocol == ModelProtocol::Ollama {
                definition["base_url"] = serde_json::json!(base);
            }
            let mut adapters = serde_json::Map::new();
            adapters.insert(adapter.to_owned(), definition);
            config["providers"]["testkit"]["adapters"] = serde_json::Value::Object(adapters);
        })
    }

    /// Raw additional foreground CLI arguments. Ownership and lifecycle flags
    /// are reserved; malformed ordinary flags are intentionally passed through.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.raw_args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.raw_args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cli_override(self, value: impl Into<OsString>) -> Self {
        self.arg("--set").arg(value)
    }

    /// Files may live anywhere under the case root: workspace/, data/, home/,
    /// fixtures/, etc. Absolute paths, traversal, and symlink ancestors fail.
    pub fn seed_file(mut self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) -> Self {
        self.seed_files.push((path.into(), bytes.into()));
        self
    }

    /// Working directory relative to the owned case root.
    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = path.into();
        self
    }

    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub fn without_password(mut self) -> Self {
        self.password = None;
        self
    }

    pub fn startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn case_timeout(mut self, timeout: Duration) -> Self {
        self.case_timeout = timeout;
        self
    }

    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    pub fn require_session_runtime(mut self, required: bool) -> Self {
        self.require_session_runtime = required;
        self
    }

    pub fn allow_failed_plugins(mut self, allow: bool) -> Self {
        self.allow_failed_plugins = allow;
        self
    }

    pub fn expect_plugin(mut self, id: impl Into<String>) -> Self {
        self.expected_plugins.push(id.into());
        self
    }

    pub fn keep_artifacts(mut self, keep: bool) -> Self {
        self.keep_artifacts = keep;
        self
    }

    /// Configure scripted model IDs, deadlines, and bounded capture limits.
    pub fn model_options(mut self, options: ModelOptions) -> Self {
        self.model_options = options;
        self
    }

    pub(crate) async fn prepare(mut self) -> Result<TestCase> {
        ensure!(
            !self.startup_timeout.is_zero()
                && !self.case_timeout.is_zero()
                && !self.shutdown_timeout.is_zero(),
            "fixture deadlines must be greater than zero"
        );
        if let Some(start) = self.scheduler_clock_start {
            ensure!(
                chrono::DateTime::from_timestamp_millis(start.timestamp_millis()) == Some(start),
                "test clock start must be exactly representable at millisecond precision"
            );
        }
        if let Some(protocol) = self.selected_protocol {
            ensure!(
                protocol.adapter().is_some(),
                "{protocol:?} is available as a scripted HTTP fixture, but Agena has no product adapter for it"
            );
            let defaults = ModelOptions::for_protocol(protocol);
            self.model_options.protocol = protocol;
            self.model_options.models = defaults.models;
            self.model_options.routes.extend(defaults.routes);
            self.model_options.auto_openai_models = defaults.auto_openai_models;
        }
        let binary = std::fs::canonicalize(&self.binary).with_context(|| {
            format!("resolving explicit Agena binary {}", self.binary.display())
        })?;
        let temp = tempfile::Builder::new()
            .prefix("agena-testkit-")
            .tempdir()?;
        let paths = CasePaths::new(temp.path().to_owned(), &self.cwd)?;
        paths.create().await?;
        // Validate all ownership inputs before starting either owned server.
        for (relative, _) in &self.seed_files {
            owned_path(&paths.root, relative)?;
        }
        let mut environment = self.environment;
        for (name, relative) in self.environment_paths {
            environment.insert(
                name,
                Some(owned_path(&paths.root, &relative)?.into_os_string()),
            );
        }
        let environment = isolated_environment(&paths, self.password.as_deref(), &environment);
        let spec = ProcessSpec {
            binary,
            raw_args: self.raw_args,
            password: environment_password(&environment)?,
            environment,
        };
        spec.validate()?;
        let owner = ScriptedModel::start_with_options(self.model_options)
            .await
            .context("starting scripted model endpoint")?;
        let model = owner.handle();
        let global = self.global.unwrap_or_else(|| {
            if self.preset {
                // The owned endpoint is constructed by ScriptedModel and
                // cannot be redirected by a fixture environment variable.
                ConfigFile::Json(
                    local_model_config(owner.base_url(), model.model_id())
                        .expect("ScriptedModel must expose a loopback HTTP endpoint"),
                )
            } else {
                ConfigFile::Missing
            }
        });
        let case = TestCase {
            inner: Arc::new(CaseInner {
                paths,
                temp: StdMutex::new(Some(temp)),
                spec: RwLock::new(spec),
                model,
                model_owner: Mutex::new(Some(owner)),
                process: Mutex::new(None),
                connection: RwLock::new(None),
                lifecycle: Mutex::new(()),
                virtual_clock_lock: Arc::new(Mutex::new(())),
                cancel: CancellationToken::new(),
                server_cancel: RwLock::new(CancellationToken::new()),
                monitor_cancel: CancellationToken::new(),
                monitor: StdMutex::new(None),
                tasks: StdMutex::new(Vec::new()),
                services: StdMutex::new(Vec::new()),
                service_cancel: CancellationToken::new(),
                fatal: CancellationToken::new(),
                failure: StdMutex::new(None),
                diagnostics: StdMutex::new(Vec::new()),
                finish_result: StdMutex::new(None),
                finish_wait: StdMutex::new(None),
                finishing: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                epoch: AtomicU64::new(0),
                expected_kernel_crash: AtomicBool::new(false),
                startup_timeout: self.startup_timeout,
                case_timeout: self.case_timeout,
                shutdown_timeout: self.shutdown_timeout,
                require_session_runtime: self.require_session_runtime,
                allow_failed_plugins: self.allow_failed_plugins,
                expected_plugins: self.expected_plugins,
            }),
        };
        let mut boundary = CleanupBoundary::new(case.clone(), "fixture preparation");
        if self.keep_artifacts {
            case.retain_artifacts();
        }
        let setup = std::panic::AssertUnwindSafe(async {
            let mut global = global;
            if !self.edits.is_empty() {
                let ConfigFile::Json(value) = &mut global else {
                    bail!("configure_global requires a JSON fixture document");
                };
                for edit in self.edits {
                    edit(value, case.paths(), &case.model());
                }
            }
            global.write(&case.paths().global_config).await?;
            self.workspace.write(&case.paths().workspace_config).await?;
            // Explicit seed bytes are applied last so fixtures can deliberately
            // replace a generated document with exact malformed content.
            for (relative, bytes) in self.seed_files {
                case.seed_file(relative, bytes).await?;
            }
            let model = case.model();
            let before_start = async {
                for setup in self.before_start {
                    setup(case.clone()).await?;
                }
                Ok::<_, anyhow::Error>(())
            };
            tokio::select! {
                result = tokio::time::timeout(case.inner.startup_timeout, before_start) => {
                    result.context("before_start setup deadline elapsed")??;
                }
                error = model.failed() => return Err(error.context("model failed during before_start setup")),
                error = case.failed() => return Err(error.context("owned fixture failed during before_start setup")),
            }
            ensure!(
                case.inner.epoch.load(Ordering::Acquire) == 0
                    && !case.inner.finishing.load(Ordering::Acquire),
                "before_start must leave the product unstarted and its case open"
            );
            Ok::<_, anyhow::Error>(())
        })
        .catch_unwind()
        .await;
        match setup {
            Ok(Ok(())) => {
                boundary.disarm();
                Ok(case)
            }
            Ok(Err(error)) => {
                case.record_diagnostic(format!("fixture setup failed: {error:#}"));
                case.retain_artifacts();
                let cleanup = case.finish_impl(true).await;
                boundary.disarm();
                Err(case.failure_with_cleanup(error, cleanup))
            }
            Err(payload) => {
                case.record_diagnostic(format!(
                    "fixture setup panicked: {}",
                    panic_message(payload.as_ref())
                ));
                case.retain_artifacts();
                if let Err(error) = case.finish_impl(true).await {
                    eprintln!("Agena fixture setup cleanup failed: {error:#}");
                }
                eprintln!(
                    "Agena test diagnostics: {}",
                    case.paths().diagnostics.display()
                );
                boundary.disarm();
                std::panic::resume_unwind(payload)
            }
        }
    }

    /// Start a case with explicit async finish responsibility. Prefer run unless
    /// a surrounding fixture already owns an async teardown boundary.
    pub async fn start(self) -> Result<TestCase> {
        let case = self.prepare().await?;
        let mut boundary = CleanupBoundary::new(case.clone(), "product startup");
        if let Err(error) = case.launch().await {
            case.record_diagnostic(format!("startup failed: {error:#}"));
            case.retain_artifacts();
            let cleanup = case.finish_impl(true).await;
            boundary.disarm();
            return Err(case.failure_with_cleanup(error, cleanup));
        }
        case.start_monitor();
        boundary.disarm();
        Ok(case)
    }

    /// Expect a real product startup error, not a discovery timeout or fixture
    /// construction error. A successful server or a clean exit fails this test.
    pub async fn expect_start_failure(self) -> Result<StartupFailure> {
        let case = self.prepare().await?;
        let mut boundary = CleanupBoundary::new(case.clone(), "expected startup failure");
        let result = case.launch().await;
        let status = {
            let mut process = case.inner.process.lock().await;
            process.as_mut().map(OwnedProcess::poll_exit).transpose()
        };
        let status = match status {
            Ok(status) => status.flatten(),
            Err(error) => {
                case.record_diagnostic(format!("checking startup failure: {error:#}"));
                case.retain_artifacts();
                let cleanup = case.finish_impl(true).await;
                boundary.disarm();
                return Err(case.failure_with_cleanup(error, cleanup));
            }
        };
        case.retain_artifacts();
        let (cause, expected) = match (result, status) {
            (Err(error), Some(status)) if !status.success() => (format!("{error:#}"), true),
            (Err(error), _) => (
                format!("expected a nonzero startup exit, but observed: {error:#}"),
                false,
            ),
            (Ok(()), _) => (
                "expected startup failure, but Agena became ready".to_owned(),
                false,
            ),
        };
        case.record_diagnostic(cause.clone());
        let cleanup = case.finish_impl(true).await;
        boundary.disarm();
        if !expected {
            return Err(case.failure_with_cleanup(anyhow!(cause), cleanup));
        }
        cleanup.context("cleaning up expected startup failure")?;
        let (stdout, stderr) = {
            let process = case.inner.process.lock().await;
            process
                .as_ref()
                .map(OwnedProcess::output)
                .unwrap_or_default()
        };
        Ok(StartupFailure {
            code: status.and_then(|status| status.code()),
            cause,
            diagnostics: case.paths().diagnostics.clone(),
            root: case.paths().root.clone(),
            stdout,
            stderr,
        })
    }

    /// Run a normal async Rust test body and await owned-resource cleanup even
    /// when the body returns an error, panics, stalls, or an endpoint fails.
    pub async fn run<F, Fut, T>(self, body: F) -> Result<T>
    where
        F: FnOnce(TestCase) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let case = self.start().await?;
        let mut boundary = CleanupBoundary::new(case.clone(), "test body");
        let outcome = {
            let body_case = case.clone();
            let future =
                std::panic::AssertUnwindSafe(async move { body(body_case).await }).catch_unwind();
            tokio::pin!(future);
            let model = case.model();
            tokio::select! {
                outcome = &mut future => outcome,
                error = model.failed() => Ok(Err(error.context("scripted model endpoint failed"))),
                error = case.failed() => Ok(Err(error)),
                _ = tokio::time::sleep(case.timeout()) => Ok(Err(anyhow!(
                    "Agena test body timed out after {:?}", case.timeout()
                ))),
            }
        };
        // The losing body future has now been dropped, releasing any in-flight
        // request before observer and process cleanup begins.
        match &outcome {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                case.record_diagnostic(format!("test failed: {error:#}"));
                case.retain_artifacts();
            }
            Err(payload) => {
                case.record_diagnostic(format!(
                    "test panicked: {}",
                    panic_message(payload.as_ref())
                ));
                case.retain_artifacts();
            }
        }
        let cleanup = case.finish().await;
        boundary.disarm();
        match outcome {
            Ok(Ok(value)) => {
                cleanup?;
                Ok(value)
            }
            Ok(Err(error)) => Err(case.failure_with_cleanup(error, cleanup)),
            Err(payload) => {
                if let Err(error) = cleanup {
                    eprintln!("Agena test cleanup also failed: {error:#}");
                }
                eprintln!(
                    "Agena test diagnostics: {}",
                    case.paths().diagnostics.display()
                );
                std::panic::resume_unwind(payload)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct StartupFailure {
    pub code: Option<i32>,
    pub cause: String,
    pub diagnostics: PathBuf,
    pub root: PathBuf,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone)]
struct Connection {
    endpoint: String,
    client: AgenaClient,
    http: TestHttpClient,
}

/// Cloneable access to one owned case. Clones share lifecycle, process epoch,
/// task ownership, paths, and expectations; cloning never starts another server.
#[derive(Clone)]
pub struct TestCase {
    inner: Arc<CaseInner>,
}

/// Cancelling the surrounding future cannot await teardown itself. Transfer
/// cleanup to the same owned operation used by finish(), even if a service or
/// Router still holds a TestCase clone and would otherwise keep the case alive.
struct CleanupBoundary {
    case: Option<TestCase>,
    phase: &'static str,
}

impl CleanupBoundary {
    fn new(case: TestCase, phase: &'static str) -> Self {
        Self {
            case: Some(case),
            phase,
        }
    }

    fn disarm(&mut self) {
        self.case.take();
    }
}

impl Drop for CleanupBoundary {
    fn drop(&mut self) {
        if let Some(case) = self.case.take() {
            case.record_diagnostic(format!(
                "{} future was cancelled; owned cleanup continues asynchronously",
                self.phase
            ));
            case.retain_artifacts();
            case.begin_finish(true);
        }
    }
}

tokio::task_local! {
    static OWNED_TASK_CASE: usize;
}

/// The case joins the observer, which in turn joins the actual producer. Abort
/// the producer first: aborting only the observer would detach its child.
struct OwnedTask {
    abort: AbortHandle,
    join: JoinHandle<Result<()>>,
}

impl OwnedTask {
    fn abort(&self) {
        self.abort.abort();
    }
}

impl Future for OwnedTask {
    type Output = std::result::Result<Result<()>, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.join).poll(cx)
    }
}

impl Drop for OwnedTask {
    fn drop(&mut self) {
        self.abort.abort();
        self.join.abort();
    }
}

struct AbortOnDropTask(JoinHandle<Result<()>>);

impl Future for AbortOnDropTask {
    type Output = std::result::Result<Result<()>, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}

impl Drop for AbortOnDropTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct CaseInner {
    paths: CasePaths,
    temp: StdMutex<Option<tempfile::TempDir>>,
    spec: RwLock<ProcessSpec>,
    model: ModelHandle,
    model_owner: Mutex<Option<ScriptedModel>>,
    process: Mutex<Option<OwnedProcess>>,
    connection: RwLock<Option<Connection>>,
    lifecycle: Mutex<()>,
    virtual_clock_lock: Arc<Mutex<()>>,
    cancel: CancellationToken,
    server_cancel: RwLock<CancellationToken>,
    monitor_cancel: CancellationToken,
    monitor: StdMutex<Option<JoinHandle<()>>>,
    tasks: StdMutex<Vec<OwnedTask>>,
    services: StdMutex<Vec<OwnedTask>>,
    service_cancel: CancellationToken,
    fatal: CancellationToken,
    failure: StdMutex<Option<String>>,
    diagnostics: StdMutex<Vec<String>>,
    finish_result: StdMutex<Option<std::result::Result<(), String>>>,
    finish_wait:
        StdMutex<Option<tokio::sync::watch::Receiver<Option<std::result::Result<(), String>>>>>,
    finishing: AtomicBool,
    finished: AtomicBool,
    epoch: AtomicU64,
    expected_kernel_crash: AtomicBool,
    startup_timeout: Duration,
    case_timeout: Duration,
    shutdown_timeout: Duration,
    require_session_runtime: bool,
    allow_failed_plugins: bool,
    expected_plugins: Vec<String>,
}

impl Drop for CaseInner {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.monitor_cancel.cancel();
        self.service_cancel.cancel();
        if let Some(monitor) = self
            .monitor
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            monitor.abort();
        }
        for task in self
            .tasks
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .drain(..)
        {
            task.abort();
        }
        for task in self
            .services
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .drain(..)
        {
            task.abort();
        }
        if !self.finished.load(Ordering::Acquire)
            && let Some(temp) = self
                .temp
                .get_mut()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        {
            let path = temp.keep();
            eprintln!(
                "Agena fixture dropped without awaited finish; process-tree kill is a fallback. Artifacts: {}",
                path.display()
            );
        }
    }
}

impl TestCase {
    pub fn paths(&self) -> &CasePaths {
        &self.inner.paths
    }

    pub fn workspace(&self) -> &Path {
        &self.paths().workspace
    }

    pub fn data_dir(&self) -> &Path {
        &self.paths().data_dir
    }

    pub fn timeout(&self) -> Duration {
        self.inner.case_timeout
    }

    pub(crate) async fn cli_process_spec(&self) -> ProcessSpec {
        self.inner.spec.read().await.clone()
    }

    pub fn model(&self) -> ModelHandle {
        self.inner.model.clone()
    }

    /// Operate the frozen scheduler clock and explicit virtual timers configured
    /// by test_clock (or its scheduler_clock compatibility name).
    pub fn clock(&self) -> Result<crate::clock::TestClock> {
        let path = self.paths().root.join("control/scheduler-clock.txt");
        ensure!(
            path.is_file(),
            "configure builder.test_clock(start) before requesting a clock driver"
        );
        Ok(crate::clock::TestClock::new(
            path,
            self.inner.virtual_clock_lock.clone(),
        ))
    }

    /// Return the case-owned kernel fault controller (if enabled before start).
    pub fn faults(&self) -> Result<crate::fault::FaultDriver> {
        let path = self.paths().root.join("control/kernel-fault.json");
        ensure!(
            path.is_file(),
            "configure builder.kernel_faults() before requesting fault injection"
        );
        Ok(
            crate::fault::FaultDriver::new(path, self.inner.virtual_clock_lock.clone())
                .with_case(self.clone()),
        )
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.inner.cancel.clone()
    }

    /// Changes on restart. Old session readers should stop when their server
    /// epoch ends, and attach again using the newly authenticated client.
    pub async fn server_cancellation_token(&self) -> CancellationToken {
        self.inner.server_cancel.read().await.clone()
    }

    pub async fn client(&self) -> AgenaClient {
        self.inner
            .connection
            .read()
            .await
            .as_ref()
            .expect("TestCase is returned only after a successful connection")
            .client
            .clone()
    }

    pub async fn http(&self) -> TestHttpClient {
        self.inner
            .connection
            .read()
            .await
            .as_ref()
            .expect("TestCase is returned only after a successful connection")
            .http
            .clone()
    }

    pub async fn endpoint(&self) -> String {
        self.inner
            .connection
            .read()
            .await
            .as_ref()
            .expect("TestCase is returned only after a successful connection")
            .endpoint
            .clone()
    }

    pub async fn process_id(&self) -> Option<u32> {
        self.inner
            .process
            .lock()
            .await
            .as_ref()
            .filter(|process| process.is_owned())
            .map(|process| process.pid)
    }

    pub async fn last_exit(&self) -> Option<ProcessExit> {
        self.inner
            .process
            .lock()
            .await
            .as_ref()
            .and_then(|process| process.last_report.clone())
    }

    pub fn settings(&self) -> SettingsDriver {
        SettingsDriver::new(self.clone())
    }

    pub async fn config_snapshot(&self) -> Result<ConfigSnapshot> {
        self.settings().snapshot().await
    }

    pub async fn write_config(
        &self,
        layer: SettingsLayer,
        source: impl Into<ConfigFile>,
    ) -> Result<()> {
        let path = match layer {
            SettingsLayer::Global => &self.paths().global_config,
            SettingsLayer::Workspace => &self.paths().workspace_config,
        };
        let relative = path.strip_prefix(&self.paths().root)?;
        let path = owned_path(&self.paths().root, relative)?;
        source.into().write(&path).await
    }

    pub async fn seed_file(
        &self,
        relative: impl AsRef<Path>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<PathBuf> {
        let path = owned_path(&self.paths().root, relative.as_ref())?;
        write_bytes(&path, bytes.as_ref()).await?;
        Ok(path)
    }

    /// Wait for an actual, exact filesystem side effect in the owned case
    /// directory. Unlike terminal echoes or model text, this verifies the
    /// producer has completed its write. Content checks are byte-exact and
    /// both absence and stale content are retried for a bounded interval.
    pub async fn wait_file_bytes(
        &self,
        relative: impl AsRef<Path>,
        expected: impl AsRef<[u8]>,
    ) -> Result<PathBuf> {
        let path = owned_path(&self.paths().root, relative.as_ref())?;
        let expected = expected.as_ref().to_vec();
        let limit = self.timeout().min(Duration::from_secs(5));
        tokio::time::timeout(limit, async {
            loop {
                match tokio::fs::read(&path).await {
                    Ok(bytes) if bytes == expected => return Ok::<(), std::io::Error>(()),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .with_context(|| {
            format!(
                "expected {} to contain exactly {} bytes, but it did not settle within {:?}",
                path.display(),
                expected.len(),
                limit,
            )
        })??;
        Ok(path)
    }

    /// Register fixtures, readers, or callback tasks so their errors and panics
    /// participate in test cleanup. Tasks should observe cancellation_token().
    pub fn spawn_task(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
    ) -> AbortHandle {
        self.track_task(tokio::spawn(
            OWNED_TASK_CASE.scope(Arc::as_ptr(&self.inner) as usize, future),
        ))
    }

    /// Transfer an already spawned task to the case. Its error/panic becomes
    /// visible immediately, just as with spawn_task; cleanup also joins it.
    pub fn track_task(&self, task: JoinHandle<Result<()>>) -> AbortHandle {
        let task = self.own_task(task, "observer");
        let abort = task.abort.clone();
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.inner.cancel.is_cancelled() {
            task.abort();
        }
        tasks.push(task);
        abort
    }

    /// External HTTP/MCP/plugin fixtures stay online through Agena's graceful
    /// shutdown. Their token is cancelled only after the product has stopped.
    pub fn service_cancellation_token(&self) -> CancellationToken {
        self.inner.service_cancel.clone()
    }

    pub fn spawn_service(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
    ) -> AbortHandle {
        self.track_service(tokio::spawn(
            OWNED_TASK_CASE.scope(Arc::as_ptr(&self.inner) as usize, future),
        ))
    }

    /// Transfer a running fixture service and observe its failure immediately.
    pub fn track_service(&self, task: JoinHandle<Result<()>>) -> AbortHandle {
        let task = self.own_task(task, "service");
        let abort = task.abort.clone();
        let mut services = self
            .inner
            .services
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.inner.service_cancel.is_cancelled() {
            task.abort();
        }
        services.push(task);
        abort
    }

    fn own_task(&self, task: JoinHandle<Result<()>>, kind: &'static str) -> OwnedTask {
        let abort = task.abort_handle();
        let task = AbortOnDropTask(task);
        let future = async move {
            task.await
                .with_context(|| format!("owned {kind} task panicked or was cancelled"))?
        };
        OwnedTask {
            abort,
            join: tokio::spawn(self.observe_task(future, kind)),
        }
    }

    /// A weak reporter lets fixture-local workers surface failures without
    /// making their router/task registry hold the whole TestCase alive.
    pub(crate) fn task_failure_reporter(
        &self,
        kind: &'static str,
    ) -> Arc<dyn Fn(String) + Send + Sync> {
        let weak = Arc::downgrade(&self.inner);
        Arc::new(move |error| {
            if let Some(inner) = weak.upgrade() {
                inner
                    .failure
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .get_or_insert_with(|| format!("owned {kind} task failed: {error}"));
                inner.fatal.cancel();
            }
        })
    }

    fn observe_task(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
        kind: &'static str,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let report = self.task_failure_reporter(kind);
        async move {
            let result = match std::panic::AssertUnwindSafe(future).catch_unwind().await {
                Ok(result) => result,
                Err(payload) => Err(anyhow!(
                    "owned {kind} task panicked: {}",
                    panic_message(payload.as_ref())
                )),
            };
            if let Err(error) = &result {
                report(format!("{error:#}"));
            }
            result
        }
    }

    /// Keep all case-owned files, including bounded diagnostic logs.
    pub fn retain_artifacts(&self) -> PathBuf {
        if let Some(temp) = self
            .inner
            .temp
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = temp.keep();
        }
        self.paths().root.clone()
    }

    async fn launch(&self) -> Result<()> {
        let epoch = self.inner.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        // The previous owned process has already stopped before restart. A
        // stale discovery file must never be treated as a live server.
        match tokio::fs::remove_file(&self.paths().endpoint_record).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("removing owned stale discovery record"),
        }
        *self.inner.server_cancel.write().await = CancellationToken::new();
        let spec = self.inner.spec.read().await.clone();
        let process = OwnedProcess::spawn(&spec, self.paths(), epoch)?;
        *self.inner.process.lock().await = Some(process);
        let model = self.model();
        let connection = tokio::select! {
            result = tokio::time::timeout(self.inner.startup_timeout, self.wait_ready()) =>
                result.context("Agena startup readiness deadline elapsed")??,
            error = model.failed() => return Err(error.context("model endpoint failed during startup")),
            error = self.failed() => return Err(error.context("owned fixture failed during startup")),
        };
        *self.inner.connection.write().await = Some(connection);
        if let Some(process) = self.inner.process.lock().await.as_mut() {
            process.expect_running = true;
        }
        Ok(())
    }

    async fn wait_ready(&self) -> Result<Connection> {
        let mut last_error = String::new();
        loop {
            let pid = {
                let mut process = self.inner.process.lock().await;
                let process = process.as_mut().context("owned process was not spawned")?;
                if let Some(status) = process.poll_exit()? {
                    bail!(
                        "Agena exited during startup: {status}; last readiness error: {last_error}"
                    );
                }
                process.pid
            };
            match tokio::fs::read(&self.paths().endpoint_record).await {
                Ok(bytes) => match serde_json::from_slice::<ServerEndpointRecord>(&bytes) {
                    Ok(record) if record.pid == pid => {
                        validate_server_url(&record.url)?;
                        ensure!(
                            record.schema == ServerEndpointRecord::SCHEMA,
                            "unsupported discovery schema"
                        );
                        let attempt = tokio::time::timeout(
                            Duration::from_secs(2),
                            self.connect_record(&record),
                        )
                        .await;
                        match attempt {
                            Ok(Ok(connection)) => {
                                let runtime = connection.client.runtime_status().await?;
                                ensure!(
                                    runtime.generation > 0,
                                    "runtime generation was not initialized"
                                );
                                if self.inner.require_session_runtime {
                                    ensure!(
                                        runtime.session_runtime_available,
                                        "session runtime is unavailable"
                                    );
                                }
                                let plugins = connection.client.plugin_statuses().await?;
                                for item in &plugins.items {
                                    if !self.inner.allow_failed_plugins {
                                        ensure!(
                                            item.status.state.as_ref() != "failed",
                                            "plugin {} failed to prepare: {:?}",
                                            item.status.plugin_id,
                                            item.status.last_failure
                                        );
                                    }
                                }
                                for expected in &self.inner.expected_plugins {
                                    let item = plugins
                                        .items
                                        .iter()
                                        .find(|item| item.status.plugin_id.to_string() == *expected)
                                        .with_context(|| {
                                            format!("expected plugin {expected} was not loaded")
                                        })?;
                                    ensure!(
                                        item.status.state.as_ref() == "running",
                                        "expected plugin {expected} is {}",
                                        item.status.state
                                    );
                                }
                                return Ok(connection);
                            }
                            Ok(Err(error)) => last_error = format!("{error:#}"),
                            Err(_) => {
                                last_error = "identity/login/readiness request timed out".to_owned()
                            }
                        }
                    }
                    Ok(record) => {
                        last_error = format!(
                            "discovery pid {} does not match owned pid {pid}",
                            record.pid
                        )
                    }
                    Err(error) => last_error = format!("reading discovery document: {error}"),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("reading owned discovery record"),
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn connect_record(&self, record: &ServerEndpointRecord) -> Result<Connection> {
        let password = self.inner.spec.read().await.password.clone();
        let client = AgenaClient::connect_server(&record.url, None, password.as_deref()).await?;
        let identity = client.server_identity().await?;
        ensure!(
            record.matches(&identity),
            "discovery record does not match the responding server identity"
        );
        let http =
            TestHttpClient::connect(&record.url, password.as_deref(), self.timeout()).await?;
        http.get("/readyz").await?.require_success()?;
        Ok(Connection {
            endpoint: record.url.clone(),
            client,
            http,
        })
    }

    fn start_monitor(&self) {
        let weak = Arc::downgrade(&self.inner);
        let cancellation = self.inner.monitor_cancel.clone();
        let monitor = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                }
                let Some(inner) = weak.upgrade() else { break };
                let mut process = inner.process.lock().await;
                let Some(process) = process.as_mut() else {
                    continue;
                };
                if !process.expect_running {
                    continue;
                }
                let failure = match process.poll_exit() {
                    Ok(Some(status)) => {
                        Some(format!("owned Agena process exited unexpectedly: {status}"))
                    }
                    Err(error) => Some(format!("monitoring owned Agena process: {error:#}")),
                    Ok(None) => None,
                };
                if let Some(failure) = failure {
                    *inner
                        .failure
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(failure);
                    inner.fatal.cancel();
                    break;
                }
            }
        });
        *self
            .inner
            .monitor
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(monitor);
    }

    async fn failed(&self) -> anyhow::Error {
        self.inner.fatal.cancelled().await;
        anyhow!(
            self.inner
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
                .unwrap_or_else(|| "owned Agena fixture failed".to_owned())
        )
    }

    /// Gracefully stop and restart the same data/workspace directories. Client
    /// and HTTP handles obtained afterwards target the newly verified identity.
    pub async fn restart(&self) -> Result<()> {
        self.restart_with_env(std::iter::empty::<(OsString, Option<OsString>)>())
            .await
    }

    /// Change only the next child environment and restart the same fixture.
    /// None removes a variable; the parent test process is never mutated.
    pub async fn restart_with_env<I, K>(&self, changes: I) -> Result<()>
    where
        I: IntoIterator<Item = (K, Option<OsString>)>,
        K: Into<OsString>,
    {
        let _lifecycle = self.inner.lifecycle.lock().await;
        ensure!(
            !self.inner.finishing.load(Ordering::Acquire),
            "case has already finished"
        );
        ensure!(
            !self.inner.expected_kernel_crash.load(Ordering::Acquire),
            "reap_expected_kernel_crash must complete before restart"
        );
        {
            let mut next = self.inner.spec.read().await.clone();
            for (name, value) in changes {
                let name = name.into();
                #[cfg(unix)]
                ensure!(
                    name != "TMUX_TMPDIR" && name != "TMUX",
                    "TMUX and TMUX_TMPDIR are reserved for the case-owned terminal server"
                );
                if let Some(value) = value {
                    next.environment.insert(name, value);
                } else {
                    next.environment.remove(&name);
                }
            }
            next.password = environment_password(&next.environment)?;
            *self.inner.spec.write().await = next;
        }
        self.inner.server_cancel.read().await.cancel();
        {
            let mut process = self.inner.process.lock().await;
            if let Some(process) = process.as_mut() {
                process
                    .stop(false, false, self.inner.shutdown_timeout)
                    .await?;
            }
        }
        if let Err(error) = self.launch().await {
            self.record_diagnostic(format!("restart failed: {error:#}"));
            self.retain_artifacts();
            return Err(error);
        }
        Ok(())
    }

    /// Kill the owned Unix process session or Windows job, preserving data for
    /// restart. This intentional crash does not trigger the unexpected-exit race.
    /// Unix descendants that create independent sessions are not contained by
    /// this kill; fixture-owned external services should use spawn_service.
    pub async fn crash(&self) -> Result<()> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        ensure!(
            !self.inner.finishing.load(Ordering::Acquire),
            "case has already finished"
        );
        self.inner.server_cancel.read().await.cancel();
        if let Some(process) = self.inner.process.lock().await.as_mut() {
            let stopped = process.stop(true, true, self.inner.shutdown_timeout).await;
            let tmux = process.cleanup_tmux().await;
            stopped?;
            tmux?;
        }
        Ok(())
    }

    /// Expect a deliberately injected kernel COMMIT crash. Disable the normal
    /// unexpected-exit monitor before arming the real SQLite failpoint. A test
    /// must await/reap this crash before calling `case.restart()`.
    pub async fn arm_expected_kernel_crash(&self) -> Result<()> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        ensure!(
            !self.inner.finishing.load(Ordering::Acquire),
            "case has already finished"
        );
        ensure!(
            !self.inner.expected_kernel_crash.load(Ordering::Acquire),
            "an expected kernel crash is already armed; reap it before arming another"
        );
        let faults = self.faults()?;
        // Hold the monitor's process lock across arming. A fast crash cannot
        // race the monitor, and a failed arm never disables exit monitoring.
        {
            let mut owned = self.inner.process.lock().await;
            let process = owned.as_mut().context("Agena process not owned")?;
            ensure!(
                process.expect_running && process.poll_exit()?.is_none(),
                "Agena is not running before expected crash"
            );
            faults
                .arm(
                    crate::fault::KERNEL_BEFORE_COMMIT,
                    crate::fault::FaultEffect::Crash,
                )
                .await?;
            process.expect_running = false;
            self.inner
                .expected_kernel_crash
                .store(true, Ordering::Release);
        }
        self.inner.server_cancel.read().await.cancel();
        Ok(())
    }

    /// Wait for the process's real unsuccessful exit, then clean up the owned
    /// process tree and its logs without marking that expected crash a failure.
    pub async fn reap_expected_kernel_crash(&self) -> Result<ProcessExit> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        ensure!(
            self.inner.expected_kernel_crash.load(Ordering::Acquire),
            "arm_expected_kernel_crash must precede crash observation"
        );
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let mut process = self.inner.process.lock().await;
                let child = process.as_mut().context("Agena process not owned")?;
                if let Some(status) = child.poll_exit()? {
                    ensure!(
                        !status.success(),
                        "Agena exited successfully instead of crashing: {status}"
                    );
                    let hit = self.faults()?.state().await?;
                    ensure!(
                        hit.point == crate::fault::KERNEL_BEFORE_COMMIT
                            && hit.effect == "crash"
                            && hit.hits > 0,
                        "Agena exited without hitting the armed kernel crash point"
                    );
                    let stopped = child.stop(false, true, self.inner.shutdown_timeout).await;
                    let tmux = child.cleanup_tmux().await;
                    let report = stopped
                        .context("reaping crashed Agena")?
                        .context("crashed Agena had no exit report")?;
                    tmux.context("cleaning tmux after kernel crash")?;
                    self.inner
                        .expected_kernel_crash
                        .store(false, Ordering::Release);
                    return Ok::<_, anyhow::Error>(report);
                }
                drop(process);
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("expected kernel.before_commit crash did not occur")?
    }

    /// Await fixture tasks, the product process tree, and the model endpoint.
    /// Cleanup errors fail the test, and repeated calls return the same result.
    /// Cancelling this wait does not cancel the case-owned cleanup operation.
    pub async fn finish(&self) -> Result<()> {
        ensure!(
            !OWNED_TASK_CASE
                .try_with(|case| *case == Arc::as_ptr(&self.inner) as usize)
                .unwrap_or(false),
            "an owned task cannot await its own case finish; finish from the test body"
        );
        self.finish_impl(false).await
    }

    async fn finish_impl(&self, allow_failed_exit: bool) -> Result<()> {
        let mut receiver = self.begin_finish(allow_failed_exit);
        loop {
            let result = receiver.borrow_and_update().clone();
            if let Some(result) = result {
                return result.map_err(anyhow::Error::msg);
            }
            receiver
                .changed()
                .await
                .context("case cleanup owner ended without a result")?;
        }
    }

    fn begin_finish(
        &self,
        allow_failed_exit: bool,
    ) -> tokio::sync::watch::Receiver<Option<std::result::Result<(), String>>> {
        let mut cached = self
            .inner
            .finish_wait
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(receiver) = cached.as_ref() {
            receiver.clone()
        } else {
            let (sender, receiver) = tokio::sync::watch::channel(None);
            *cached = Some(receiver.clone());
            self.inner.finishing.store(true, Ordering::Release);
            let case = self.clone();
            match tokio::runtime::Handle::try_current() {
                Ok(executor) => {
                    executor.spawn(async move {
                        let result = match std::panic::AssertUnwindSafe(
                            case.perform_finish(allow_failed_exit),
                        )
                        .catch_unwind()
                        .await
                        {
                            Ok(result) => result.map_err(|error| format!("{error:#}")),
                            Err(payload) => {
                                let message = format!(
                                    "case cleanup task panicked: {}",
                                    panic_message(payload.as_ref()),
                                );
                                case.record_diagnostic(message.clone());
                                case.retain_artifacts();
                                let _ = case.write_diagnostics().await;
                                Err(format!(
                                    "{message}\nAgena test diagnostics: {}",
                                    case.paths().diagnostics.display()
                                ))
                            }
                        };
                        sender.send_replace(Some(result));
                    });
                }
                Err(error) => {
                    case.retain_artifacts();
                    sender.send_replace(Some(Err(format!(
                        "case cleanup requires a Tokio executor: {error}",
                    ))));
                }
            }
            receiver
        }
    }

    async fn perform_finish(&self, allow_failed_exit: bool) -> Result<()> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let previous = self
            .inner
            .finish_result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(result) = previous {
            return result.map_err(anyhow::Error::msg);
        }
        let mut errors = Vec::new();
        // An armed fault that never fired is an untested failure path. Check
        // before stopping the product so teardown cannot conceal the miss.
        let fault_path = self.paths().root.join("control/kernel-fault.json");
        match tokio::fs::try_exists(&fault_path).await {
            Ok(true) => match tokio::fs::read(&fault_path).await
                .context("read fault state before case cleanup")
                .and_then(|raw| serde_json::from_slice::<crate::fault::FaultState>(&raw)
                    .context("decode fault state before case cleanup"))
            {
                Ok(state) if !state.point.is_empty() && state.remaining > 0 => errors.push(format!(
                    "unconsumed kernel fault arm {} ({}, effect {}, remaining {}); call wait_hit or explicitly clear a deliberately disarmed fault",
                    state.arm_id, state.point, state.effect, state.remaining
                )),
                Ok(state) if state.point == crate::fault::KERNEL_BEFORE_COMMIT
                    && state.effect == "hold" && state.hits > 0 && !state.released => errors.push(format!(
                    "kernel fault arm {} ({}) still holds an uncommitted transaction; release or clear it explicitly",
                    state.arm_id, state.point
                )),
                Ok(_) => {},
                Err(error) => errors.push(format!("kernel fault state: {error:#}")),
            },
            Ok(false) => {},
            Err(error) => errors.push(format!("inspect kernel fault state: {error:#}")),
        }
        if self.inner.expected_kernel_crash.load(Ordering::Acquire) {
            errors.push(
                "expected kernel crash was not observed and reaped before case cleanup".to_owned(),
            );
        }
        self.inner.cancel.cancel();
        self.inner.server_cancel.read().await.cancel();
        self.inner.monitor_cancel.cancel();
        let monitor = self
            .inner
            .monitor
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(mut monitor) = monitor {
            match tokio::time::timeout(Duration::from_secs(2), &mut monitor).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("process monitor: {error}")),
                Err(_) => {
                    monitor.abort();
                    let _ = monitor.await;
                    errors.push("process monitor did not stop".to_owned());
                }
            }
        }
        let tasks = std::mem::take(
            &mut *self
                .inner
                .tasks
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        errors.extend(join_owned_tasks(tasks, self.inner.shutdown_timeout, "observer").await);
        if let Some(process) = self.inner.process.lock().await.as_mut() {
            if let Err(error) = process
                .stop(false, allow_failed_exit, self.inner.shutdown_timeout)
                .await
            {
                errors.push(format!("Agena process cleanup: {error:#}"));
            }
            if let Err(error) = process.cleanup_tmux().await {
                errors.push(format!("owned tmux cleanup: {error:#}"));
            }
        }
        self.inner.service_cancel.cancel();
        let services = std::mem::take(
            &mut *self
                .inner
                .services
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        errors.extend(join_owned_tasks(services, self.inner.shutdown_timeout, "service").await);
        // A stopping service might have raced task registration. Such tasks
        // are immediately aborted by registration and still must be reaped.
        let late_tasks = std::mem::take(
            &mut *self
                .inner
                .tasks
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        errors.extend(
            join_owned_tasks(late_tasks, self.inner.shutdown_timeout, "late observer").await,
        );
        let late_services = std::mem::take(
            &mut *self
                .inner
                .services
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        errors.extend(
            join_owned_tasks(late_services, self.inner.shutdown_timeout, "late service").await,
        );
        if let Err(error) = self.model().assert_finished() {
            errors.push(format!("model expectations: {error:#}"));
        }
        if let Some(owner) = self.inner.model_owner.lock().await.take()
            && let Err(error) = owner.shutdown().await
        {
            errors.push(format!("model endpoint cleanup: {error:#}"));
        }
        for error in &errors {
            self.record_diagnostic(error.clone());
        }
        if !errors.is_empty() {
            self.retain_artifacts();
        }
        if let Err(error) = self.write_diagnostics().await {
            errors.push(format!("saving diagnostics: {error:#}"));
            self.retain_artifacts();
        }
        self.inner.finished.store(true, Ordering::Release);
        let result = if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{}\nAgena test diagnostics: {}",
                errors.join("\n"),
                self.paths().diagnostics.display()
            ))
        };
        *self
            .inner
            .finish_result
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(result.clone());
        result.map_err(anyhow::Error::msg)
    }

    fn record_diagnostic(&self, message: String) {
        self.inner
            .diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(message);
    }

    async fn write_diagnostics(&self) -> Result<()> {
        let notes = self
            .inner
            .diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .join("\n\n");
        tokio::fs::write(self.paths().diagnostics.join("case.txt"), notes).await?;
        tokio::fs::write(
            self.paths().diagnostics.join("model.txt"),
            format!(
                "Requests:\n{:#?}\nFailures:\n{:#?}",
                self.model().requests(),
                self.model().failures()
            ),
        )
        .await?;
        Ok(())
    }

    fn failure_with_cleanup(&self, error: anyhow::Error, cleanup: Result<()>) -> anyhow::Error {
        let cleanup = cleanup
            .err()
            .map(|error| format!("\nCleanup also failed: {error:#}"))
            .unwrap_or_default();
        anyhow!(
            "{error:#}{cleanup}\nAgena test diagnostics: {}",
            self.paths().diagnostics.display()
        )
    }
}

pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload")
}

async fn join_owned_tasks(tasks: Vec<OwnedTask>, timeout: Duration, kind: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let aborts = tasks
        .iter()
        .map(|task| task.abort.clone())
        .collect::<Vec<_>>();
    // Consume each completed JoinHandle exactly once. A timed-out join_all over
    // mutable references loses already collected outputs, and polling those
    // consumed handles again can panic before the rest of cleanup runs.
    let mut pending = tasks.into_iter().collect::<FuturesUnordered<_>>();
    let drain = async {
        while let Some(result) = pending.next().await {
            collect_task_result(&mut errors, result, kind);
        }
    };
    if tokio::time::timeout(timeout, drain).await.is_err() {
        for abort in aborts {
            abort.abort();
        }
        errors.push(format!(
            "owned {kind} tasks did not stop before their cleanup deadline"
        ));
        let reap = async {
            while let Some(result) = pending.next().await {
                collect_task_result(&mut errors, result, kind);
            }
        };
        if tokio::time::timeout(Duration::from_secs(2), reap)
            .await
            .is_err()
        {
            errors.push(format!("aborted {kind} tasks did not yield for reaping"));
        }
    }
    errors
}

fn collect_task_result(
    errors: &mut Vec<String>,
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
    kind: &str,
) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => errors.push(format!("owned {kind} task: {error:#}")),
        Err(error) => errors.push(format!(
            "owned {kind} task panicked or was cancelled: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn prepared() -> Result<TestCase> {
        TestAgena::builder(std::env::current_exe()?)
            .raw()
            .shutdown_timeout(Duration::from_millis(100))
            .prepare()
            .await
    }

    #[tokio::test]
    async fn selecting_a_protocol_survives_later_resource_options() -> Result<()> {
        let case = TestAgena::builder(std::env::current_exe()?)
            .model_protocol(ModelProtocol::BedrockConverse)
            .model_protocol(ModelProtocol::Anthropic)
            .model_options(ModelOptions {
                default_timeout: Duration::from_secs(2),
                ..ModelOptions::default()
            })
            .prepare()
            .await?;
        let config: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
        assert_eq!(
            config["providers"]["default_selection"]["adapter"],
            "anthropic"
        );
        case.model()
            .expect(crate::RequestMatch::user_text("builder-protocol"))
            .reply(crate::Reply::text("configured-native-reply"));
        let body: serde_json::Value = reqwest::Client::new()
            .post(format!("{}/messages", case.model().base_url()))
            .json(&serde_json::json!({
                "model":"fake-model", "max_tokens":64, "stream":false,
                "messages":[{"role":"user","content":"builder-protocol"}]
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(body["content"][0]["text"], "configured-native-reply");
        case.finish().await
    }

    #[tokio::test]
    async fn initial_clock_precision_is_checked_before_opening_resources() -> Result<()> {
        let start = chrono::DateTime::from_timestamp(1_800_000_000, 1).context("test timestamp")?;
        let result = TestAgena::builder("this-binary-does-not-exist")
            .test_clock(start)
            .prepare()
            .await;
        let error = result
            .err()
            .context("submillisecond start unexpectedly prepared")?;
        assert!(
            error.to_string().contains("millisecond precision"),
            "{error:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn unsupported_product_protocol_fails_before_opening_resources() -> Result<()> {
        let result = TestAgena::builder("this-binary-does-not-exist")
            .model_protocol(ModelProtocol::BedrockConverse)
            .prepare()
            .await;
        let error = result
            .err()
            .context("unsupported product protocol unexpectedly prepared")?;
        assert!(
            error.to_string().contains("no product adapter"),
            "{error:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn bedrock_fixture_uses_only_explicit_local_signing_credentials() -> Result<()> {
        let case = TestAgena::builder(std::env::current_exe()?)
            .model_protocol(ModelProtocol::BedrockAnthropic)
            .prepare()
            .await?;
        let config: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
        let auth = &config["providers"]["testkit"]["auth"];
        assert_eq!(auth["subtype"], "bedrock_sigv4");
        assert_eq!(auth["access_key_id"], "agena-test-access-key");
        assert_eq!(auth["secret_access_key"], "agena-test-secret-key");
        let endpoint = url::Url::parse(auth["base_url"].as_str().context("local Bedrock URL")?)?;
        assert_eq!(endpoint.host_str(), Some("127.0.0.1"));
        assert_eq!(
            config["providers"]["default_selection"]["adapter"],
            "amazon_bedrock"
        );
        assert_eq!(
            config["providers"]["default_selection"]["model"],
            ModelProtocol::BedrockAnthropic.model_id()
        );
        case.finish().await
    }

    #[tokio::test]
    async fn unconsumed_fault_is_not_silently_accepted_during_case_cleanup() -> Result<()> {
        let case = TestAgena::builder(std::env::current_exe()?)
            .kernel_faults()
            .prepare()
            .await?;
        case.faults()?
            .arm(
                crate::fault::KERNEL_BEFORE_COMMIT,
                crate::fault::FaultEffect::Fail,
            )
            .await?;
        let err = case
            .finish()
            .await
            .expect_err("case accepted an unconsumed fault arm");
        ensure!(
            err.to_string().contains("unconsumed kernel fault arm"),
            "{err:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn clearing_an_intentionally_unused_fault_permits_case_cleanup() -> Result<()> {
        let case = TestAgena::builder(std::env::current_exe()?)
            .kernel_faults()
            .prepare()
            .await?;
        let faults = case.faults()?;
        faults
            .arm(
                crate::fault::KERNEL_BEFORE_COMMIT,
                crate::fault::FaultEffect::Fail,
            )
            .await?;
        faults.clear().await?;
        case.finish().await
    }

    #[tokio::test]
    async fn a_cancelled_server_token_does_not_authorize_reaping_an_injected_crash() -> Result<()> {
        let case = prepared().await?;
        case.inner.server_cancel.read().await.cancel();
        let error = case
            .reap_expected_kernel_crash()
            .await
            .expect_err("a cancelled token is not an armed crash");
        assert!(error.to_string().contains("must precede"));
        case.finish().await
    }

    #[derive(Clone, Copy)]
    enum SetupFailure {
        Error,
        Panic,
        Timeout,
    }

    async fn assert_before_start_cleanup(failure: SetupFailure) -> Result<()> {
        let captured = Arc::new(StdMutex::new(None::<TestCase>));
        let completed = Arc::new(AtomicBool::new(false));
        let captured_case = captured.clone();
        let completion = completed.clone();
        let builder = TestAgena::builder(std::env::current_exe()?)
            .raw()
            .startup_timeout(Duration::from_millis(50))
            .shutdown_timeout(Duration::from_secs(2))
            .before_start(move |case| async move {
                *captured_case.lock().unwrap() = Some(case.clone());
                let cancel = case.service_cancellation_token();
                case.spawn_service(async move {
                    cancel.cancelled().await;
                    tokio::task::yield_now().await;
                    completion.store(true, Ordering::Release);
                    Ok(())
                });
                match failure {
                    SetupFailure::Error => bail!("before_start fixture error"),
                    SetupFailure::Panic => panic!("before_start fixture panic"),
                    SetupFailure::Timeout => std::future::pending::<Result<()>>().await,
                }
            });
        let outcome = std::panic::AssertUnwindSafe(builder.prepare())
            .catch_unwind()
            .await;
        match (failure, outcome) {
            (SetupFailure::Error, Ok(Err(error))) => {
                assert!(format!("{error:#}").contains("before_start fixture error"));
            }
            (SetupFailure::Timeout, Ok(Err(error))) => {
                assert!(format!("{error:#}").contains("before_start setup deadline elapsed"));
            }
            (SetupFailure::Panic, Err(payload)) => {
                assert_eq!(
                    panic_message(payload.as_ref()),
                    "before_start fixture panic"
                );
            }
            _ => panic!("before_start did not preserve the expected failure"),
        }
        let case = captured.lock().unwrap().take().expect("hook was entered");
        assert!(
            completed.load(Ordering::Acquire),
            "setup failure must await owned service cleanup"
        );
        assert!(case.inner.finished.load(Ordering::Acquire));
        assert!(case.inner.model_owner.lock().await.is_none());
        assert!(case.process_id().await.is_none());
        assert!(case.paths().diagnostics.join("case.txt").exists());
        // Failed setup retains its artifacts. Remove only this regression's
        // isolated directory after checking that cleanup completed first.
        tokio::fs::remove_dir_all(&case.paths().root).await?;
        Ok(())
    }

    #[tokio::test]
    async fn before_start_error_awaits_owned_service_cleanup() -> Result<()> {
        assert_before_start_cleanup(SetupFailure::Error).await
    }

    #[tokio::test]
    async fn before_start_panic_awaits_owned_service_cleanup() -> Result<()> {
        assert_before_start_cleanup(SetupFailure::Panic).await
    }

    #[tokio::test]
    async fn before_start_timeout_awaits_owned_service_cleanup() -> Result<()> {
        assert_before_start_cleanup(SetupFailure::Timeout).await
    }

    #[tokio::test]
    async fn explicit_missing_source_and_removed_env_override_earlier_helpers() -> Result<()> {
        let case = TestAgena::builder(std::env::current_exe()?)
            .env_relative("AGENA_TESTKIT_FILE", "fixtures/secret")
            .env_remove("AGENA_TESTKIT_FILE")
            .configure_global(|_| panic!("a later complete source replaces this edit"))
            .global_config(ConfigFile::Missing)
            .prepare()
            .await?;
        assert!(!case.paths().global_config.exists());
        assert!(
            !case
                .inner
                .spec
                .read()
                .await
                .environment
                .contains_key(std::ffi::OsStr::new("AGENA_TESTKIT_FILE"))
        );
        case.finish().await?;
        Ok(())
    }

    #[tokio::test]
    async fn cleanup_awaits_observers_before_stopping_services() -> Result<()> {
        let case = prepared().await?;
        let order = Arc::new(StdMutex::new(Vec::new()));
        let cancel = case.cancellation_token();
        let observed = order.clone();
        case.spawn_task(async move {
            cancel.cancelled().await;
            tokio::task::yield_now().await;
            observed.lock().unwrap().push("observer");
            Ok(())
        });
        let cancel = case.service_cancellation_token();
        let observed = order.clone();
        case.spawn_service(async move {
            cancel.cancelled().await;
            observed.lock().unwrap().push("service");
            Ok(())
        });
        case.finish().await?;
        assert_eq!(*order.lock().unwrap(), vec!["observer", "service"]);
        case.finish().await?;
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_finish_waiter_does_not_cancel_owned_service_join() -> Result<()> {
        let case = TestAgena::builder(std::env::current_exe()?)
            .raw()
            .shutdown_timeout(Duration::from_secs(2))
            .prepare()
            .await?;
        let cancelled = case.service_cancellation_token();
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let completed = Arc::new(AtomicBool::new(false));
        let completion = completed.clone();
        case.spawn_service(async move {
            cancelled.cancelled().await;
            let _ = started.send(());
            released.await.context("release gated service")?;
            completion.store(true, Ordering::Release);
            Ok(())
        });
        let first = {
            let case = case.clone();
            tokio::spawn(async move { case.finish().await })
        };
        tokio::time::timeout(Duration::from_secs(1), observed).await??;
        first.abort();
        let _ = first.await;
        assert!(!completed.load(Ordering::Acquire));
        let second = {
            let case = case.clone();
            tokio::spawn(async move { case.finish().await })
        };
        tokio::task::yield_now().await;
        assert!(
            !second.is_finished(),
            "cleanup must still await the gated service"
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), second).await???;
        assert!(completed.load(Ordering::Acquire));
        assert!(case.inner.model_owner.lock().await.is_none());
        case.finish().await?;
        Ok(())
    }

    #[tokio::test]
    async fn cleanup_task_panic_and_timeout_are_failures_with_retained_artifacts() -> Result<()> {
        let case = prepared().await?;
        case.spawn_task(async {
            panic!("fixture task exploded");
            #[allow(unreachable_code)]
            Ok(())
        });
        case.spawn_service(std::future::pending());
        let error = case.finish().await.unwrap_err().to_string();
        assert!(error.contains("panicked"), "{error}");
        assert!(error.contains("cleanup deadline"), "{error}");
        assert!(case.paths().diagnostics.join("case.txt").exists());
        assert_eq!(case.finish().await.unwrap_err().to_string(), error);
        // This unit test already asserted retention. Remove only its own files.
        tokio::fs::remove_dir_all(&case.paths().root).await?;
        Ok(())
    }
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn mixed_completed_and_stalled_tasks_preserve_errors_and_finish_all_cleanup() -> Result<()>
    {
        let case = prepared().await?;
        let dropped = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        case.spawn_task(async { Ok(()) });
        case.spawn_task(async { bail!("completed-before-timeout-marker") });
        let guard = DropFlag(dropped.clone());
        let (entered, started) = tokio::sync::oneshot::channel();
        case.spawn_task(async move {
            let _guard = guard;
            let _ = entered.send(());
            std::future::pending::<Result<()>>().await
        });
        let cancel = case.service_cancellation_token();
        let completion = stopped.clone();
        case.spawn_service(async move {
            cancel.cancelled().await;
            completion.store(true, Ordering::Release);
            Ok(())
        });
        started.await?;
        let error = tokio::time::timeout(Duration::from_secs(2), case.finish())
            .await?
            .unwrap_err()
            .to_string();
        assert!(error.contains("completed-before-timeout-marker"), "{error}");
        assert!(error.contains("cleanup deadline"), "{error}");
        assert!(!error.contains("cleanup task panicked"), "{error}");
        assert!(dropped.load(Ordering::Acquire));
        assert!(
            stopped.load(Ordering::Acquire),
            "service teardown was skipped"
        );
        assert!(case.inner.finished.load(Ordering::Acquire));
        assert!(case.inner.model_owner.lock().await.is_none());
        assert_eq!(case.finish().await.unwrap_err().to_string(), error);
        tokio::fs::remove_dir_all(&case.paths().root).await?;
        Ok(())
    }

    #[tokio::test]
    async fn tracked_existing_tasks_report_errors_and_panics_before_finish() -> Result<()> {
        for service in [false, true] {
            let case = prepared().await?;
            let task = tokio::spawn(async move {
                if service {
                    panic!("tracked-service-panic-marker");
                }
                bail!("tracked-observer-error-marker")
            });
            if service {
                case.track_service(task);
            } else {
                case.track_task(task);
            }
            let marker = if service {
                "tracked-service-panic-marker"
            } else {
                "tracked-observer-error-marker"
            };
            let failure = tokio::time::timeout(Duration::from_secs(2), case.failed()).await?;
            assert!(format!("{failure:#}").contains(marker), "{failure:#}");
            let cleanup = case.finish().await.unwrap_err().to_string();
            assert!(cleanup.contains(marker), "{cleanup}");
            assert!(case.inner.model_owner.lock().await.is_none());
            tokio::fs::remove_dir_all(&case.paths().root).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn owned_workers_reject_waiting_for_their_own_case_finish() -> Result<()> {
        for service in [false, true] {
            let case = prepared().await?;
            let retained = case.clone();
            let (reported, report) = tokio::sync::oneshot::channel();
            let future = async move {
                let result = retained.finish().await;
                let _ = reported.send(result.map_err(|error| format!("{error:#}")));
                Ok(())
            };
            if service {
                case.spawn_service(future);
            } else {
                case.spawn_task(future);
            }
            let failure = tokio::time::timeout(Duration::from_secs(2), report)
                .await??
                .expect_err("a worker cannot join itself");
            assert!(
                failure.contains("cannot await its own case finish"),
                "{failure}"
            );
            case.finish().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_preparation_drains_services_and_routers_that_retain_case_clones()
    -> Result<()> {
        let (ready, prepared_case) = tokio::sync::oneshot::channel();
        let (completed, completion) = tokio::sync::oneshot::channel();
        let builder = TestAgena::builder(std::env::current_exe()?)
            .raw()
            .startup_timeout(Duration::from_secs(10))
            .shutdown_timeout(Duration::from_secs(2))
            .before_start(move |case| async move {
                let retained = case.clone();
                let cancel = case.service_cancellation_token();
                case.spawn_service(async move {
                    let _retained = retained;
                    cancel.cancelled().await;
                    let _ = completed.send(());
                    Ok(())
                });
                let router_case = case.clone();
                let router = axum::Router::new().route(
                    "/held-case",
                    axum::routing::get(move || {
                        let retained = router_case.clone();
                        async move { retained.workspace().display().to_string() }
                    }),
                );
                let server = case.serve_http(router).await?;
                let _ = ready.send((case, server.address()));
                std::future::pending::<Result<()>>().await
            });
        let owner = tokio::spawn(builder.prepare());
        let (case, address) = tokio::time::timeout(Duration::from_secs(2), prepared_case).await??;
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()?
            .get(format!("http://{address}/held-case"))
            .send()
            .await?;
        assert!(response.status().is_success());
        drop(response);
        owner.abort();
        match owner.await {
            Err(error) => assert!(error.is_cancelled()),
            Ok(_) => panic!("cancelled preparation unexpectedly completed"),
        }
        // Wait for the guard to initiate teardown before explicitly joining it.
        // Merely calling finish here would hide the external-cancellation bug.
        tokio::time::timeout(Duration::from_secs(2), completion).await??;
        tokio::time::timeout(Duration::from_secs(2), case.finish()).await??;
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
        assert!(case.inner.finished.load(Ordering::Acquire));
        assert!(case.inner.model_owner.lock().await.is_none());
        let diagnostic =
            tokio::fs::read_to_string(case.paths().diagnostics.join("case.txt")).await?;
        assert!(diagnostic.contains("future was cancelled"), "{diagnostic}");
        tokio::fs::remove_dir_all(&case.paths().root).await?;
        Ok(())
    }
}
