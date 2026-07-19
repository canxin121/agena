use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::{
    AppError,
    config::{
        ConfigLoader, ConfigResolution, LoadConfigRequest, ProcessEnvironment, TracingConfig,
    },
    db::init_schema,
    session::SessionManager,
    storage::StorageConfig,
    tracing as tracing_config,
};

use super::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind,
    RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskStart, RuntimeReloadCause,
    RuntimeReloadReport, RuntimeSnapshot,
    background_tasks::{
        RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskRegistry, RuntimeBackgroundTaskSpec,
    },
    janitor, reload,
    store::{RuntimeSnapshotStore, TaskControl},
};

pub type TracingFilterReloadHandle =
    tracing_subscriber::reload::Handle<EnvFilter, tracing_subscriber::Registry>;

pub struct AgenaRuntimeConfig {
    pub load_request: LoadConfigRequest,
    pub workspace_root: Option<PathBuf>,
    pub database_connection: Option<Arc<DatabaseConnection>>,
    pub database_url: Option<String>,
    pub initialize_schema: bool,
    pub tracing_reload_handle: Option<TracingFilterReloadHandle>,
}

impl AgenaRuntime {
    pub async fn new(config: AgenaRuntimeConfig) -> Result<Self, AppError> {
        let AgenaRuntimeConfig {
            load_request,
            workspace_root,
            database_connection,
            database_url,
            initialize_schema,
            tracing_reload_handle,
        } = config;
        let workspace_root = workspace_root.unwrap_or(env::current_dir()?);
        let mut load_request = load_request;
        if load_request.workspace_root.is_none() {
            load_request.workspace_root = Some(workspace_root.clone());
        }
        let loader = ConfigLoader::new(ProcessEnvironment);
        let initial_resolution = loader.load(&load_request)?;
        let database = connect_database(
            database_connection,
            database_url,
            initialize_schema,
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
                background_tasks: RuntimeBackgroundTaskRegistry::default(),
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
            super::host_client::install_plugin_host_event_publisher(host_handle, runtime.clone());
        }

        runtime.apply_tracing_filter(&initial_snapshot.config_resolution().config.tracing);
        runtime.spawn_background_tasks();
        Ok(runtime)
    }
}

async fn connect_database(
    database_connection: Option<Arc<DatabaseConnection>>,
    database_url: Option<String>,
    initialize_schema: bool,
    tracing: &TracingConfig,
) -> Result<Option<Arc<DatabaseConnection>>, AppError> {
    let database = if let Some(db) = database_connection {
        Some(db)
    } else {
        let url = StorageConfig {
            database_url,
            database_path: None,
        }
        .resolve_url()?;
        StorageConfig::ensure_parent(url.as_str())?;
        Some(Arc::new(
            tracing_config::connect_database(url.as_str(), tracing).await?,
        ))
    };

    if initialize_schema && let Some(db) = database.as_ref() {
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
    background_tasks: RuntimeBackgroundTaskRegistry,
    task_control: Arc<TaskControl>,
    tracing_reload_handle: Option<TracingFilterReloadHandle>,
}

impl Drop for AgenaRuntimeInner {
    fn drop(&mut self) {
        self.task_control.shutdown();
    }
}

impl AgenaRuntime {
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
        self.inner.background_tasks.cancel_all();
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
            super::host_client::install_plugin_host_event_publisher(host_handle, self.clone());
        }
        let _ = self.inner.snapshot_store.swap(next.clone());
        let _ = self.start_model_catalog_refresh_if_needed(RuntimeBackgroundTaskOrigin::System);

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

    pub fn background_tasks(&self) -> Vec<RuntimeBackgroundTask> {
        self.inner.background_tasks.list()
    }

    pub fn model_catalog_refresh_active(&self) -> bool {
        self.inner
            .background_tasks
            .is_kind_running(RuntimeBackgroundTaskKind::ModelCatalogRefresh)
    }

    pub fn cancel_background_task(
        &self,
        task_id: &str,
    ) -> Result<RuntimeBackgroundTask, RuntimeBackgroundTaskControlError> {
        self.inner.background_tasks.cancel(task_id)
    }

    pub fn spawn_background_task<F, Fut>(
        &self,
        kind: RuntimeBackgroundTaskKind,
        origin: RuntimeBackgroundTaskOrigin,
        title: impl Into<String>,
        dedupe_key: Option<String>,
        cancellable: bool,
        work: F,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<RuntimeBackgroundTaskOutcome, AppError>>
            + Send
            + 'static,
    {
        if self.is_shutdown() {
            return Err(RuntimeBackgroundTaskControlError::Shutdown);
        }

        let spec =
            RuntimeBackgroundTaskSpec::from_parts(kind, origin, title, dedupe_key, cancellable);
        Ok(self.inner.background_tasks.spawn(spec, work))
    }

    pub fn start_runtime_reload_task(
        &self,
        cause: RuntimeReloadCause,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        let dedupe_key = Some("runtime_reload".to_owned());
        let title = match &cause {
            RuntimeReloadCause::Manual => "Reload runtime".to_owned(),
            RuntimeReloadCause::WatchedPathsChanged { paths } => {
                format!(
                    "Reload runtime after {} watched path change(s)",
                    paths.len()
                )
            }
        };
        let runtime = self.clone();
        self.spawn_background_task(
            RuntimeBackgroundTaskKind::RuntimeReload,
            origin,
            title,
            dedupe_key,
            false,
            move |_| async move {
                let report = runtime.reload_with_cause(cause.clone()).await?;
                let message = match &cause {
                    RuntimeReloadCause::Manual => {
                        format!("Runtime reloaded to generation {}.", report.generation)
                    }
                    RuntimeReloadCause::WatchedPathsChanged { paths } => format!(
                        "Runtime reloaded to generation {} after changes in {} watched path(s).",
                        report.generation,
                        paths.len()
                    ),
                };
                Ok(RuntimeBackgroundTaskOutcome::succeeded(message))
            },
        )
    }

    pub fn start_model_catalog_refresh(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        self.spawn_model_catalog_refresh(origin)
    }

    fn start_model_catalog_refresh_if_needed(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Option<RuntimeBackgroundTaskStart> {
        if self.is_shutdown() {
            return None;
        }

        if !self
            .current_snapshot()
            .model_catalog()
            .needs_startup_refresh()
        {
            return None;
        }

        self.spawn_model_catalog_refresh(origin).ok()
    }

    fn spawn_model_catalog_refresh(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        let runtime = self.clone();
        self.spawn_background_task(
            RuntimeBackgroundTaskKind::ModelCatalogRefresh,
            origin,
            "Refresh model catalog",
            Some("model_catalog_refresh".to_owned()),
            true,
            move |cancel| async move {
                let result: Result<RuntimeBackgroundTaskOutcome, AppError> = async {
                    let snapshot = runtime.current_snapshot();
                    let providers = snapshot.catalog_source_provider_registry();
                    let model_catalog = snapshot.model_catalog();
                    let refreshed = model_catalog
                        .refresh_from_registry(providers.as_ref(), Some(snapshot.config_resolution()))
                        .await?;

                    if cancel.is_cancelled() || runtime.is_shutdown() {
                        return Ok(RuntimeBackgroundTaskOutcome::cancelled(
                            "Cancelled before applying the refreshed catalog to the runtime snapshot.",
                        ));
                    }

                    runtime.reload().await?;

                    let message = refreshed
                        .last_error
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|warning| format!("Refreshed model catalog with warnings: {warning}"))
                        .unwrap_or_else(|| "Refreshed model catalog.".to_owned());

                    Ok(RuntimeBackgroundTaskOutcome::succeeded(message))
                }
                .await;

                if let Err(error) = &result {
                    runtime
                        .current_snapshot()
                        .model_catalog()
                        .record_refresh_failure(error.to_string());
                    tracing::warn!(
                        error = %error,
                        origin = ?origin,
                        "background model catalog refresh failed"
                    );
                }

                result
            },
        )
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

        let _ = self.start_model_catalog_refresh_if_needed(RuntimeBackgroundTaskOrigin::System);
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
                    database = tracing.database,
                    "invalid tracing filter in runtime config"
                );
            }
        }
    }
}
