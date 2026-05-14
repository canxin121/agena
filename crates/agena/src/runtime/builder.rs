use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use crate::{
    AppError,
    config::{
        ConfigLoader, ConfigResolution, LoadConfigRequest, ProcessEnvironment, TracingConfig,
    },
    db::init_schema,
    session::SessionManager,
    tracing as tracing_config,
};

use super::{
    RuntimeReloadCause, RuntimeReloadReport, RuntimeSnapshot, janitor, reload,
    store::{RuntimeSnapshotStore, TaskControl},
};

pub type TracingFilterReloadHandle =
    tracing_subscriber::reload::Handle<EnvFilter, tracing_subscriber::Registry>;

pub struct AgenaRuntimeBuilder {
    load_request: LoadConfigRequest,
    workspace_root: Option<PathBuf>,
    database_connection: Option<Arc<DatabaseConnection>>,
    database_url: Option<String>,
    auto_migrate: bool,
    tracing_reload_handle: Option<TracingFilterReloadHandle>,
}

impl Default for AgenaRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgenaRuntimeBuilder {
    pub fn new() -> Self {
        Self {
            load_request: LoadConfigRequest::default(),
            workspace_root: None,
            database_connection: None,
            database_url: None,
            auto_migrate: true,
            tracing_reload_handle: None,
        }
    }

    pub fn with_load_request(mut self, request: LoadConfigRequest) -> Self {
        self.load_request = request;
        self
    }

    pub fn with_workspace_root(mut self, workspace_root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(workspace_root.into());
        self
    }

    pub fn with_database_connection(mut self, db: DatabaseConnection) -> Self {
        self.database_connection = Some(Arc::new(db));
        self
    }

    pub fn with_database_url(mut self, url: impl Into<String>) -> Self {
        self.database_url = Some(url.into());
        self
    }

    pub fn with_auto_migrate(mut self, auto_migrate: bool) -> Self {
        self.auto_migrate = auto_migrate;
        self
    }

    pub fn with_tracing_reload_handle(mut self, handle: TracingFilterReloadHandle) -> Self {
        self.tracing_reload_handle = Some(handle);
        self
    }

    pub async fn build(self) -> Result<AgenaRuntime, AppError> {
        let AgenaRuntimeBuilder {
            load_request,
            workspace_root,
            database_connection,
            database_url,
            auto_migrate,
            tracing_reload_handle,
        } = self;
        let workspace_root = workspace_root.unwrap_or(env::current_dir()?);
        let loader = ConfigLoader::new(ProcessEnvironment);
        let initial_resolution = loader.load(&load_request)?;
        let database = connect_database(
            database_connection,
            database_url,
            auto_migrate,
            &initial_resolution.config.tracing,
        )
        .await?;
        let initial_snapshot = Arc::new(
            RuntimeSnapshot::build(
                1,
                &loader,
                &load_request,
                workspace_root.as_path(),
                database.clone(),
                None,
            )
            .await?,
        );

        let runtime = AgenaRuntime {
            inner: Arc::new(AgenaRuntimeInner {
                loader,
                load_request,
                workspace_root,
                database,
                snapshot_store: RuntimeSnapshotStore::new(initial_snapshot.clone()),
                reload_lock: Mutex::new(()),
                task_control: Arc::new(TaskControl::default()),
                tracing_reload_handle,
            }),
        };

        // Install the runtime-backed HostClient into the plugin host so
        // plugin → host callbacks (log/read_config/etc.) actually do work.
        {
            let host_handle = initial_snapshot.plugin_manager().host_handle();
            let client = super::host_client_for(runtime.clone());
            host_handle.install_client(client).await;
        }

        runtime.apply_tracing_filter(&initial_snapshot.config_resolution().config.tracing);
        runtime.spawn_background_tasks();
        Ok(runtime)
    }
}

async fn connect_database(
    database_connection: Option<Arc<DatabaseConnection>>,
    database_url: Option<String>,
    auto_migrate: bool,
    tracing: &TracingConfig,
) -> Result<Option<Arc<DatabaseConnection>>, AppError> {
    let database = if let Some(db) = database_connection {
        Some(db)
    } else if let Some(url) = database_url {
        Some(Arc::new(
            tracing_config::connect_database(url.as_str(), tracing).await?,
        ))
    } else {
        None
    };

    if auto_migrate && let Some(db) = database.as_ref() {
        init_schema(db.as_ref()).await?;
    }

    Ok(database)
}

#[derive(Clone)]
pub struct AgenaRuntime {
    pub(crate) inner: Arc<AgenaRuntimeInner>,
}

pub(crate) struct AgenaRuntimeInner {
    loader: ConfigLoader<ProcessEnvironment>,
    load_request: LoadConfigRequest,
    workspace_root: PathBuf,
    database: Option<Arc<DatabaseConnection>>,
    snapshot_store: RuntimeSnapshotStore,
    reload_lock: Mutex<()>,
    task_control: Arc<TaskControl>,
    tracing_reload_handle: Option<TracingFilterReloadHandle>,
}

impl Drop for AgenaRuntimeInner {
    fn drop(&mut self) {
        self.task_control.shutdown();
    }
}

impl AgenaRuntime {
    pub fn builder() -> AgenaRuntimeBuilder {
        AgenaRuntimeBuilder::new()
    }

    pub fn current_snapshot(&self) -> Arc<RuntimeSnapshot> {
        self.inner.snapshot_store.current()
    }

    pub fn config_resolution(&self) -> Arc<ConfigResolution> {
        Arc::new(self.current_snapshot().config_resolution().clone())
    }

    pub fn session_manager(&self) -> Option<Arc<SessionManager>> {
        self.current_snapshot().session_manager()
    }

    pub fn workspace_root(&self) -> &Path {
        &self.inner.workspace_root
    }

    pub fn shutdown(&self) {
        if let Some(session_manager) = self.session_manager() {
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    handle.spawn(async move {
                        session_manager
                            .broadcast_active_session_end(crate::plugin::SessionEndReason::Other)
                            .await;
                    });
                }
                Err(_) => {
                    tracing::debug!(
                        target: "agena_plugin_host::session_end",
                        "no tokio runtime available during shutdown; skipping session.end broadcast"
                    );
                }
            }
        }
        self.inner.task_control.shutdown();
    }

    pub async fn reload(&self) -> Result<RuntimeReloadReport, AppError> {
        self.reload_with_cause(RuntimeReloadCause::Manual).await
    }

    pub(crate) async fn reload_with_cause(
        &self,
        cause: RuntimeReloadCause,
    ) -> Result<RuntimeReloadReport, AppError> {
        let _guard = self.inner.reload_lock.lock().await;
        let previous = self.current_snapshot();
        let next = Arc::new(
            RuntimeSnapshot::build_with_previous(
                previous.generation() + 1,
                &self.inner.loader,
                &self.inner.load_request,
                self.inner.workspace_root.as_path(),
                self.inner.database.clone(),
                previous.session_manager(),
                Arc::clone(&previous),
            )
            .await?,
        );

        self.apply_tracing_filter(&next.config_resolution().config.tracing);
        let previous_generation = previous.generation();
        // Install runtime-backed HostClient into the new snapshot's plugin
        // host so post-reload plugin callbacks keep working.
        {
            let host_handle = next.plugin_manager().host_handle();
            let client = super::host_client_for(self.clone());
            host_handle.install_client(client).await;
        }
        let _ = self.inner.snapshot_store.swap(next.clone());

        Ok(RuntimeReloadReport {
            cause,
            previous_generation,
            generation: next.generation(),
            loaded_at: next.loaded_at(),
        })
    }

    pub(crate) fn task_control(&self) -> &TaskControl {
        self.inner.task_control.as_ref()
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.inner.task_control.is_shutdown()
    }

    fn spawn_background_tasks(&self) {
        let janitor_runtime = self.clone();
        tokio::spawn(async move {
            janitor::run(janitor_runtime).await;
        });

        let reload_runtime = self.clone();
        tokio::spawn(async move {
            reload::run(reload_runtime).await;
        });
    }

    fn apply_tracing_filter(&self, tracing: &TracingConfig) {
        let Some(handle) = self.inner.tracing_reload_handle.as_ref() else {
            return;
        };

        match tracing_config::env_filter(tracing) {
            Ok(next) => {
                if let Err(err) = handle.reload(next) {
                    tracing::warn!(error = %err, "failed to reload tracing filter");
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    filter = tracing.filter,
                    database_level = tracing.database_level,
                    "invalid tracing filter in runtime config"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::config::LoadConfigRequest;

    use super::{AgenaRuntime, AgenaRuntimeBuilder};

    #[tokio::test]
    async fn manual_reload_swaps_runtime_generation() {
        let path = write_temp_config(
            r#"
[tracing]
filter = "info"

[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        );
        let workspace_root = path
            .parent()
            .expect("config should have parent")
            .to_path_buf();

        let runtime = AgenaRuntimeBuilder::new()
            .with_load_request(LoadConfigRequest {
                config_path: Some(path.clone()),
                ..LoadConfigRequest::default()
            })
            .with_workspace_root(workspace_root)
            .build()
            .await
            .expect("runtime should build");

        assert_eq!(runtime.current_snapshot().generation(), 1);

        fs::write(
            &path,
            r#"
[tracing]
filter = "debug"

[providers.openai]
default_model = "openai/gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config rewrite should succeed");

        let report = runtime.reload().await.expect("reload should succeed");
        assert_eq!(report.previous_generation, 1);
        assert_eq!(report.generation, 2);
        assert_eq!(
            runtime
                .current_snapshot()
                .config_resolution()
                .config
                .tracing
                .filter,
            "debug"
        );

        runtime.shutdown();
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn builder_creates_session_manager_when_database_is_configured() {
        let path = write_temp_config(
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        );
        let workspace_root = path
            .parent()
            .expect("config should have parent")
            .to_path_buf();

        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(path.clone()),
                ..LoadConfigRequest::default()
            })
            .with_workspace_root(workspace_root)
            .with_database_url("sqlite::memory:")
            .build()
            .await
            .expect("runtime should build");

        assert!(runtime.session_manager().is_some());

        runtime.shutdown();
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn reload_reuses_existing_session_manager() {
        let path = write_temp_config(
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        );
        let workspace_root = path
            .parent()
            .expect("config should have parent")
            .to_path_buf();

        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(path.clone()),
                ..LoadConfigRequest::default()
            })
            .with_workspace_root(workspace_root)
            .with_database_url("sqlite::memory:")
            .build()
            .await
            .expect("runtime should build");
        let before = runtime
            .session_manager()
            .expect("session manager should be available");

        fs::write(
            &path,
            r#"
[tracing]
filter = "debug"

[providers.openai]
default_model = "openai/gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config rewrite should succeed");

        runtime.reload().await.expect("reload should succeed");
        let after = runtime
            .session_manager()
            .expect("session manager should still be available");

        assert!(Arc::ptr_eq(&before, &after));

        runtime.shutdown();
        let _ = fs::remove_file(path);
    }

    fn write_temp_config(content: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("agena-runtime-test-{suffix}"));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        let path = dir.join("config.toml");
        fs::write(&path, content).expect("config should be written");
        path
    }
}
