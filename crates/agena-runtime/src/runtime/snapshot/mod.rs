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
    authorization::ExecutionPrincipal,
    config::{ConfigLoader, LoadConfigRequest, ProcessEnvironment},
    provider::{ProviderRegistry, catalog_decoration_source},
    session::{ContextGovernor, SessionManager, SessionProcessor},
    tool::ToolExecutor,
};
use agena_domain::ModelRef;
use agena_plugin_host::PluginHost;
use agena_provider::{ModelCatalogResponse, decorate_provider_models};
use agena_runtime::{
    ConfigResolutionMeta, ModelCatalogService, ResolvedConfig, ResolvedProviderConfig, UiConfig,
};

pub(super) type RuntimeServices = agena_runtime::RuntimeServiceBundle<
    Arc<ProviderRegistry>,
    Arc<ProviderRegistry>,
    Arc<ModelCatalogService>,
    Arc<PluginHost>,
    Option<Arc<SessionManager>>,
    Option<Arc<agena_mcp_client::McpConnectionManager>>,
    Option<Arc<agena_lsp::LspRegistry>>,
>;

type SnapshotState = agena_runtime::RuntimeSnapshotState<Arc<ResolvedConfig>, RuntimeServices>;

/// Connections handed to snapshot composition. The chat database backs
/// session and model-catalog storage; the scheduler database backs the cron
/// scheduler (`None` degrades the scheduler to its in-memory store).
#[derive(Clone, Default)]
pub(crate) struct SnapshotDatabases {
    pub(crate) chat: Option<Arc<DatabaseConnection>>,
    pub(crate) scheduler: Option<Arc<DatabaseConnection>>,
}

pub(crate) struct RuntimeSnapshot {
    state: SnapshotState,
    resolution_meta: ConfigResolutionMeta,
}

impl RuntimeSnapshot {
    pub(crate) async fn build(
        generation: u64,
        loader: &ConfigLoader<ProcessEnvironment>,
        load_request: &LoadConfigRequest,
        workspace_root: &Path,
        databases: SnapshotDatabases,
        existing_session_manager: Option<Arc<SessionManager>>,
        monitor_registry: Option<Arc<dyn crate::MonitorService>>,
    ) -> Result<Self, AppError> {
        Self::build_inner(
            generation,
            loader,
            load_request,
            workspace_root,
            databases,
            existing_session_manager,
            None,
            monitor_registry,
        )
        .await
    }

    /// Hot-reload variant that lets the new snapshot reuse plugin transports
    /// from the previous snapshot when the corresponding `plugins.list.<id>`
    /// entry is byte-identical. In-proc `Static` plugins are the exception:
    /// their instance binds the host handle during `meta/init`, so they are
    /// always recreated against the new handle instead of reused, keeping
    /// display contributions (plan chip, terminal activity) on the live host.
    pub(crate) async fn build_with_previous(
        generation: u64,
        loader: &ConfigLoader<ProcessEnvironment>,
        load_request: &LoadConfigRequest,
        workspace_root: &Path,
        databases: SnapshotDatabases,
        existing_session_manager: Option<Arc<SessionManager>>,
        previous: Arc<RuntimeSnapshot>,
        monitor_registry: Option<Arc<dyn crate::MonitorService>>,
    ) -> Result<Self, AppError> {
        Self::build_inner(
            generation,
            loader,
            load_request,
            workspace_root,
            databases,
            existing_session_manager,
            Some(previous),
            monitor_registry,
        )
        .await
    }

    async fn build_inner(
        generation: u64,
        loader: &ConfigLoader<ProcessEnvironment>,
        load_request: &LoadConfigRequest,
        workspace_root: &Path,
        databases: SnapshotDatabases,
        existing_session_manager: Option<Arc<SessionManager>>,
        previous: Option<Arc<RuntimeSnapshot>>,
        monitor_registry: Option<Arc<dyn crate::MonitorService>>,
    ) -> Result<Self, AppError> {
        let SnapshotDatabases {
            chat: database,
            scheduler: scheduler_database,
        } = databases;
        let mut resolution = loader.load(load_request)?;
        // Bundled implementations are a composition concern: the pure
        // config crate cannot depend on concrete plugin factories. Inject the
        // bundled entries before any plugin-dependent service is built so
        // user entries still override the bundled defaults by plugin id.
        resolution.config.plugins = agena_runtime::merge_bundled_plugin_config(
            resolution.config.plugins.clone(),
            agena_bundled_plugins::plugins::sources::bundled_plugin_entries(),
        )
        .map_err(AppError::Config)?;
        agena_runtime::set_provider_client_versions(agena_provider::ProviderClientVersions {
            codex: resolution
                .config
                .runtime
                .providers
                .client_versions
                .codex
                .clone(),
            claude: resolution
                .config
                .runtime
                .providers
                .client_versions
                .claude
                .clone(),
            gemini: resolution
                .config
                .runtime
                .providers
                .client_versions
                .gemini
                .clone(),
        });
        let mcp_manager = agena_runtime::build_configured_mcp_manager(
            &resolution.config.plugins,
            agena_runtime::RUNTIME_CODEX_MCP_CLIENT_NAME,
            agena_runtime::codex_package_version(),
            workspace_root,
        )
        .await
        .map_err(AppError::Config)?;
        let (previous_host, previous_config) = previous
            .as_ref()
            .map(|prev| {
                (
                    Some(prev.plugin_manager()),
                    Some(prev.plugin_config().clone()),
                )
            })
            .unwrap_or((None, None));
        let plugins = build_plugin_services(agena_runtime::PluginCompositionInputs {
            plugin_config: resolution.config.plugins.clone(),
            workspace_root: resolution
                .meta
                .config_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
            previous_host,
            previous_config,
            mcp_manager: mcp_manager.clone(),
        })
        .await?;
        let (catalog_source_providers, model_catalog) =
            build_model_catalog_services(agena_runtime::ModelCatalogCompositionInputs {
                providers: &resolution.config.providers,
                config_path: resolution.meta.config_path.as_path(),
                plugins: plugins.as_ref(),
                database: database.clone(),
            })
            .await?;
        let catalog_snapshot = model_catalog.snapshot();
        let providers = build_runtime_provider_registry(
            &resolution.config.providers,
            resolution.meta.config_path.as_path(),
            plugins.as_ref(),
            &catalog_snapshot,
        )
        .await?;
        // Notify plugins of the resolved config (best-effort).
        if let Ok(value) = agena_runtime::config_resolution_json_value(&resolution) {
            agena_runtime::dispatch_config_if_nonempty(Arc::clone(&plugins), value).await;
        }
        let (lsp_registry, lsp_registration) = agena_runtime::compose_lsp_services(
            &resolution.config.plugins,
            workspace_root,
            agena_runtime::RUNTIME_CODEX_ORIGINATOR,
            agena_runtime::codex_package_version(),
        )
        .map_err(AppError::Config)?;
        let session_build_config =
            agena_runtime::session_build_config_from_resolved(&resolution.config);
        let session_manager = database.as_ref().map(|db| {
            build_or_reconfigure_session_manager(agena_runtime::SessionCompositionInputs {
                existing: existing_session_manager,
                database: db,
                providers: Arc::clone(&providers),
                plugins: Arc::clone(&plugins),
                lsp_registry: lsp_registry.clone(),
                workspace_root,
                config: &session_build_config,
                mcp_manager: mcp_manager.clone(),
                monitor_registry: monitor_registry.clone(),
                scheduler_database: scheduler_database.clone(),
            })
        });
        // v2 has no persisted event store to resume (14.3): interrupted-run
        // reconciliation is deferred to `SessionManager::get_session` on open
        // (17.4), so there is nothing to do here.
        let plugin_shutdown = agena_runtime::plugin_shutdown_guard(Arc::clone(&plugins));
        let services = agena_runtime::RuntimeServiceBundle::new(
            providers,
            catalog_source_providers,
            model_catalog,
            plugins,
            session_manager,
            mcp_manager,
            lsp_registry,
            lsp_registration,
            None,
            plugin_shutdown,
        );
        let tasks = agena_runtime::RuntimeTaskState::new(agena_runtime::runtime_watch_paths(
            resolution.meta.config_path.as_path(),
            resolution.meta.project_config_path.as_path(),
            &resolution.config.plugins,
        ));

        let config = resolution.config;
        let meta = resolution.meta;
        Ok(Self {
            state: agena_runtime::compose_runtime_snapshot_state(
                agena_runtime::RuntimeSnapshotCompositionInputs {
                    generation,
                    resolution: Arc::new(config),
                    services,
                    tasks,
                },
            ),
            resolution_meta: meta,
        })
    }

    pub(crate) fn generation(&self) -> u64 {
        self.state.metadata().generation()
    }

    pub(crate) fn loaded_at(&self) -> DateTime<Utc> {
        self.state.metadata().loaded_at()
    }

    pub(crate) fn provider_configs(
        &self,
    ) -> &std::collections::BTreeMap<String, ResolvedProviderConfig> {
        &self.state.resolution().providers
    }

    pub(crate) fn plugin_storage(&self) -> Arc<dyn crate::plugins::storage::PluginStorage> {
        Arc::new(crate::plugins::storage::FilePluginStorage::new(
            crate::plugins::storage::default_storage_root(),
        ))
    }

    pub(crate) fn plugin_secret_store(
        &self,
    ) -> Arc<dyn crate::plugins::storage::PluginSecretStore> {
        Arc::new(crate::plugins::storage::PluginKeyringSecretStore::system(
            crate::plugins::storage::default_storage_root(),
            true,
        ))
    }

    pub(crate) fn config_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        agena_runtime::config_resolution_json_value(&agena_runtime::ConfigResolution {
            config: self.state.resolution().as_ref().clone(),
            meta: self.resolution_meta.clone(),
        })
    }

    pub(crate) fn resolved_config_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        agena_runtime::resolved_config_json_value(self.state.resolution().as_ref())
    }

    pub(crate) fn tracing_config(&self) -> &agena_runtime::RuntimeTracingConfiguration {
        &self.state.resolution().tracing
    }

    pub(crate) fn plugin_config(&self) -> &agena_plugin_host::PluginsConfig {
        &self.state.resolution().plugins
    }

    pub(crate) fn default_provider(&self) -> Option<&str> {
        self.state
            .resolution()
            .default_selection
            .provider
            .as_deref()
    }

    pub(crate) fn default_selection(&self) -> agena_domain::ExecutionSelection {
        self.state.resolution().default_selection.clone()
    }

    pub(crate) fn ui_config(&self) -> UiConfig {
        self.state.resolution().ui.clone()
    }

    pub(crate) fn config_path(&self) -> &Path {
        self.resolution_meta.config_path.as_path()
    }

    pub(crate) fn project_config_path(&self) -> &Path {
        self.resolution_meta.project_config_path.as_path()
    }

    pub(crate) fn config_found(&self) -> bool {
        self.resolution_meta.config_found
    }

    pub(crate) fn project_config_found(&self) -> bool {
        self.resolution_meta.project_config_found
    }

    pub(crate) fn applied_layer_descriptions(&self) -> Vec<String> {
        self.resolution_meta.applied_layer_descriptions()
    }

    pub(crate) fn provider_registry(&self) -> Arc<ProviderRegistry> {
        Arc::clone(&self.state.services().providers)
    }

    pub(crate) fn catalog_source_provider_registry(&self) -> Arc<ProviderRegistry> {
        Arc::clone(&self.state.services().catalog_source_providers)
    }

    pub(crate) fn model_catalog(&self) -> Arc<ModelCatalogService> {
        Arc::clone(&self.state.services().model_catalog)
    }

    pub(crate) fn model_catalog_response(&self) -> ModelCatalogResponse {
        self.state.services().model_catalog.snapshot().to_response()
    }

    pub(crate) fn mcp_manager(&self) -> Option<Arc<agena_mcp_client::McpConnectionManager>> {
        self.state.services().mcp_manager.clone()
    }

    pub(crate) fn lsp_registry(&self) -> Option<Arc<agena_lsp::LspRegistry>> {
        self.state.services().lsp_registry.clone()
    }

    pub(crate) fn configured_local_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<agena_domain::Model>, AppError> {
        let Some(configured) = self.provider_configs().get(provider_id) else {
            return Ok(Vec::new());
        };
        let enabled_adapter_ids = agena_runtime::configured_enabled_adapter_ids(configured);
        let models = agena_runtime::configured_local_models(provider_id, configured);
        let provider = self
            .state
            .services()
            .providers
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        let provider_record = self
            .state
            .services()
            .model_catalog
            .effective_provider_record(&enabled_adapter_ids)
            .unwrap_or_default();
        let local_provider_record = agena_provider::ModelCatalogProviderRecord {
            models: provider_record.models,
            appendable_model_ids: Default::default(),
        };
        Ok(decorate_provider_models(
            &catalog_decoration_source(provider.as_ref()),
            &local_provider_record,
            models,
        ))
    }

    pub(crate) async fn list_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<agena_domain::Model>, AppError> {
        let models = self
            .state
            .services()
            .providers
            .list_models(provider_id)
            .await?;
        let adapter_ids = self
            .provider_configs()
            .get(provider_id)
            .map(agena_runtime::configured_enabled_adapter_ids)
            .unwrap_or_default();
        let Some(provider_record) = self
            .state
            .services()
            .model_catalog
            .effective_provider_record(&adapter_ids)
        else {
            return Ok(models);
        };
        let provider = self
            .state
            .services()
            .providers
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        Ok(decorate_provider_models(
            &catalog_decoration_source(provider.as_ref()),
            &provider_record,
            models,
        ))
    }

    pub(crate) fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, AppError> {
        Ok(self
            .state
            .services()
            .providers
            .resolve_model_target(target, model)?)
    }

    pub(crate) fn resolve_default_model(&self) -> Result<Option<ModelRef>, AppError> {
        Ok(self
            .state
            .services()
            .providers
            .resolve_default_model_selection(&self.state.resolution().default_selection)?)
    }

    pub(crate) fn plugin_manager(&self) -> Arc<PluginHost> {
        Arc::clone(&self.state.services().plugins)
    }

    pub(crate) fn session_manager(&self) -> Option<Arc<SessionManager>> {
        self.state
            .services()
            .session_manager
            .as_ref()
            .map(Arc::clone)
    }

    pub(crate) fn watch_paths(&self) -> &[PathBuf] {
        self.state.tasks().watch_paths().as_slice()
    }

    pub(crate) fn reload_enabled(&self) -> bool {
        self.state.tasks().scheduling().reload_enabled
    }

    pub(crate) fn reload_poll_interval(&self) -> Duration {
        self.state.tasks().scheduling().reload_poll_interval
    }

    pub(crate) fn session_gc_enabled(&self) -> bool {
        self.state.tasks().scheduling().session_gc_enabled
    }

    pub(crate) fn session_gc_interval(&self) -> Duration {
        self.state.tasks().scheduling().session_gc_interval
    }
}

impl fmt::Debug for RuntimeSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSnapshot")
            .field("generation", &self.state.metadata().generation())
            .field("loaded_at", &self.state.metadata().loaded_at())
            .field("config_path", &self.resolution_meta.config_path)
            .field(
                "provider_count",
                &self.state.services().providers.provider_ids().len(),
            )
            .field(
                "plugin_count",
                &self.state.services().plugins.plugins().len(),
            )
            .field(
                "session_manager",
                &self.state.services().session_manager.is_some(),
            )
            .finish()
    }
}
