use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;

mod builders;

use builders::*;

use crate::{
    AppError,
    agent::Agent,
    config::{ConfigLoader, ConfigResolution, LoadConfigRequest, ProcessEnvironment},
    model::ModelRef,
    model_catalog::{
        ModelCatalogConfig, ModelCatalogResponse, ModelCatalogService, ModelCatalogSnapshot,
        ModelCatalogStore, decorate_provider_models,
    },
    plugin::PluginHost,
    provider::ProviderRegistry,
    session::{
        ContextGovernor, ContextPolicy, SessionManager, SessionManagerConfig, SessionProcessor,
    },
    tool::ToolExecutor,
};

pub struct RuntimeSnapshot {
    generation: u64,
    loaded_at: DateTime<Utc>,
    resolution: Arc<ConfigResolution>,
    services: RuntimeServices,
    tasks: RuntimeTasks,
}

#[derive(Clone)]
struct RuntimeServices {
    providers: Arc<ProviderRegistry>,
    catalog_source_providers: Arc<ProviderRegistry>,
    model_catalog: Arc<ModelCatalogService>,
    plugins: Arc<PluginHost>,
    agents: crate::agents::SubagentRegistry,
    session_manager: Option<Arc<SessionManager>>,
    mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
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
    #[allow(clippy::too_many_arguments)]
    fn new(
        providers: Arc<ProviderRegistry>,
        catalog_source_providers: Arc<ProviderRegistry>,
        model_catalog: Arc<ModelCatalogService>,
        plugins: Arc<PluginHost>,
        agents: crate::agents::SubagentRegistry,
        session_manager: Option<Arc<SessionManager>>,
        mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
        lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
        event_bridge: Option<Arc<EventBridgeGuard>>,
        plugin_shutdown: Option<Arc<PluginShutdownGuard>>,
    ) -> Self {
        Self {
            providers,
            catalog_source_providers,
            model_catalog,
            plugins,
            agents,
            session_manager,
            mcp_manager,
            lsp_registry,
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
struct RuntimeTasks {
    watch_paths: Vec<PathBuf>,
    reload: RuntimeTaskPolicy,
    session_gc: RuntimeTaskPolicy,
}

impl RuntimeTasks {
    fn from_resolution(resolution: &ConfigResolution) -> Self {
        Self {
            watch_paths: collect_watch_paths(resolution),
            reload: RuntimeTaskPolicy {
                enabled: resolution.config.runtime.reload.enabled,
                interval: Duration::from_secs(resolution.config.runtime.reload.poll_interval_secs),
            },
            session_gc: RuntimeTaskPolicy {
                enabled: resolution.config.runtime.session.gc.enabled,
                interval: Duration::from_secs(resolution.config.runtime.session.gc.interval_secs),
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
    /// from the previous snapshot when the corresponding `plugins.list.<id>`
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
        let mcp_config =
            crate::plugins::provided::mcp::config_from_plugins(&resolution.config.plugins)
                .map_err(AppError::Config)?;
        let mcp_manager =
            if crate::plugins::provided::mcp::static_bridge_enabled(&resolution.config.plugins) {
                Some(crate::plugins::provided::mcp::build_manager(&mcp_config).await)
            } else {
                None
            };
        let plugins = if let Some(prev) = previous.as_ref() {
            let prev_host = prev.plugin_manager();
            let prev_cfg = prev.config_resolution().config.plugins.clone();
            resolution
                .build_plugin_host_with_previous_and_mcp(
                    Some(prev_host),
                    Some(&prev_cfg),
                    mcp_manager.clone(),
                )
                .await
                .map_err(AppError::from)?
        } else {
            resolution
                .build_plugin_host_with_previous_and_mcp(None, None, mcp_manager.clone())
                .await
                .map_err(AppError::from)?
        };
        // Make the active host visible to provider request builders for the
        // `chat.headers` hook (no constructor threading required).
        super::plugin_slot::install(Arc::clone(&plugins));
        let mut model_catalog_config = ModelCatalogConfig::default();
        model_catalog_config.cache_max_age_secs =
            resolution.config.runtime.model_catalog.cache_max_age_secs;
        let catalog_source_providers = Arc::new(
            resolution
                .build_provider_registry_with_plugins_and_catalog(plugins.as_ref(), None)
                .await?,
        );
        let catalog_store_db = database
            .as_ref()
            .cloned()
            .ok_or_else(|| AppError::Config("runtime database connection missing".to_owned()))?;
        let model_catalog_store = ModelCatalogStore::new(model_catalog_config, catalog_store_db);
        let model_catalog = Arc::new(ModelCatalogService::new(model_catalog_store).await?);
        let catalog_snapshot = model_catalog.snapshot();
        let providers = Arc::new(
            resolution
                .build_provider_registry_with_plugins_and_catalog(
                    plugins.as_ref(),
                    Some(&catalog_snapshot),
                )
                .await?,
        );
        // Notify plugins of the resolved config (best-effort).
        if !plugins.is_empty()
            && let Ok(value) = serde_json::to_value(&resolution)
        {
            let _ = plugins
                .dispatch_config(crate::plugin::ConfigInput { current: value })
                .await;
        }
        let agents = crate::agents::SubagentRegistry::discover(
            workspace_root,
            crate::agents::default_user_agents_dir().as_deref(),
        );
        register_config_agents(&agents, &resolution, &resolution.config.agents);
        let reusing_session_manager = existing_session_manager.is_some();
        let lsp_config =
            crate::plugins::provided::lsp::config_from_plugins(&resolution.config.plugins)
                .map_err(AppError::Config)?;
        let lsp_plugin_enabled = resolution
            .config
            .plugins
            .list
            .get(crate::tool::lsp_plugin_id())
            .is_some_and(|entry| !entry.disabled());
        let lsp_registry = if lsp_plugin_enabled {
            Some(build_lsp_registry(workspace_root, &lsp_config))
        } else {
            None
        };
        let session_manager = database.as_ref().map(|db| {
            build_or_reconfigure_session_manager(
                existing_session_manager,
                db,
                Arc::clone(&providers),
                Arc::clone(&plugins),
                agents.clone(),
                lsp_registry.clone(),
                workspace_root,
                &resolution,
            )
        });
        if !reusing_session_manager && let Some(manager) = session_manager.as_ref() {
            manager
                .event_publisher()
                .resume_from_store()
                .await
                .map_err(|err| {
                    AppError::Internal(format!("resume event sequence failed: {err}"))
                })?;
        }
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
            catalog_source_providers,
            model_catalog,
            plugins,
            agents,
            session_manager,
            mcp_manager,
            lsp_registry,
            event_bridge,
            plugin_shutdown,
        );
        let tasks = RuntimeTasks::from_resolution(&resolution);

        Ok(Self {
            generation,
            loaded_at: Utc::now(),
            resolution: Arc::new(resolution),
            services,
            tasks,
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

    pub fn catalog_source_provider_registry(&self) -> Arc<ProviderRegistry> {
        Arc::clone(&self.services.catalog_source_providers)
    }

    pub fn model_catalog(&self) -> Arc<ModelCatalogService> {
        Arc::clone(&self.services.model_catalog)
    }

    pub fn model_catalog_snapshot(&self) -> ModelCatalogSnapshot {
        self.services.model_catalog.snapshot()
    }

    pub fn model_catalog_response(&self) -> ModelCatalogResponse {
        self.services.model_catalog.snapshot().to_response()
    }

    pub fn mcp_manager(&self) -> Option<Arc<agena_mcp_client::McpConnectionManager>> {
        self.services.mcp_manager.clone()
    }

    pub fn lsp_registry(&self) -> Option<Arc<agena_lsp::LspRegistry>> {
        self.services.lsp_registry.clone()
    }

    pub async fn list_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::provider::ProviderModel>, AppError> {
        let models = self.services.providers.list_models(provider_id).await?;
        let adapter_ids = self
            .resolution
            .config
            .providers
            .get(provider_id)
            .map(|provider| {
                provider
                    .adapters
                    .iter()
                    .filter(|(_, adapter)| adapter.enabled)
                    .map(|(adapter_id, _)| adapter_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let Some(provider_record) = self
            .services
            .model_catalog
            .effective_provider_record(&adapter_ids)
        else {
            return Ok(models);
        };
        let provider = self
            .services
            .providers
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        Ok(decorate_provider_models(
            provider.as_ref(),
            &provider_record,
            models,
        ))
    }

    pub fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, AppError> {
        self.services.providers.resolve_model_target(target, model)
    }

    pub fn resolve_default_model(&self) -> Result<Option<ModelRef>, AppError> {
        self.services
            .providers
            .resolve_default_model_selection(&self.resolution.config.default_selection)
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

    pub fn agents(&self) -> crate::agents::SubagentRegistry {
        self.services.agents.clone()
    }

    pub fn session_manager(&self) -> Option<Arc<SessionManager>> {
        self.services.session_manager.as_ref().map(Arc::clone)
    }

    pub fn watch_paths(&self) -> &[PathBuf] {
        self.tasks.watch_paths.as_slice()
    }

    pub fn reload_enabled(&self) -> bool {
        self.tasks.reload.enabled
    }

    pub fn reload_poll_interval(&self) -> Duration {
        self.tasks.reload.interval
    }

    pub fn session_gc_enabled(&self) -> bool {
        self.tasks.session_gc.enabled
    }

    pub fn session_gc_interval(&self) -> Duration {
        self.tasks.session_gc.interval
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
