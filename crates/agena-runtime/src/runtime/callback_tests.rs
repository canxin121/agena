use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agena_plugin_host::config::{ConfiguredPlugin, HttpAuth, PluginPackage, PluginsConfig};
use agena_plugin_sdk::host_api::{HostClient, NoopHostClient};
use agena_plugin_sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, ProviderListInput,
    ProviderListPatch, ToolInvokeInput, ToolInvokeOutput,
};
use serde_json::{Value, json};

use super::AgenaRuntime;

mod client_identity_tests;
mod reload_shutdown_tests;
mod shutdown_notification_tests;

const KEY: &str = "test.runtime-callback";
const WAIT: Duration = Duration::from_secs(15);
static RUNTIMES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Default)]
struct Observed {
    init_config: Mutex<Vec<Value>>,
    host: Mutex<Option<Arc<dyn HostClient>>>,
    contexts: Mutex<Vec<InitContext>>,
    provider_error: AtomicBool,
    provider_wait: AtomicBool,
    provider_entered: tokio::sync::Notify,
    provider_release: tokio::sync::Notify,
    shutdowns: AtomicUsize,
    init_readiness_error: Mutex<Option<agena_plugin_sdk::PluginError>>,
    reload_entered: tokio::sync::Notify,
    reload_release: tokio::sync::Notify,
    reload_request: Mutex<Option<agena_plugin_sdk::host_api::HostConfigReloadRequestResponse>>,
    prompt_entered: tokio::sync::Notify,
    prompt_release: tokio_util::sync::CancellationToken,
    session_ends: Mutex<Vec<agena_plugin_sdk::SessionEndInput>>,
    session_end_entered: tokio::sync::Notify,
    session_end_release: tokio_util::sync::CancellationToken,
}

struct CallbackPlugin(Arc<Observed>);

#[async_trait::async_trait]
impl Plugin for CallbackPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("test", "runtime-callback", "1.0.0");
        manifest.hooks |= HookSubscription::PROVIDER_LIST
            | HookSubscription::SHELL_ENV
            | HookSubscription::CHAT_PARAMS
            | HookSubscription::USER_PROMPT_SUBMIT
            | HookSubscription::SESSION_END;
        manifest.settings = Some(agena_plugin_sdk::SettingsContract::bounded_json(
            "Settings",
            "test revision",
            1024,
            8,
        ));
        manifest.tools.push(
            serde_json::from_value(json!({
                "name": "reload", "docs": {"summary": "Reload the owning runtime."},
                "contract": {"input_schema": {"type": "object"}}
            }))
            .unwrap(),
        );
        manifest
    }

    async fn init(
        &self,
        context: InitContext,
        host: Arc<dyn HostClient>,
    ) -> agena_plugin_sdk::Result<InitOutcome> {
        let config = host.read_config(Some("config.ui.locale".into())).await?;
        self.0.init_config.lock().unwrap().push(config);
        self.0.contexts.lock().unwrap().push(context);
        *self.0.init_readiness_error.lock().unwrap() = host.request_config_reload().await.err();
        *self.0.host.lock().unwrap() = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn provider_list(
        &self,
        _: ProviderListInput,
    ) -> agena_plugin_sdk::Result<Option<ProviderListPatch>> {
        if self.0.provider_wait.load(Ordering::SeqCst) {
            self.0.provider_entered.notify_one();
            self.0.provider_release.notified().await;
        }
        if self.0.provider_error.load(Ordering::SeqCst) {
            return Err(agena_plugin_sdk::PluginError::internal(
                "injected provider composition failure",
            ));
        }
        Ok(None)
    }

    async fn shutdown(&self) -> agena_plugin_sdk::Result<()> {
        self.0.shutdowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn user_prompt_submit(
        &self,
        _: agena_plugin_sdk::UserPromptSubmitInput,
    ) -> agena_plugin_sdk::Result<Option<agena_plugin_sdk::UserPromptSubmitPatch>> {
        self.0.prompt_entered.notify_one();
        self.0.prompt_release.cancelled().await;
        Ok(None)
    }

    async fn session_end(
        &self,
        input: agena_plugin_sdk::SessionEndInput,
    ) -> agena_plugin_sdk::Result<()> {
        self.0.session_ends.lock().unwrap().push(input);
        self.0.session_end_entered.notify_one();
        self.0.session_end_release.cancelled().await;
        Ok(())
    }

    async fn shell_env(
        &self,
        _: agena_plugin_sdk::ShellEnvInput,
    ) -> agena_plugin_sdk::Result<Option<agena_plugin_sdk::ShellEnvPatch>> {
        self.waiting_hook_reload().await?;
        Ok(None)
    }

    async fn chat_params(
        &self,
        _: agena_plugin_sdk::ChatParamsInput,
    ) -> agena_plugin_sdk::Result<Option<agena_plugin_sdk::ChatParamsPatch>> {
        self.waiting_hook_reload().await?;
        Ok(None)
    }

    async fn tool_invoke(
        &self,
        input: ToolInvokeInput,
    ) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
        let host = self.0.host.lock().unwrap().clone().unwrap();
        let result = if input.input["nested"] == true {
            let output = host
                .invoke_tool(
                    "agena.settings.set".into(),
                    json!({
                        "layer": "global", "path": "ui.locale", "value": "zh-CN", "reload": true
                    }),
                )
                .await?;
            serde_json::from_value(output.payload.unwrap()["reload_task"].clone()).unwrap()
        } else {
            host.request_config_reload().await?
        };
        if input.input["repeat"] == true {
            let repeated = host.request_config_reload().await?;
            assert!(!repeated.started);
            assert_eq!(repeated.task_id, result.task_id);
        }
        *self.0.reload_request.lock().unwrap() = Some(result.clone());
        if input.input["wait"] == true {
            self.0.reload_entered.notify_one();
            self.0.reload_release.notified().await;
        }
        Ok(ToolInvokeOutput::text(
            serde_json::to_string(&result).unwrap(),
        ))
    }

    async fn tool_invoke_stream(
        &self,
        input: ToolInvokeInput,
        sink: agena_plugin_sdk::ToolStreamSink,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::ToolStreamEnd> {
        let host = self.0.host.lock().unwrap().clone().unwrap();
        let accepted = host.request_config_reload().await?;
        *self.0.reload_request.lock().unwrap() = Some(accepted.clone());
        sink.text("queued").await;
        self.0.reload_entered.notify_one();
        self.0.reload_release.notified().await;
        let repeated = host.request_config_reload().await?;
        assert!(!repeated.started);
        assert_eq!(repeated.task_id, accepted.task_id);
        assert_eq!(input.tool_name, "reload");
        sink.text("complete").await;
        Ok(agena_plugin_sdk::ToolStreamEnd::text(
            sink.stream_id(),
            serde_json::to_string(&accepted).unwrap(),
        ))
    }
}

impl CallbackPlugin {
    async fn waiting_hook_reload(&self) -> agena_plugin_sdk::Result<()> {
        let context =
            agena_plugin_sdk::host_api::current_host_callback_context().unwrap_or_default();
        assert!(
            context.authority_token.is_some(),
            "a request-response hook must retain its originating call authority"
        );
        assert!(context.session_id.is_none());
        assert!(context.workspace_root.is_none());
        let host = self.0.host.lock().unwrap().clone().unwrap();
        let accepted = host.request_config_reload().await?;
        *self.0.reload_request.lock().unwrap() = Some(accepted.clone());
        self.0.reload_entered.notify_one();
        self.0.reload_release.notified().await;
        let repeated = host.request_config_reload().await?;
        assert!(!repeated.started);
        assert_eq!(repeated.task_id, accepted.task_id);
        Ok(())
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
    plugins: PluginsConfig,
    observed: Arc<Observed>,
    runtime: Option<Arc<AgenaRuntime>>,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(runtime) = &self.runtime {
            runtime.shutdown();
        }
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let serial = RUNTIMES.lock().await;
        let observed = Arc::new(Observed::default());
        let factory_observed = observed.clone();
        let router = agena_plugin_sdk::drivers::http::router(
            move || CallbackPlugin(factory_observed.clone()),
            Arc::new(NoopHostClient),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/rpc", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut plugins = PluginsConfig::default();
        plugins.list.insert(
            KEY.into(),
            ConfiguredPlugin {
                package: PluginPackage::Http {
                    url: url.parse().unwrap(),
                    auth: HttpAuth::None,
                },
                ..Default::default()
            },
        );
        Self {
            directory: tempfile::tempdir().unwrap(),
            server,
            plugins,
            observed,
            runtime: None,
            _serial: serial,
        }
    }

    fn write_config(&self, locale: &str) {
        std::fs::write(
            self.directory.path().join("config.json"),
            serde_json::to_vec(&json!({
                "ui": {"locale": locale},
                "plugins": self.plugins,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn composition_config(&self) -> crate::RuntimeCompositionConfig {
        let path = self.directory.path().to_path_buf();
        crate::RuntimeCompositionConfig {
            load_request: crate::LoadConfigRequest {
                config_path: Some(path.join("config.json")),
                workspace_root: Some(path.clone()),
                ..Default::default()
            },
            workspace_root: Some(path),
            bootstrap_preflight: None,
            database_connection: None,
            database_url: Some("sqlite::memory:".into()),
            database_path: None,
            scheduler_database_connection: None,
            scheduler_database_url: Some("sqlite::memory:".into()),
            scheduler_database_path: None,
            initialize_schema: true,
            tracing_reload_handle: None,
        }
    }

    async fn start(&mut self) -> Arc<AgenaRuntime> {
        self.start_with_maintenance(false).await
    }

    async fn start_with_maintenance(&mut self, maintain: bool) -> Arc<AgenaRuntime> {
        let runtime = tokio::time::timeout(
            WAIT,
            AgenaRuntime::new_with_maintenance(self.composition_config(), maintain),
        )
        .await
        .expect("runtime bootstrap finishes")
        .expect("isolated runtime builds");
        self.runtime = Some(runtime.clone());
        runtime
    }

    async fn read_config(&self) -> Value {
        let host = self
            .observed
            .host
            .lock()
            .unwrap()
            .clone()
            .expect("initialized HTTP plugin");
        tokio::time::timeout(WAIT, host.read_config(Some("config.ui.locale".into())))
            .await
            .unwrap()
            .unwrap()
    }
}

#[tokio::test]
async fn production_http_plugin_can_read_candidate_config_during_initialization() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    assert_eq!(
        *fixture.observed.init_config.lock().unwrap(),
        vec![json!("en-US")],
        "production composition must serve first-init callbacks before publishing the runtime"
    );
    let error = fixture
        .observed
        .init_readiness_error
        .lock()
        .unwrap()
        .clone()
        .expect("runtime-dependent init callback is rejected");
    assert!(error.to_string().contains("runtime services are not ready"));
    assert_eq!(fixture.read_config().await, json!("en-US"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn runtime_callback_adapters_do_not_retain_the_runtime_after_shutdown() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let inner = Arc::downgrade(&runtime.inner);
    let host = runtime.current_snapshot().plugin_manager().host_handle();
    let client = runtime.current_snapshot().host_client.clone();
    let retained_clone = runtime.as_ref().clone();
    runtime.current_snapshot().plugin_manager().shutdown().await;
    fixture.runtime.take();
    drop(runtime);
    assert!(
        client.plugin_status_list().await.is_ok(),
        "the weak binding survives a cloned runtime handle even after its original outer Arc drops"
    );
    drop(retained_clone);
    tokio::time::timeout(Duration::from_secs(2), async {
        while inner.upgrade().is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a retained HostHandle/client/listener must not retain the stopped runtime");
    assert!(
        client.plugin_status_list().await.is_err(),
        "released runtime services must report unavailability"
    );
    drop(host);
}

#[tokio::test]
async fn production_reload_rebinds_config_without_reinitializing_the_http_plugin() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let old = runtime.current_snapshot();
    let old_handle = old.plugin_manager().host_handle();
    let old_token = old_handle.callback_token(KEY).await.unwrap();
    let old_url = old_handle.callback_url(KEY).unwrap();
    fixture.write_config("zh-CN");
    let report = tokio::time::timeout(WAIT, runtime.reload())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((report.previous_generation, report.generation), (1, 2));
    assert_eq!(fixture.observed.init_config.lock().unwrap().len(), 1);
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    assert_eq!(
        runtime
            .current_snapshot()
            .plugin_manager()
            .host_handle()
            .callback_url(KEY)
            .unwrap(),
        old_url
    );
    assert!(
        !old_handle
            .validate_callback_token(KEY, Some(&old_token))
            .await
    );
    old.plugin_manager().shutdown().await;
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn production_restart_reads_candidate_config_before_snapshot_publication() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let old = runtime.current_snapshot();
    fixture.plugins.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.write_config("zh-CN");
    fixture.observed.provider_wait.store(true, Ordering::SeqCst);
    let reload_runtime = runtime.clone();
    let reload = tokio::spawn(async move { reload_runtime.reload().await });
    tokio::time::timeout(WAIT, fixture.observed.provider_entered.notified())
        .await
        .unwrap();
    assert_eq!(
        *fixture.observed.init_config.lock().unwrap(),
        vec![json!("en-US"), json!("zh-CN")]
    );
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert!(Arc::ptr_eq(
        &crate::current_plugin_host().unwrap(),
        &old.plugin_manager()
    ));
    fixture
        .observed
        .provider_wait
        .store(false, Ordering::SeqCst);
    fixture.observed.provider_release.notify_one();
    tokio::time::timeout(WAIT, reload)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    old.plugin_manager().shutdown().await;
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

async fn wait_for_cleanup(observed: &Observed) {
    tokio::time::timeout(WAIT, async {
        while observed.shutdowns.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("completed plugin initialization must be cleaned up after a failed build");
    let url = observed.contexts.lock().unwrap()[0]
        .host_callback_url
        .clone()
        .unwrap();
    let endpoint = url::Url::parse(&url).unwrap();
    tokio::time::timeout(WAIT, async {
        while tokio::net::TcpStream::connect(("127.0.0.1", endpoint.port().unwrap()))
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("an unpublished runtime must release its callback listener");
}

#[tokio::test]
async fn failed_runtime_service_composition_shuts_down_initialized_plugins_and_listener() {
    let fixture = Fixture::new().await;
    fixture.write_config("en-US");
    fixture
        .observed
        .provider_error
        .store(true, Ordering::SeqCst);
    let result = tokio::time::timeout(WAIT, AgenaRuntime::new(fixture.composition_config()))
        .await
        .unwrap();
    assert!(
        matches!(result, Err(error) if error.to_string().contains("injected provider composition failure"))
    );
    wait_for_cleanup(&fixture.observed).await;
}

#[tokio::test]
async fn cancelled_runtime_service_composition_shuts_down_initialized_plugins_and_listener() {
    let fixture = Fixture::new().await;
    fixture.write_config("en-US");
    fixture.observed.provider_wait.store(true, Ordering::SeqCst);
    let build = tokio::spawn(AgenaRuntime::new(fixture.composition_config()));
    tokio::time::timeout(WAIT, fixture.observed.provider_entered.notified())
        .await
        .unwrap();
    build.abort();
    assert!(matches!(build.await, Err(error) if error.is_cancelled()));
    wait_for_cleanup(&fixture.observed).await;
}

async fn invoke_named(
    runtime: &AgenaRuntime,
    plugin: &str,
    tool: &str,
    input: Value,
) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
    invoke_in_session(runtime, plugin, tool, input, -1).await
}

async fn invoke_in_session(
    runtime: &AgenaRuntime,
    plugin: &str,
    tool: &str,
    input: Value,
    session_id: i64,
) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
    let host = runtime.current_snapshot().plugin_manager();
    let registered = host
        .registered_tools()
        .into_iter()
        .find(|candidate| candidate.canonical_name() == format!("{plugin}.{tool}"))
        .expect("registered fixture tool");
    tokio::time::timeout(
        WAIT,
        host.invoke_tool(
            &registered,
            ToolInvokeInput {
                tool_name: tool.into(),
                session_id,
                call_id: 1,
                workspace_root: runtime.inner.workspace_root.to_string_lossy().into(),
                input,
            },
            None,
        ),
    )
    .await
    .expect("plugin-triggered reload must settle within a deadline")
}

#[tokio::test]
async fn http_plugin_self_reload_returns_success_and_keeps_the_successor_usable() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    fixture.write_config("zh-CN");
    let result = invoke_named(&runtime, KEY, "reload", json!({})).await;
    assert!(
        result.is_ok(),
        "self reload must not cancel its own invocation: {result:?}"
    );
    let accepted: agena_plugin_sdk::host_api::HostConfigReloadRequestResponse =
        serde_json::from_str(&result.unwrap().output_text).unwrap();
    wait_reload(&runtime, &accepted.task_id).await;
    let client = fixture.observed.host.lock().unwrap().clone().unwrap();
    assert!(matches!(
        client
            .config_reload_status(agena_plugin_sdk::host_api::HostConfigReloadStatusRequest {
                task_id: accepted.task_id
            })
            .await
            .unwrap()
            .state,
        agena_plugin_sdk::host_api::HostConfigReloadState::Succeeded {}
    ));
    assert_eq!(runtime.current_snapshot().generation(), 2);
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn bundled_settings_reload_returns_success_and_publishes_written_configuration() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let result = invoke_named(
        &runtime,
        "agena.settings",
        "set",
        json!({
            "layer": "global", "path": "ui.locale", "value": "zh-CN", "reload": true
        }),
    )
    .await;
    assert!(
        result.is_ok(),
        "bundled settings write/reload must settle successfully: {result:?}"
    );
    let payload = result.unwrap().payload.unwrap();
    assert!(
        payload["reload"].is_null(),
        "queue acceptance must not be a completed-generation report"
    );
    let task_id = payload["reload_task"]["task_id"].as_str().unwrap();
    wait_reload(&runtime, task_id).await;
    assert_eq!(runtime.current_snapshot().generation(), 2);
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

async fn wait_reload(runtime: &AgenaRuntime, task_id: &str) {
    tokio::time::timeout(WAIT, async {
        loop {
            let task = runtime
                .background_tasks()
                .into_iter()
                .find(|task| task.id == task_id)
                .expect("accepted task remains visible");
            if !task.is_running() {
                assert_eq!(
                    task.status,
                    crate::RuntimeBackgroundTaskStatus::Succeeded,
                    "reload must finish successfully: {task:?}"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("accepted reload must settle after its caller returns");
}

#[tokio::test]
async fn self_reload_waits_for_the_originating_http_call_and_deduplicates_its_requests() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let caller = runtime.clone();
    let invocation = tokio::spawn(async move {
        invoke_named(
            &caller,
            KEY,
            "reload",
            json!({"wait": true, "repeat": true}),
        )
        .await
    });
    tokio::time::timeout(WAIT, fixture.observed.reload_entered.notified())
        .await
        .unwrap();
    let accepted = fixture
        .observed
        .reload_request
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert!(matches!(
        runtime
            .current_snapshot()
            .host_client
            .config_reload_status(agena_plugin_sdk::host_api::HostConfigReloadStatusRequest {
                task_id: accepted.task_id.clone()
            })
            .await
            .unwrap()
            .state,
        agena_plugin_sdk::host_api::HostConfigReloadState::Running {}
    ));
    fixture.observed.reload_release.notify_one();
    assert!(invocation.await.unwrap().is_ok());
    wait_reload(&runtime, &accepted.task_id).await;
    assert_eq!(runtime.current_snapshot().generation(), 2);
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn nested_bundled_settings_reload_waits_for_the_outer_http_call() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let session = runtime
        .session_manager()
        .unwrap()
        .create_session(agena_runtime_session::SessionCreateRequest {
            title: "nested callback reload".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    let caller = runtime.clone();
    let invocation = tokio::spawn(async move {
        invoke_in_session(
            &caller,
            KEY,
            "reload",
            json!({"nested": true, "wait": true, "repeat": true}),
            session.id,
        )
        .await
    });
    tokio::time::timeout(WAIT, fixture.observed.reload_entered.notified())
        .await
        .unwrap();
    let accepted = fixture
        .observed
        .reload_request
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(
        runtime.current_snapshot().generation(),
        1,
        "nested settings return must not end the outer call's barrier"
    );
    fixture.observed.reload_release.notify_one();
    let result = invocation.await.unwrap();
    assert!(result.is_ok(), "nested reload caller completes: {result:?}");
    wait_reload(&runtime, &accepted.task_id).await;
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn accepted_reload_reports_its_build_failure_and_unknown_tasks_are_rejected() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let client = runtime.current_snapshot().host_client.clone();
    fixture
        .observed
        .provider_error
        .store(true, Ordering::SeqCst);
    let output = invoke_named(&runtime, KEY, "reload", json!({}))
        .await
        .unwrap();
    let accepted: agena_plugin_sdk::host_api::HostConfigReloadRequestResponse =
        serde_json::from_str(&output.output_text).unwrap();
    let state = tokio::time::timeout(WAIT, async {
        loop {
            let status = client
                .config_reload_status(agena_plugin_sdk::host_api::HostConfigReloadStatusRequest {
                    task_id: accepted.task_id.clone(),
                })
                .await
                .unwrap();
            if !matches!(
                status.state,
                agena_plugin_sdk::host_api::HostConfigReloadState::Running {}
            ) {
                break status.state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        state,
        agena_plugin_sdk::host_api::HostConfigReloadState::Failed { .. }
    ));
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert!(
        client
            .config_reload_status(agena_plugin_sdk::host_api::HostConfigReloadStatusRequest {
                task_id: "unknown".into()
            })
            .await
            .is_err()
    );
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn native_http_stream_defers_self_reload_until_its_terminal_result() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let plugin_host = runtime.current_snapshot().plugin_manager();
    let registered = plugin_host
        .registered_tools()
        .into_iter()
        .find(|tool| tool.canonical_name() == format!("{KEY}.reload"))
        .unwrap();
    let mut stream = tokio::time::timeout(
        WAIT,
        plugin_host.invoke_tool_stream(
            &registered,
            ToolInvokeInput {
                tool_name: "reload".into(),
                session_id: -1,
                call_id: 1,
                workspace_root: runtime.workspace_root().to_string_lossy().into(),
                input: json!({}),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!stream.stream_id.starts_with("emu-"));
    tokio::time::timeout(WAIT, fixture.observed.reload_entered.notified())
        .await
        .unwrap();
    let first = tokio::time::timeout(WAIT, stream.chunks.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.text_delta.as_deref(), Some("queued"));
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert!(matches!(
        stream.end.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    fixture.observed.reload_release.notify_one();
    let last = tokio::time::timeout(WAIT, stream.chunks.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last.text_delta.as_deref(), Some("complete"));
    assert!(
        tokio::time::timeout(WAIT, stream.chunks.recv())
            .await
            .unwrap()
            .is_none()
    );
    let end = tokio::time::timeout(WAIT, stream.end)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let accepted: agena_plugin_sdk::host_api::HostConfigReloadRequestResponse =
        serde_json::from_str(&end.output_text).unwrap();
    wait_reload(&runtime, &accepted.task_id).await;
    assert_eq!(runtime.current_snapshot().generation(), 2);
    assert_eq!(fixture.read_config().await, json!("en-US"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn self_reload_with_changed_settings_returns_before_replacing_the_requesting_object() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let old = runtime.current_snapshot();
    let caller = runtime.clone();
    let invocation =
        tokio::spawn(
            async move { invoke_named(&caller, KEY, "reload", json!({"wait": true})).await },
        );
    tokio::time::timeout(WAIT, fixture.observed.reload_entered.notified())
        .await
        .unwrap();
    fixture.plugins.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.write_config("zh-CN");
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.init_config.lock().unwrap().len(), 1);
    fixture.observed.reload_release.notify_one();
    let output = invocation.await.unwrap().unwrap();
    let accepted: agena_plugin_sdk::host_api::HostConfigReloadRequestResponse =
        serde_json::from_str(&output.output_text).unwrap();
    wait_reload(&runtime, &accepted.task_id).await;
    assert_eq!(runtime.current_snapshot().generation(), 2);
    assert_eq!(
        *fixture.observed.init_config.lock().unwrap(),
        vec![json!("en-US"), json!("zh-CN")]
    );
    assert_eq!(
        fixture.observed.contexts.lock().unwrap()[1].settings,
        json!({"revision": 2})
    );
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
    old.plugin_manager().shutdown().await;
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.read_config().await, json!("zh-CN"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn shutdown_cancels_an_accepted_reload_waiting_for_its_originating_call() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let caller = runtime.clone();
    let invocation =
        tokio::spawn(
            async move { invoke_named(&caller, KEY, "reload", json!({"wait": true})).await },
        );
    tokio::time::timeout(WAIT, fixture.observed.reload_entered.notified())
        .await
        .unwrap();
    let task_id = fixture
        .observed
        .reload_request
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .task_id
        .clone();
    runtime.shutdown();
    tokio::time::timeout(WAIT, async {
        loop {
            let task = runtime
                .background_tasks()
                .into_iter()
                .find(|task| task.id == task_id)
                .unwrap();
            if !task.is_running() {
                assert_eq!(task.status, crate::RuntimeBackgroundTaskStatus::Cancelled);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.observed.reload_release.notify_one();
    assert!(invocation.await.unwrap().is_ok());
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
    assert!(
        invoke_named(&runtime, KEY, "reload", json!({}))
            .await
            .is_err()
    );
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn accepted_reload_survives_cancellation_of_its_originating_call() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let caller = runtime.clone();
    let invocation =
        tokio::spawn(
            async move { invoke_named(&caller, KEY, "reload", json!({"wait": true})).await },
        );
    tokio::time::timeout(WAIT, fixture.observed.reload_entered.notified())
        .await
        .unwrap();
    let task_id = fixture
        .observed
        .reload_request
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .task_id
        .clone();
    invocation.abort();
    assert!(invocation.await.unwrap_err().is_cancelled());
    wait_reload(&runtime, &task_id).await;
    assert_eq!(runtime.current_snapshot().generation(), 2);
    assert_eq!(fixture.read_config().await, json!("en-US"));
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn malformed_reload_callbacks_are_rejected_before_task_admission() {
    use agena_plugin_sdk::rpc::{
        JsonRpcVersion, Request, RequestId, Response, ResponsePayload, method,
    };
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let context = fixture.observed.contexts.lock().unwrap()[0].clone();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(WAIT)
        .build()
        .unwrap();
    for (index, (method, params)) in [
        (
            method::HOST_CONFIG_RELOAD_REQUEST,
            json!({"context": {}, "config": {}}),
        ),
        (
            method::HOST_CONFIG_RELOAD_REQUEST,
            json!({"context": {"authority_token": "forged"}}),
        ),
        (
            method::HOST_CONFIG_RELOAD_REQUEST,
            json!({"context": {"session_id": 42}}),
        ),
        (method::HOST_CONFIG_RELOAD_REQUEST, json!({})),
        (
            method::HOST_CONFIG_RELOAD_STATUS,
            json!({"context": {}, "request": {"task_id": "unknown", "config": {}}}),
        ),
        (
            method::HOST_CONFIG_RELOAD_STATUS,
            json!({"context": {}, "request": {"task_id": "unknown"}, "generation": 2}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let id = RequestId::Num(index as i64);
        let response: Response = client
            .post(context.host_callback_url.as_ref().unwrap())
            .bearer_auth(context.host_callback_token.as_ref().unwrap())
            .json(&Request {
                jsonrpc: JsonRpcVersion,
                id: id.clone(),
                method: method.into(),
                params: Some(params),
                context: None,
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(response.id, id);
        assert!(matches!(response.payload, ResponsePayload::Err { .. }));
    }
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert!(
        !runtime
            .background_tasks()
            .iter()
            .any(|task| task.kind == crate::RuntimeBackgroundTaskKind::RuntimeReload)
    );
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn context_free_request_response_hooks_also_defer_self_reload() {
    for chained in [false, true] {
        let mut fixture = Fixture::new().await;
        fixture.write_config("en-US");
        let runtime = fixture.start_with_maintenance(true).await;
        let caller = runtime.clone();
        let invocation = tokio::spawn(async move {
            let host = caller.current_snapshot().plugin_manager();
            if chained {
                host.dispatch_chat_params(agena_plugin_sdk::ChatParamsInput {
                    provider: "test".into(),
                    model: "test".into(),
                    params: json!({}),
                    session_id: None,
                })
                .await
                .map(|_| ())
            } else {
                host.dispatch_shell_env(
                    agena_plugin_sdk::ShellEnvInput {
                        cwd: caller.workspace_root().to_path_buf(),
                        session_id: None,
                        call_id: None,
                    },
                    None,
                )
                .await
                .map(|_| ())
            }
        });
        tokio::time::timeout(WAIT, fixture.observed.reload_entered.notified())
            .await
            .unwrap();
        assert_eq!(runtime.current_snapshot().generation(), 1);
        fixture.observed.reload_release.notify_one();
        assert!(invocation.await.unwrap().is_ok());
        let accepted = fixture
            .observed
            .reload_request
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        wait_reload(&runtime, &accepted.task_id).await;
        assert_eq!(runtime.current_snapshot().generation(), 2);
        runtime.current_snapshot().plugin_manager().shutdown().await;
    }
}

#[tokio::test]
async fn background_admission_rechecks_shutdown_before_registering_work() {
    struct ShutdownDuringTitle(Arc<AgenaRuntime>);
    impl From<ShutdownDuringTitle> for String {
        fn from(title: ShutdownDuringTitle) -> Self {
            title.0.shutdown();
            "shutdown during task admission".into()
        }
    }
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    let result = runtime.spawn_background_task(
        crate::RuntimeBackgroundTaskKind::RuntimeReload,
        crate::RuntimeBackgroundTaskOrigin::User,
        ShutdownDuringTitle(runtime.clone()),
        None,
        true,
        |_| async {
            panic!("shutdown must reject this work before registration");
        },
    );
    assert!(
        matches!(
            result,
            Err(crate::RuntimeBackgroundTaskControlError::Shutdown)
        ),
        "admission crossed shutdown: {result:?}"
    );
    assert!(
        !runtime
            .background_tasks()
            .iter()
            .any(|task| task.kind == crate::RuntimeBackgroundTaskKind::RuntimeReload)
    );
    runtime.current_snapshot().plugin_manager().shutdown().await;
}

#[tokio::test]
async fn plugin_reload_reports_capacity_and_recovers_after_a_task_finishes() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start_with_maintenance(true).await;
    for task in runtime
        .background_tasks()
        .into_iter()
        .filter(|task| task.is_running())
    {
        runtime.cancel_background_task(&task.id).unwrap();
    }
    tokio::time::timeout(WAIT, async {
        while runtime
            .background_tasks()
            .iter()
            .any(|task| task.is_running())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let limit = crate::background_task_registry::DEFAULT_ACTIVE_TASK_LIMIT;
    let mut fillers = Vec::new();
    for _ in 0..limit {
        let accepted = runtime
            .spawn_background_task(
                crate::RuntimeBackgroundTaskKind::RuntimeReload,
                crate::RuntimeBackgroundTaskOrigin::User,
                "held capacity fixture",
                None,
                true,
                |_| std::future::pending(),
            )
            .unwrap();
        fillers.push(accepted.task.id);
    }
    let error = invoke_named(&runtime, KEY, "reload", json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        error.kind,
        agena_plugin_sdk::PluginErrorKind::HostUnavailable
    );
    assert_eq!(error.failure.retry, agena_failure::RetryDirective::Backoff);
    assert_eq!(
        error.failure.recovery,
        agena_failure::RecoveryDirective::Retry
    );
    assert!(
        error.to_string().contains("capacity"),
        "overload should explain why the task was rejected: {error}"
    );
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert_eq!(
        runtime
            .background_tasks()
            .iter()
            .filter(|task| task.is_running())
            .count(),
        limit
    );
    runtime.cancel_background_task(&fillers[0]).unwrap();
    tokio::time::timeout(WAIT, async {
        while runtime
            .background_tasks()
            .iter()
            .any(|task| task.id == fillers[0] && task.is_running())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let output = invoke_named(&runtime, KEY, "reload", json!({}))
        .await
        .unwrap();
    let accepted: agena_plugin_sdk::host_api::HostConfigReloadRequestResponse =
        serde_json::from_str(&output.output_text).unwrap();
    wait_reload(&runtime, &accepted.task_id).await;
    assert_eq!(runtime.current_snapshot().generation(), 2);
    assert!(
        runtime
            .current_snapshot()
            .model_catalog()
            .snapshot()
            .last_failure
            .is_some(),
        "optional refresh admission failure must remain observable after the reload publishes"
    );
    runtime.shutdown();
    tokio::time::timeout(WAIT, async {
        while runtime
            .background_tasks()
            .iter()
            .any(|task| task.is_running())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    runtime.current_snapshot().plugin_manager().shutdown().await;
}
