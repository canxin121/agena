use super::Application;
use agena_api::management::{SettingsCommand, settings::*};
use agena_runtime::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind,
    RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskStart, RuntimeBackgroundTaskWork,
    RuntimeControlService, RuntimeControlServiceError, RuntimeMetricsSnapshot, RuntimeReloadCause,
    RuntimeReloadReport,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Fail before publication once, then use the real runtime and session reload.
struct FailFirstReload {
    inner: Arc<dyn RuntimeControlService>,
    attempts: AtomicUsize,
}

#[async_trait::async_trait]
impl RuntimeControlService for FailFirstReload {
    async fn reload(&self) -> Result<RuntimeReloadReport, RuntimeControlServiceError> {
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(RuntimeControlServiceError::new(
                "injected runtime preparation failure before publication",
            ));
        }
        self.inner.reload().await
    }

    async fn fetch_provider_client_versions(
        &self,
    ) -> Result<agena_provider::ProviderClientVersions, RuntimeControlServiceError> {
        self.inner.fetch_provider_client_versions().await
    }

    fn runtime_metrics(&self) -> RuntimeMetricsSnapshot {
        self.inner.runtime_metrics()
    }

    fn start_runtime_reload_task(
        &self,
        cause: RuntimeReloadCause,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        self.inner.start_runtime_reload_task(cause, origin)
    }

    fn start_background_task(
        &self,
        kind: RuntimeBackgroundTaskKind,
        origin: RuntimeBackgroundTaskOrigin,
        title: String,
        dedupe_key: Option<String>,
        cancellable: bool,
        work: RuntimeBackgroundTaskWork,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        self.inner
            .start_background_task(kind, origin, title, dedupe_key, cancellable, work)
    }

    fn background_tasks(&self) -> Vec<RuntimeBackgroundTask> {
        self.inner.background_tasks()
    }

    fn cancel_background_task(
        &self,
        task_id: &str,
    ) -> Result<RuntimeBackgroundTask, RuntimeBackgroundTaskControlError> {
        self.inner.cancel_background_task(task_id)
    }
}

/// Reproduce a profile with variant defaults before the provider catalog resolves
/// its route. Other control operations still use the composed session service.
struct VariantOnlySelection {
    inner: Arc<dyn agena_runtime::SessionExecutionControl>,
}

#[async_trait::async_trait]
impl agena_runtime::SessionExecutionControl for VariantOnlySelection {
    async fn selected_execution_selection(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::ExecutionSelection, agena_runtime::SessionExecutionControlError> {
        let mut selection = self.inner.selected_execution_selection(session_id).await?;
        selection.provider = None;
        selection.adapter = None;
        selection.model = None;
        Ok(selection)
    }

    async fn selected_model(
        &self,
        _session_id: i64,
    ) -> Result<Option<agena_domain::ModelRef>, agena_runtime::SessionExecutionControlError> {
        Ok(None)
    }

    async fn set_message_queue_paused(
        &self,
        session_id: i64,
        paused: bool,
    ) -> Result<(), agena_runtime::SessionExecutionControlError> {
        self.inner
            .set_message_queue_paused(session_id, paused)
            .await
    }

    async fn active_execution(&self, session_id: i64) -> Option<agena_domain::ExecutionLifecycle> {
        self.inner.active_execution(session_id).await
    }

    async fn cancel_execution_with_outcome(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationOutcome, agena_runtime::SessionExecutionControlError>
    {
        self.inner
            .cancel_execution_with_outcome(session_id, execution_id)
            .await
    }

    async fn cancel_session_with_outcome(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::CancellationOutcome, agena_runtime::SessionExecutionControlError>
    {
        self.inner.cancel_session_with_outcome(session_id).await
    }

    async fn list_scheduled_jobs(
        &self,
    ) -> Result<Vec<agena_scheduler::ScheduledJob>, agena_runtime::SessionExecutionControlError>
    {
        self.inner.list_scheduled_jobs().await
    }

    fn scheduler_available(&self) -> bool {
        self.inner.scheduler_available()
    }
}

struct Fixture {
    application: Application,
    runtime: agena_runtime::RuntimeBootstrapResult,
    directory: tempfile::TempDir,
}

impl Fixture {
    async fn with_config(mut config: Value) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("workspace")).unwrap();
        let plugins = agena_bundled_plugins::plugins::sources::bundled_plugin_entries()
            .into_keys()
            .map(|id| (id, json!({"enabled":false,"package":{"kind":"static"}})))
            .collect::<serde_json::Map<String, Value>>();
        config["plugins"] = json!({"list":plugins});
        std::fs::write(
            root.join("agena.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let (application, runtime) =
            Application::bootstrap(agena_runtime::RuntimeBootstrapRequest {
                workspace_root: Some(root.join("workspace")),
                config_path: Some(root.join("agena.json")),
                database_path: Some(root.join("test.sqlite")),
                automatic_maintenance: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        Self {
            application,
            runtime,
            directory,
        }
    }

    async fn command(&self, value: Value) -> Value {
        serde_json::to_value(
            crate::dispatch::dispatch_command(
                &self.application,
                serde_json::from_value(value).unwrap(),
            )
            .await
            .unwrap(),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

async fn assert_session_path_modes(
    application: &Application,
    session_id: i64,
    workspace: &str,
    external: &str,
) {
    let session = serde_json::to_value(
        application
            .session_execution_resource(session_id)
            .await
            .unwrap(),
    )
    .unwrap();
    let execution = &session["execution"];
    assert!(execution["selected_permission"].get("path").is_none());
    for source in ["inherited_permission", "effective_permission"] {
        assert_eq!(execution[source]["path"]["workspace"], workspace);
        assert_eq!(execution[source]["path"]["external"], external);
    }
}

fn permission_save() -> SettingsCommand {
    SettingsCommand::SetLayer {
        layer: ConfigSettingsLayer::Global,
        workspace_id: None,
        input: ConfigSettingsSetInput {
            path: "permission".into(),
            value: json!({"path":{"workspace":"readwrite","external":"readonly"}}),
            options: ConfigSettingsEditOptions {
                expected_revision: None,
                dry_run: false,
                validate: true,
                reload: true,
            },
        },
    }
}

#[tokio::test]
async fn identical_permission_save_retries_failed_reload_and_updates_existing_sessions() {
    const CHILD_GUARD: &str = "AGENA_SETTINGS_RELOAD_TEST_CHILD";
    if std::env::var_os(CHILD_GUARD).is_none() {
        // StorageLayout is process-wide; explicit config/database paths do not
        // isolate every plugin, attachment, or cache path. Run the real runtime
        // in its own process without changing the parallel test runner's env.
        let storage = tempfile::tempdir().unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("application::settings_reload_tests::identical_permission_save_retries_failed_reload_and_updates_existing_sessions")
            .args(["--exact", "--nocapture", "--test-threads=1", "--format=pretty"])
            .env(CHILD_GUARD, "1")
            .env("AGENA_DATA_DIR", storage.path())
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(90), command.output())
            .await
            .expect("isolated settings reload regression timed out")
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "isolated regression failed:\n{stdout}\n{stderr}"
        );
        assert!(
            stdout.contains("1 passed"),
            "the isolated test did not run:\n{stdout}"
        );
        return;
    }
    let isolated_storage = std::env::var_os("AGENA_DATA_DIR")
        .expect("the isolated reload test requires its private data directory");
    agena_storage_layout::StorageLayout::initialize_process(isolated_storage).unwrap();
    let mut fixture = Fixture::with_config(json!({
        "permission": {"path": {"workspace": "readonly", "external": "deny"}}
    }))
    .await;
    let workspace = fixture
        .command(json!({"method":"create_workspace","params":{
            "path":fixture.directory.path().join("workspace")
        }}))
        .await;
    let session = fixture
        .command(json!({"method":"create_session","params":{
            "workspace_id":workspace["data"]["id"], "title":"Created before settings reload"
        }}))
        .await;
    let project_session = session["data"]["id"].as_i64().unwrap();
    let global_session = fixture
        .application
        .ensure_bot_session()
        .await
        .unwrap()
        .session_id;
    for session in [project_session, global_session] {
        assert_session_path_modes(&fixture.application, session, "readonly", "deny").await;
    }
    let initial_generation = fixture
        .application
        .runtime_snapshot_summary()
        .await
        .generation;
    let control = Arc::new(FailFirstReload {
        inner: fixture.application.runtime_control.clone(),
        attempts: AtomicUsize::new(0),
    });
    fixture.application.runtime_control = control.clone();

    // The actual settings handler persists first. A failed activation must not
    // publish new permissions, but its file is now identical to a retry's body.
    assert!(
        fixture
            .application
            .command_settings(permission_save())
            .await
            .is_err()
    );
    assert_eq!(control.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .application
            .runtime_snapshot_summary()
            .await
            .generation,
        initial_generation
    );
    let config_path = fixture.directory.path().join("agena.json");
    let saved_bytes = std::fs::read(&config_path).unwrap();
    let saved: Value = serde_json::from_slice(&saved_bytes).unwrap();
    assert_eq!(
        saved["permission"]["path"],
        json!({
            "workspace":"readwrite", "external":"readonly"
        })
    );
    for session in [project_session, global_session] {
        assert_session_path_modes(&fixture.application, session, "readonly", "deny").await;
    }

    let retry = fixture
        .application
        .command_settings(permission_save())
        .await
        .unwrap();
    assert_eq!(retry["changed"], false);
    assert_eq!(retry["reload_required"], true);
    assert_eq!(control.attempts.load(Ordering::SeqCst), 2);
    assert!(retry["reload"]["generation"].as_u64().unwrap() > initial_generation);
    assert_eq!(std::fs::read(&config_path).unwrap(), saved_bytes);
    let effective = fixture.application.config_json_sources().unwrap().effective;
    assert_eq!(effective["permission"]["path"]["workspace"], "readwrite");
    assert_eq!(effective["permission"]["path"]["external"], "readonly");
    for session in [project_session, global_session] {
        assert_session_path_modes(&fixture.application, session, "readwrite", "readonly").await;
    }
    fixture.runtime.shutdown_and_wait().await.unwrap();
}

/// Exercise the public JSON command/metadata DTO and ApplicationService path;
/// ordinary Web/API sessions do not use SessionManager::create_session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_profiles_are_applied_through_public_session_commands() {
    const CHILD_GUARD: &str = "AGENA_APPLICATION_PROFILE_TEST_CHILD";
    if std::env::var_os(CHILD_GUARD).is_none() {
        let storage = tempfile::tempdir().unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command.arg("application::settings_reload_tests::named_profiles_are_applied_through_public_session_commands")
            .args(["--exact", "--nocapture", "--test-threads=1", "--format=pretty"])
            .env(CHILD_GUARD, "1").env("AGENA_DATA_DIR", storage.path()).kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(90), command.output())
            .await
            .expect("isolated profile application regression timed out")
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "isolated profile regression failed:\n{stdout}\n{stderr}"
        );
        assert!(
            stdout.contains("1 passed"),
            "isolated profile test did not run: {stdout}"
        );
        return;
    }
    agena_storage_layout::StorageLayout::initialize_process(
        std::env::var_os("AGENA_DATA_DIR").unwrap(),
    )
    .unwrap();
    let fixture = Fixture::with_config(json!({
        "sandbox":false,
        "permission":{"path":{"workspace":"readwrite","external":"deny"}},
        "providers":{
            "default_selection":{"provider":"local","adapter":"openai_chat_completions","model":"base-model"},
            "local":{
                "auth":{"mode":"api","subtype":"custom","base_url":"http://127.0.0.1:1","api_key":{"kind":"inline","value":"profile-test-key"}},
                "adapters":{"openai_chat_completions":{"enabled":true,"models":{
                    "base-model":{
                        "supports_parallel_tool_calls":true,
                        "supports_verbosity":true,
                        "verbosity_levels":["low","high"],
                        "thinking_modes":{"default":null,"high":{"strategy":"effort","effort":"high"}},
                        "speed_modes":{"default":null,"fast":{"display_name":"Fast","request_override":{"body_patch":{"service_tier":"priority"}}}}
                    }
                }}}
            }
        },
        "default_profile":"review",
        "profiles":{"review":{
            "model":{"provider":"local","adapter":"openai_chat_completions","model":"review-model"},
            "permission":{"path":{"workspace":"readonly"}},
            "prompts":{"order":["review-note"],"inline":[{"id":"review-note","body":"Review carefully."}]}
        },"careful":{
            "model":{"thinking_mode":"high","speed_mode":"fast","verbosity":"low","parallel_tool_calls":false}
        }}
    })).await;
    let workspace = fixture.command(json!({"method":"create_workspace","params":{"path":fixture.directory.path().join("workspace")}})).await;
    let workspace_id = workspace["data"]["id"].as_i64().unwrap();
    let created = fixture.command(json!({"method":"create_session","params":{"workspace_id":workspace_id,"title":"Profile application test"}})).await;
    let id = created["data"]["id"].as_i64().unwrap();
    let execution = serde_json::to_value(
        fixture
            .application
            .session_execution_resource(id)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(execution["execution"]["profile"], "review");
    assert_eq!(execution["execution"]["model_id"], "review-model");
    assert_eq!(
        execution["execution"]["effective_permission"]["path"]["workspace"],
        "readonly"
    );

    let before = fixture
        .application
        .service()
        .session_kernel()
        .reader
        .query_summaries(Default::default())
        .await
        .unwrap()
        .sessions
        .len();
    let invalid = crate::dispatch::dispatch_command(
        &fixture.application,
        serde_json::from_value(json!({"method":"create_session","params":{
            "workspace_id":workspace_id,"title":"Must not be created","profile":"missing"
        }}))
        .unwrap(),
    )
    .await;
    assert!(invalid.is_err());
    let after = fixture
        .application
        .service()
        .session_kernel()
        .reader
        .query_summaries(Default::default())
        .await
        .unwrap()
        .sessions
        .len();
    assert_eq!(
        before, after,
        "unknown profile creation must not leave a session behind"
    );

    fixture
        .application
        .dispatch_extended_session_command(
            agena_api::extended_sessions::SessionCommand::UpdateMetadata {
                session_id: id,
                update: serde_json::from_value(json!({"profile":""})).unwrap(),
                expected_version: Some(created["data"]["version"].as_i64().unwrap()),
            },
        )
        .await
        .unwrap();
    let execution = serde_json::to_value(
        fixture
            .application
            .session_execution_resource(id)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(execution["execution"]["profile"], "main");
    assert_eq!(execution["execution"]["profile_is_default"], true);
    assert_eq!(execution["execution"]["model_id"], "base-model");
    assert_eq!(
        execution["execution"]["effective_permission"]["path"]["workspace"],
        "readwrite"
    );
    let inherited =
        crate::session::resolve_session_run_options(&fixture.application, id, Default::default())
            .await
            .unwrap();
    assert_eq!(inherited.model_supports_parallel_tool_calls, Some(true));

    let created = fixture
        .command(json!({"method":"create_session","params":{
            "workspace_id":workspace_id,"title":"Variant profile","profile":"careful"
        }}))
        .await;
    let careful_id = created["data"]["id"].as_i64().unwrap();
    let options = crate::session::resolve_session_run_options(
        &fixture.application,
        careful_id,
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(options.model.model_id.as_ref(), "base-model");
    assert_eq!(options.thinking_mode.as_deref(), Some("high"));
    assert_eq!(options.speed_mode.as_deref(), Some("fast"));
    assert_eq!(options.verbosity.as_deref(), Some("low"));
    assert_eq!(options.model_supports_parallel_tool_calls, Some(false));
    assert!(options.selection_override.as_ref().unwrap().is_empty());

    fixture
        .application
        .service()
        .session_kernel()
        .sessions
        .configure(
            agena_storage::kernel::RequestId::new(),
            agena_domain::session_kernel::SessionId::try_from(careful_id).unwrap(),
            agena_storage::kernel::SessionConfigurationChange {
                permission_ceiling_layers: Some(vec![
                    serde_json::from_value(json!({
                        "path":{"workspace":"readonly","external":"deny"}
                    }))
                    .unwrap(),
                ]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let execution = serde_json::to_value(
        fixture
            .application
            .session_execution_resource(careful_id)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        execution["execution"]["permission_ceiling_layers"][0]["path"]["workspace"],
        "readonly"
    );

    // The profile may supply only variants; provider-catalog fallback supplies
    // the model independently. Inheriting variants must not require a route.
    let control = VariantOnlySelection {
        inner: fixture
            .application
            .session_execution_services()
            .unwrap()
            .execution_control,
    };
    let options = fixture
        .application
        .service()
        .resolve_run_options(
            fixture.application.provider_catalog().as_ref(),
            &control,
            careful_id,
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(options.model.model_id.as_ref(), "base-model");
    assert_eq!(options.thinking_mode.as_deref(), Some("high"));
    assert_eq!(options.speed_mode.as_deref(), Some("fast"));
    assert_eq!(options.verbosity.as_deref(), Some("low"));
    assert_eq!(options.model_supports_parallel_tool_calls, Some(false));
    fixture.runtime.shutdown_and_wait().await.unwrap();
}
