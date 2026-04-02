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
    plugin::PluginManager,
    provider::{ProviderRegistry, auth::AuthStore},
    session::{
        ContextGovernor, ContextPolicy, SessionProcessor, SessionService, SessionServiceConfig,
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
    providers: Arc<ProviderRegistry>,
    plugins: Arc<PluginManager>,
    auth_store: RuntimeAuthStore,
    session_service: Option<Arc<SessionService>>,
    watch_paths: Vec<PathBuf>,
}

impl RuntimeSnapshot {
    pub(crate) async fn build(
        generation: u64,
        loader: &ConfigLoader<ProcessEnvironment>,
        load_request: &LoadConfigRequest,
        workspace_root: &Path,
        database: Option<Arc<DatabaseConnection>>,
    ) -> Result<Self, AppError> {
        let resolution = loader.load(load_request)?;
        let providers = Arc::new(resolution.config.build_provider_registry()?);
        let plugins = Arc::new(resolution.build_plugin_manager()?);
        let auth_store = RuntimeAuthStore::new(resolution.config.auth_store());
        let session_service = database.as_ref().map(|db| {
            build_session_service(
                db,
                Arc::clone(&providers),
                Arc::clone(&plugins),
                workspace_root,
                &resolution,
            )
        });
        let watch_paths = collect_watch_paths(&resolution);

        Ok(Self {
            generation,
            loaded_at: Utc::now(),
            resolution: Arc::new(resolution),
            providers,
            plugins,
            auth_store,
            session_service,
            watch_paths,
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
        Arc::clone(&self.providers)
    }

    pub fn plugin_manager(&self) -> Arc<PluginManager> {
        Arc::clone(&self.plugins)
    }

    pub fn auth_store(&self) -> RuntimeAuthStore {
        self.auth_store.clone()
    }

    pub fn session_service(&self) -> Option<Arc<SessionService>> {
        self.session_service.as_ref().map(Arc::clone)
    }

    pub(crate) fn watch_paths(&self) -> &[PathBuf] {
        &self.watch_paths
    }

    pub(crate) fn reload_enabled(&self) -> bool {
        self.resolution.config.runtime.reload.enabled
    }

    pub(crate) fn reload_poll_interval(&self) -> Duration {
        Duration::from_secs(self.resolution.config.runtime.reload.poll_interval_secs)
    }

    pub(crate) fn janitor_enabled(&self) -> bool {
        self.resolution.config.runtime.janitor.enabled
    }

    pub(crate) fn janitor_interval(&self) -> Duration {
        Duration::from_secs(self.resolution.config.runtime.janitor.interval_secs)
    }
}

impl fmt::Debug for RuntimeSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSnapshot")
            .field("generation", &self.generation)
            .field("loaded_at", &self.loaded_at)
            .field("config_path", &self.resolution.meta.config_path)
            .field("provider_count", &self.providers.provider_ids().len())
            .field("plugin_count", &self.plugins.plugins().len())
            .field("session_service", &self.session_service.is_some())
            .finish()
    }
}

fn build_session_service(
    db: &Arc<DatabaseConnection>,
    providers: Arc<ProviderRegistry>,
    plugins: Arc<PluginManager>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
) -> Arc<SessionService> {
    let processor =
        SessionProcessor::new(providers, ContextGovernor::new(ContextPolicy::default()));
    let executor = ToolExecutor::new(
        workspace_root.to_path_buf(),
        Agent::new("build", resolution.config.permission_policy()),
    )
    .with_plugin_manager(plugins);
    let service = SessionService::new(db.as_ref().clone(), processor, executor).with_config(
        SessionServiceConfig {
            cache_max_sessions: resolution.config.runtime.session_cache.max_sessions,
            cache_ttl: Duration::from_secs(resolution.config.runtime.session_cache.ttl_secs),
            cache_max_bytes: resolution.config.runtime.session_cache.max_bytes,
            max_turn_loops: SessionServiceConfig::default().max_turn_loops,
        },
    );
    Arc::new(service)
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
