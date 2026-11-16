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

struct Fixture {
    application: Application,
    runtime: agena_runtime::RuntimeBootstrapResult,
    directory: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("workspace")).unwrap();
        let plugins = agena_bundled_plugins::plugins::sources::bundled_plugin_entries()
            .into_keys()
            .map(|id| (id, json!({"enabled":false,"package":{"kind":"static"}})))
            .collect::<serde_json::Map<String, Value>>();
        std::fs::write(
            root.join("agena.json"),
            serde_json::to_vec(&json!({"plugins":{"list":plugins}})).unwrap(),
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
    let mut fixture = Fixture::new().await;
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
