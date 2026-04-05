use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;

use crate::{
    AppError,
    agent::Agent,
    config::{ConfigLoader, ConfigResolution, LoadConfigRequest, ProcessEnvironment},
    model::ModelRef,
    plugin::PluginManager,
    provider::{ProviderRegistry, auth::AuthStore},
    session::{
        ContextGovernor, ContextPolicy, SessionManager, SessionManagerConfig, SessionProcessor,
    },
    tool::ToolExecutor,
};

#[derive(Clone)]
pub struct RuntimeAuthStore {
    inner: Arc<dyn AuthStore>,
}

impl RuntimeAuthStore {
    pub fn new(store: impl AuthStore + 'static) -> Self {
        Self {
            inner: Arc::new(store),
        }
    }

    pub fn inner(&self) -> &Arc<dyn AuthStore> {
        &self.inner
    }
}

impl fmt::Debug for RuntimeAuthStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeAuthStore").finish_non_exhaustive()
    }
}

impl AuthStore for RuntimeAuthStore {
    fn all(
        &self,
    ) -> Result<std::collections::HashMap<String, crate::provider::auth::AuthData>, AppError> {
        self.inner.all()
    }

    fn get(&self, provider_id: &str) -> Result<Option<crate::provider::auth::AuthData>, AppError> {
        self.inner.get(provider_id)
    }

    fn set(
        &self,
        provider_id: &str,
        auth: crate::provider::auth::AuthData,
    ) -> Result<(), AppError> {
        self.inner.set(provider_id, auth)
    }

    fn remove(&self, provider_id: &str) -> Result<(), AppError> {
        self.inner.remove(provider_id)
    }
}

pub struct RuntimeSnapshot {
    generation: u64,
    loaded_at: DateTime<Utc>,
    resolution: Arc<ConfigResolution>,
    services: RuntimeServices,
    maintenance: RuntimeMaintenance,
}

#[derive(Clone)]
struct RuntimeServices {
    providers: Arc<ProviderRegistry>,
    plugins: Arc<PluginManager>,
    auth_store: RuntimeAuthStore,
    session_manager: Option<Arc<SessionManager>>,
}

impl RuntimeServices {
    fn new(
        providers: Arc<ProviderRegistry>,
        plugins: Arc<PluginManager>,
        auth_store: RuntimeAuthStore,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Self {
        Self {
            providers,
            plugins,
            auth_store,
            session_manager,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeTaskPolicy {
    enabled: bool,
    interval: Duration,
}

#[derive(Debug, Clone)]
struct RuntimeMaintenance {
    watch_paths: Vec<PathBuf>,
    reload: RuntimeTaskPolicy,
    janitor: RuntimeTaskPolicy,
}

impl RuntimeMaintenance {
    fn from_resolution(resolution: &ConfigResolution) -> Self {
        Self {
            watch_paths: collect_watch_paths(resolution),
            reload: RuntimeTaskPolicy {
                enabled: resolution.config.runtime.reload.enabled,
                interval: Duration::from_secs(resolution.config.runtime.reload.poll_interval_secs),
            },
            janitor: RuntimeTaskPolicy {
                enabled: resolution.config.runtime.janitor.enabled,
                interval: Duration::from_secs(resolution.config.runtime.janitor.interval_secs),
            },
        }
    }
}

impl RuntimeSnapshot {
    pub(crate) async fn build(
        generation: u64,
        loader: &ConfigLoader<ProcessEnvironment>,
        load_request: &LoadConfigRequest,
        workspace_root: &Path,
        database: Option<Arc<DatabaseConnection>>,
        existing_session_manager: Option<Arc<SessionManager>>,
    ) -> Result<Self, AppError> {
        let resolution = loader.load(load_request)?;
        let providers = Arc::new(resolution.config.build_provider_registry()?);
        let plugins = Arc::new(resolution.build_plugin_manager()?);
        let auth_store = RuntimeAuthStore::new(resolution.config.auth_store());
        let session_manager = if let Some(db) = database.as_ref() {
            Some(build_or_reconfigure_session_manager(
                existing_session_manager,
                db,
                Arc::clone(&providers),
                Arc::clone(&plugins),
                workspace_root,
                &resolution,
            ))
        } else {
            None
        };
        let services = RuntimeServices::new(providers, plugins, auth_store, session_manager);
        let maintenance = RuntimeMaintenance::from_resolution(&resolution);

        Ok(Self {
            generation,
            loaded_at: Utc::now(),
            resolution: Arc::new(resolution),
            services,
            maintenance,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn loaded_at(&self) -> DateTime<Utc> {
        self.loaded_at.to_owned()
    }

    pub fn config_resolution(&self) -> &ConfigResolution {
        self.resolution.as_ref()
    }

    pub fn provider_registry(&self) -> Arc<ProviderRegistry> {
        Arc::clone(&self.services.providers)
    }

    pub async fn list_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::provider::ProviderModel>, AppError> {
        self.services.providers.list_models(provider_id).await
    }

    pub fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, AppError> {
        self.services.providers.resolve_model_target(target, model)
    }

    pub async fn resolve_model(
        &self,
        model: &ModelRef,
    ) -> Result<crate::provider::ProviderModel, AppError> {
        self.services.providers.resolve_model(model).await
    }

    pub fn model_capabilities_for(
        &self,
        model: &ModelRef,
    ) -> Result<crate::provider::ModelCapabilities, AppError> {
        self.services.providers.model_capabilities(model)
    }

    pub async fn provider_model(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<crate::provider::ProviderModel, AppError> {
        self.resolve_model(&ModelRef::new(provider_id, model)).await
    }

    pub fn model_capabilities(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<crate::provider::ModelCapabilities, AppError> {
        self.model_capabilities_for(&ModelRef::new(provider_id, model))
    }

    pub fn plugin_manager(&self) -> Arc<PluginManager> {
        Arc::clone(&self.services.plugins)
    }

    pub fn auth_store(&self) -> RuntimeAuthStore {
        self.services.auth_store.clone()
    }

    pub fn session_manager(&self) -> Option<Arc<SessionManager>> {
        self.services.session_manager.as_ref().map(Arc::clone)
    }

    pub fn watch_paths(&self) -> &[PathBuf] {
        self.maintenance.watch_paths.as_slice()
    }

    pub fn reload_enabled(&self) -> bool {
        self.maintenance.reload.enabled
    }

    pub fn reload_poll_interval(&self) -> Duration {
        self.maintenance.reload.interval
    }

    pub fn janitor_enabled(&self) -> bool {
        self.maintenance.janitor.enabled
    }

    pub fn janitor_interval(&self) -> Duration {
        self.maintenance.janitor.interval
    }
}

impl fmt::Debug for RuntimeSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSnapshot")
            .field("generation", &self.generation)
            .field("loaded_at", &self.loaded_at)
            .field("config_path", &self.resolution.meta.config_path)
            .field(
                "provider_count",
                &self.services.providers.provider_ids().len(),
            )
            .field("plugin_count", &self.services.plugins.plugins().len())
            .field("session_manager", &self.services.session_manager.is_some())
            .finish()
    }
}

fn build_or_reconfigure_session_manager(
    existing: Option<Arc<SessionManager>>,
    db: &Arc<DatabaseConnection>,
    providers: Arc<ProviderRegistry>,
    plugins: Arc<PluginManager>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
) -> Arc<SessionManager> {
    let processor = build_session_processor(providers);
    let executor = build_tool_executor(plugins, workspace_root, resolution);
    let config = session_manager_config(resolution);

    if let Some(manager) = existing {
        manager.reconfigure(processor, executor, config);
        return manager;
    }

    Arc::new(SessionManager::new(db.as_ref().clone(), processor, executor).with_config(config))
}

fn build_session_processor(providers: Arc<ProviderRegistry>) -> SessionProcessor {
    SessionProcessor::new(providers, ContextGovernor::new(ContextPolicy::default()))
}

fn build_tool_executor(
    plugins: Arc<PluginManager>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
) -> ToolExecutor {
    ToolExecutor::new(
        workspace_root.to_path_buf(),
        Agent::new("build", resolution.config.permission_policy()),
    )
    .with_plugin_manager(plugins)
}

fn session_manager_config(resolution: &ConfigResolution) -> SessionManagerConfig {
    SessionManagerConfig {
        cache_max_sessions: resolution.config.runtime.session_cache.max_sessions,
        cache_ttl: Duration::from_secs(resolution.config.runtime.session_cache.ttl_secs),
        cache_max_bytes: resolution.config.runtime.session_cache.max_bytes,
        max_turn_loops: SessionManagerConfig::default().max_turn_loops,
    }
}

fn collect_watch_paths(resolution: &ConfigResolution) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_watch_path(&mut paths, resolution.meta.config_path.clone());
    push_watch_path(&mut paths, resolution.config.auth.store_path.clone());

    let base_dir = resolution
        .meta
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let explicit_paths = !resolution.config.plugins.paths.is_empty();
    let plugin_paths = if explicit_paths {
        resolution.config.plugins.paths.clone()
    } else {
        vec![PathBuf::from("plugins")]
    };

    for path in plugin_paths {
        let resolved = if path.is_absolute() {
            path
        } else {
            base_dir.join(path)
        };
        push_watch_path(&mut paths, resolved);
    }

    paths
}

fn push_watch_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}
