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
    plugin::PluginHost,
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
    plugins: Arc<PluginHost>,
    auth_store: RuntimeAuthStore,
    session_manager: Option<Arc<SessionManager>>,
    /// Lives for as long as this snapshot does; aborts the event bridge
    /// task when the snapshot is dropped.
    _event_bridge: Option<Arc<EventBridgeGuard>>,
    /// Drives `PluginHost::shutdown` when the snapshot is dropped.
    _plugin_shutdown: Option<Arc<PluginShutdownGuard>>,
}

struct EventBridgeGuard(tokio::task::JoinHandle<()>);

impl Drop for EventBridgeGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct PluginShutdownGuard {
    plugins: Arc<PluginHost>,
    handle: Option<tokio::runtime::Handle>,
}

impl Drop for PluginShutdownGuard {
    fn drop(&mut self) {
        let plugins = Arc::clone(&self.plugins);
        match self.handle.take() {
            Some(h) => {
                h.spawn(async move { plugins.shutdown().await });
            }
            None => {
                tracing::debug!(
                    target: "agena_plugin_host",
                    "no tokio runtime available at snapshot drop; plugins will be cleaned up by their own transports"
                );
            }
        }
    }
}

impl RuntimeServices {
    fn new(
        providers: Arc<ProviderRegistry>,
        plugins: Arc<PluginHost>,
        auth_store: RuntimeAuthStore,
        session_manager: Option<Arc<SessionManager>>,
        event_bridge: Option<Arc<EventBridgeGuard>>,
        plugin_shutdown: Option<Arc<PluginShutdownGuard>>,
    ) -> Self {
        Self {
            providers,
            plugins,
            auth_store,
            session_manager,
            _event_bridge: event_bridge,
            _plugin_shutdown: plugin_shutdown,
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
        Self::build_inner(
            generation,
            loader,
            load_request,
            workspace_root,
            database,
            existing_session_manager,
            None,
        )
        .await
    }

    /// Hot-reload variant that lets the new snapshot reuse plugin transports
    /// from the previous snapshot when the corresponding `[plugins.list.<id>]`
    /// entry is byte-identical.
    pub(crate) async fn build_with_previous(
        generation: u64,
        loader: &ConfigLoader<ProcessEnvironment>,
        load_request: &LoadConfigRequest,
        workspace_root: &Path,
        database: Option<Arc<DatabaseConnection>>,
        existing_session_manager: Option<Arc<SessionManager>>,
        previous: Arc<RuntimeSnapshot>,
    ) -> Result<Self, AppError> {
        Self::build_inner(
            generation,
            loader,
            load_request,
            workspace_root,
            database,
            existing_session_manager,
            Some(previous),
        )
        .await
    }

    async fn build_inner(
        generation: u64,
        loader: &ConfigLoader<ProcessEnvironment>,
        load_request: &LoadConfigRequest,
        workspace_root: &Path,
        database: Option<Arc<DatabaseConnection>>,
        existing_session_manager: Option<Arc<SessionManager>>,
        previous: Option<Arc<RuntimeSnapshot>>,
    ) -> Result<Self, AppError> {
        let resolution = loader.load(load_request)?;
        let providers = Arc::new(resolution.config.build_provider_registry()?);
        let plugins = if let Some(prev) = previous.as_ref() {
            let prev_host = prev.plugin_manager();
            let prev_cfg = prev.config_resolution().config.plugins.clone();
            resolution
                .build_plugin_host_with_previous(Some(prev_host), Some(&prev_cfg))
                .await
                .map_err(AppError::from)?
        } else {
            resolution.build_plugin_host().await.map_err(AppError::from)?
        };
        // Make the active host visible to provider request builders for the
        // `chat.headers` hook (no constructor threading required).
        super::plugin_slot::install(Arc::clone(&plugins));
        // Notify plugins of the resolved config (best-effort).
        if !plugins.is_empty() {
            if let Ok(value) = serde_json::to_value(&resolution) {
                let _ = plugins
                    .dispatch_config(crate::plugin::ConfigInput { current: value })
                    .await;
            }
        }
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
        let event_bridge = session_manager.as_ref().map(|mgr| {
            let handle =
                super::event_bridge::spawn_event_bridge(mgr.event_bus(), Arc::clone(&plugins));
            Arc::new(EventBridgeGuard(handle))
        });
        let plugin_shutdown = if !plugins.is_empty() {
            Some(Arc::new(PluginShutdownGuard {
                plugins: Arc::clone(&plugins),
                handle: tokio::runtime::Handle::try_current().ok(),
            }))
        } else {
            None
        };
        let services = RuntimeServices::new(
            providers,
            plugins,
            auth_store,
            session_manager,
            event_bridge,
            plugin_shutdown,
        );
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

    pub fn plugin_manager(&self) -> Arc<PluginHost> {
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
    plugins: Arc<PluginHost>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
) -> Arc<SessionManager> {
    let processor = build_session_processor(providers, Arc::clone(&plugins));
    let executor = build_tool_executor(plugins, workspace_root, resolution);
    let config = session_manager_config(resolution);

    if let Some(manager) = existing {
        manager.reconfigure(processor, executor, config);
        return manager;
    }

    Arc::new(SessionManager::new(db.as_ref().clone(), processor, executor).with_config(config))
}

fn build_session_processor(
    providers: Arc<ProviderRegistry>,
    plugins: Arc<PluginHost>,
) -> SessionProcessor {
    SessionProcessor::new(providers, ContextGovernor::new(ContextPolicy::default()))
        .with_plugin_host(plugins)
}

fn build_tool_executor(
    plugins: Arc<PluginHost>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
) -> ToolExecutor {
    let mut executor = ToolExecutor::new(
        workspace_root.to_path_buf(),
        Agent::new("build", resolution.config.permission_policy()),
    )
    .with_plugin_manager(plugins)
    .with_web_search_backend(resolution.config.web.search.resolve())
    .with_plan_registry(crate::tool::plan_registry_for_executor())
    .with_worktree_registry(crate::tool::worktree_registry_for_executor())
    .with_scheduler(build_scheduler());

    if let Ok(mgr) = agena_skills::SkillsManager::build(Some(workspace_root)) {
        executor = executor.with_skills_manager(Arc::new(mgr));
    }

    if !resolution.config.mcp.servers.is_empty() {
        let manager = build_mcp_manager(&resolution.config.mcp);
        executor = executor.with_mcp_manager(manager);
    }
    executor
}

fn build_mcp_manager(
    config: &crate::config::McpConfig,
) -> Arc<agena_mcp_client::McpConnectionManager> {
    use agena_mcp_client::{HttpTransportMode, McpConnectionManager, ServerSpec};

    let manager = Arc::new(McpConnectionManager::new(
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    ));
    // Connect each configured server in the background.  Failures only
    // disable that one server — the rest of the runtime keeps booting.
    for (name, entry) in &config.servers {
        let manager = manager.clone();
        let name = name.clone();
        let spec = match entry {
            crate::config::McpServerConfig::Stdio { command, args, env, cwd } => {
                ServerSpec::Stdio {
                    command: command.clone(),
                    args: args.clone(),
                    env: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    cwd: cwd.clone(),
                }
            }
            crate::config::McpServerConfig::Http { url, mode, headers } => {
                let parsed = match url::Url::parse(url) {
                    Ok(u) => u,
                    Err(e) => {
                        tracing::warn!(
                            target: "agena::mcp",
                            "skipping mcp server '{name}': invalid url '{url}': {e}"
                        );
                        continue;
                    }
                };
                let mode = match mode {
                    crate::config::McpHttpMode::Sse => HttpTransportMode::Sse,
                    crate::config::McpHttpMode::StreamableHttp => {
                        HttpTransportMode::StreamableHttp
                    }
                };
                ServerSpec::Http {
                    url: parsed,
                    mode,
                    headers: headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                }
            }
        };
        tokio::spawn(async move {
            if let Err(e) = manager.add_server(&name, spec).await {
                tracing::warn!(
                    target: "agena::mcp",
                    "failed to connect MCP server '{name}': {e}"
                );
            } else {
                tracing::info!(target: "agena::mcp", "connected MCP server '{name}'");
            }
        });
    }
    manager
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

    for entry in resolution.config.plugins.list.values() {
        if let crate::plugin::PluginEntry::Cdylib { path, .. } = entry {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                base_dir.join(path)
            };
            push_watch_path(&mut paths, resolved);
        }
    }

    paths
}

fn push_watch_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}

/// Build a process-wide cron scheduler.  The sink is a `tracing` logger
/// — the SessionManager bridge that actually re-injects fired prompts is
/// expected to be wired up in a follow-up commit.
fn build_scheduler() -> Arc<agena_scheduler::Scheduler> {
    use std::time::Duration;
    struct LogSink;
    #[async_trait::async_trait]
    impl agena_scheduler::JobSink for LogSink {
        async fn deliver(&self, job: &agena_scheduler::ScheduledJob) {
            tracing::info!(
                target: "agena::scheduler",
                job_id = %job.id,
                prompt = %job.prompt,
                "scheduled job fired (no SessionManager bridge yet — prompt logged only)"
            );
        }
    }
    let sched = agena_scheduler::scheduler::build_in_memory(
        Arc::new(LogSink),
        Duration::from_secs(10),
    );
    sched.start();
    sched
}
