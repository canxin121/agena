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
    ConfigResolutionMeta, ModelCatalogService, ResolvedConfig, ResolvedProviderConfig,
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
    pub(crate) chat: Option<Arc<agena_storage_sqlite::SqliteKernel>>,
    pub(crate) scheduler: Option<Arc<DatabaseConnection>>,
}

pub(crate) struct RuntimeSnapshot {
    client_identity: crate::ProviderClientIdentity,
    plugin_shutdown: agena_runtime::PluginShutdownCoordinator,
    global_plugins: Arc<PluginHost>,
    global_plugin_config: agena_plugin_host::PluginsConfig,
    _global_mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    _global_plugin_shutdown: Option<Arc<agena_runtime::CallbackOnDrop>>,
    _global_lsp_registration: Option<Arc<agena_runtime::AbortOnDrop>>,
    state: SnapshotState,
    resolution_meta: ConfigResolutionMeta,
    pub(super) host_client: Arc<super::host_client::RuntimeHostClient>,
    pub(super) global_host_client: Arc<super::host_client::RuntimeHostClient>,
    callback_server: Arc<super::callback_server::PluginCallbackServer>,
    pending_session_reconfiguration: parking_lot::Mutex<Option<PendingSessionReconfiguration>>,
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
            chat: session_kernel,
            scheduler: scheduler_database,
        } = databases;
        let database = session_kernel
            .as_ref()
            .map(|kernel| kernel.database.clone());
        let global_loader = loader.clone();
        let global_request = load_request.clone();
        let mut global_resolution = crate::blocking::FILE_OPERATIONS
            .run(move || global_loader.load_global(&global_request))
            .await
            .map_err(|error| {
                AppError::Internal(format!("global config load worker failed: {error}"))
            })??;
        let worker_loader = loader.clone();
        let request = load_request.clone();
        let mut resolution = crate::blocking::FILE_OPERATIONS
            .run(move || worker_loader.load(&request))
            .await
            .map_err(|error| AppError::Internal(format!("config load worker failed: {error}")))??;
        // Bundled implementations are a composition concern: the pure
        // config crate cannot depend on concrete plugin factories. Inject the
        // bundled entries before any plugin-dependent service is built so
        // user entries still override the bundled defaults by plugin id.
        resolution.config.plugins = agena_runtime::merge_bundled_plugin_config(
            resolution.config.plugins.clone(),
            agena_bundled_plugins::plugins::sources::bundled_plugin_entries(),
        )
        .map_err(AppError::Config)?;
        global_resolution.config.plugins = agena_runtime::merge_bundled_plugin_config(
            global_resolution.config.plugins.clone(),
            agena_bundled_plugins::plugins::sources::bundled_plugin_entries(),
        )
        .map_err(AppError::Config)?;
        // Only process initialization uses this instance location. Per-session
        // executors clear it when binding a global session.
        let global_internal_root = global_resolution
            .meta
            .config_path
            .parent()
            .filter(|path| path.is_absolute())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| {
                agena_storage_layout::StorageLayout::current()
                    .root()
                    .to_path_buf()
            });
        let client_identity =
            crate::ProviderClientIdentity::new(agena_provider::ProviderClientVersions {
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
            client_identity.codex_package_version(),
            workspace_root,
        )
        .await
        .map_err(AppError::Config)?;
        let global_mcp_manager = agena_runtime::build_configured_mcp_manager(
            &global_resolution.config.plugins,
            agena_runtime::RUNTIME_CODEX_MCP_CLIENT_NAME,
            client_identity.codex_package_version(),
            &global_internal_root,
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
        let callback_server = match &previous {
            Some(previous) => previous.callback_server.clone(),
            None => super::callback_server::PluginCallbackServer::start().await?,
        };
        let host_client = super::host_client::RuntimeHostClient::candidate(
            generation,
            agena_runtime::config_resolution_json_value(&resolution)?,
        );
        let global_host_client = super::host_client::RuntimeHostClient::candidate(
            generation,
            agena_runtime::config_resolution_json_value(&global_resolution)?,
        );
        let plugins = build_plugin_services(
            agena_runtime::PluginCompositionInputs {
                plugin_config: resolution.config.plugins.clone(),
                workspace_root: resolution
                    .meta
                    .config_path
                    .parent()
                    .unwrap_or_else(|| Path::new(".")),
                previous_host,
                previous_config,
                mcp_manager: mcp_manager.clone(),
                callback_base_url: callback_server.base_url.clone(),
                callback_dispatcher: callback_server.dispatcher.clone(),
                host_client: host_client.clone(),
            },
            session_kernel
                .as_ref()
                .map(|kernel| agena_bundled_plugins::tool::NotificationSource {
                    reader: kernel.ports.reader.clone(),
                    state: kernel.instance_state.clone(),
                }),
        )
        .await?;
        // Own successful plugin initialization across every later service
        // build error and cancellation, before awaiting provider composition.
        let plugin_shutdown_coordinator = previous
            .as_ref()
            .map(|previous| previous.plugin_shutdown.clone())
            .unwrap_or_default();
        let plugin_shutdown = match plugin_shutdown_coordinator.register(Arc::clone(&plugins)) {
            Ok(guard) => guard,
            Err(mut report) => {
                report
                    .failures
                    .extend(plugins.shutdown_report().await.failures);
                return Err(AppError::Internal(report.to_string()));
            }
        };
        let global_plugins = build_plugin_services(
            agena_runtime::PluginCompositionInputs {
                plugin_config: global_resolution.config.plugins.clone(),
                workspace_root: &global_internal_root,
                previous_host: previous
                    .as_ref()
                    .map(|prev| Arc::clone(&prev.global_plugins)),
                previous_config: previous
                    .as_ref()
                    .map(|prev| prev.global_plugin_config.clone()),
                mcp_manager: global_mcp_manager.clone(),
                callback_base_url: callback_server.base_url.clone(),
                callback_dispatcher: callback_server.dispatcher.clone(),
                host_client: global_host_client.clone(),
            },
            session_kernel
                .as_ref()
                .map(|kernel| agena_bundled_plugins::tool::NotificationSource {
                    reader: kernel.ports.reader.clone(),
                    state: kernel.instance_state.clone(),
                }),
        )
        .await?;
        let global_plugin_shutdown =
            match plugin_shutdown_coordinator.register(Arc::clone(&global_plugins)) {
                Ok(guard) => guard,
                Err(mut report) => {
                    report
                        .failures
                        .extend(global_plugins.shutdown_report().await.failures);
                    return Err(AppError::Internal(report.to_string()));
                }
            };
        let (catalog_source_providers, model_catalog) = build_model_catalog_services(
            agena_runtime::ModelCatalogCompositionInputs {
                providers: &resolution.config.providers,
                config_path: resolution.meta.config_path.as_path(),
                plugins: plugins.as_ref(),
                database: database.clone(),
            },
            &client_identity,
        )
        .await?;
        let catalog_snapshot = model_catalog.snapshot();
        let providers = build_runtime_provider_registry(
            &resolution.config.providers,
            resolution.meta.config_path.as_path(),
            plugins.as_ref(),
            &catalog_snapshot,
            &client_identity,
        )
        .await?;
        let global_providers = build_runtime_provider_registry(
            &global_resolution.config.providers,
            global_resolution.meta.config_path.as_path(),
            global_plugins.as_ref(),
            &catalog_snapshot,
            &client_identity,
        )
        .await?;
        let global_session_config =
            agena_runtime::session_build_config_from_resolved(&global_resolution.config);
        // Notify plugins of the resolved config (best-effort).
        if let Ok(value) = agena_runtime::config_resolution_json_value(&resolution) {
            agena_runtime::dispatch_config_if_nonempty(Arc::clone(&plugins), value).await;
        }
        if let Ok(value) = agena_runtime::config_resolution_json_value(&global_resolution) {
            agena_runtime::dispatch_config_if_nonempty(Arc::clone(&global_plugins), value).await;
        }
        let (global_lsp_registry, global_lsp_registration) = agena_runtime::compose_lsp_services(
            &global_resolution.config.plugins,
            &global_internal_root,
            agena_runtime::RUNTIME_CODEX_ORIGINATOR,
            client_identity.codex_package_version(),
        )
        .map_err(AppError::Config)?;
        let (lsp_registry, lsp_registration) = agena_runtime::compose_lsp_services(
            &resolution.config.plugins,
            workspace_root,
            agena_runtime::RUNTIME_CODEX_ORIGINATOR,
            client_identity.codex_package_version(),
        )
        .map_err(AppError::Config)?;
        let session_build_config =
            agena_runtime::session_build_config_from_resolved(&resolution.config);
        let (session_manager, pending_session_reconfiguration) =
            if let Some(db) = session_kernel.as_ref() {
                let (manager, pending) = build_or_prepare_session_manager(
                    agena_runtime::SessionCompositionInputs {
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
                    },
                    global_providers.clone(),
                    Arc::clone(&global_plugins),
                    global_mcp_manager.clone(),
                    global_lsp_registry,
                    &global_internal_root,
                    &global_session_config,
                );
                (Some(manager), pending)
            } else {
                (None, None)
            };
        // there is no persisted event store to resume: interrupted-run
        // reconciliation is deferred to `SessionManager::get_session` on open
        // (17.4), so there is nothing to do here.
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
            plugin_shutdown: plugin_shutdown_coordinator,
            global_plugins,
            global_plugin_config: global_resolution.config.plugins,
            _global_mcp_manager: global_mcp_manager,
            _global_plugin_shutdown: global_plugin_shutdown,
            _global_lsp_registration: global_lsp_registration,
            state: agena_runtime::compose_runtime_snapshot_state(
                agena_runtime::RuntimeSnapshotCompositionInputs {
                    generation,
                    resolution: Arc::new(config),
                    services,
                    tasks,
                },
            ),
            resolution_meta: meta,
            client_identity,
            host_client,
            global_host_client,
            callback_server,
            pending_session_reconfiguration: parking_lot::Mutex::new(
                pending_session_reconfiguration,
            ),
        })
    }

    pub(super) fn global_plugin_manager(&self) -> Arc<PluginHost> {
        Arc::clone(&self.global_plugins)
    }

    /// Apply a prepared shared-manager update only while the caller owns the
    /// runtime publication guard. Dropping an unpublished snapshot leaves the
    /// live manager and its generation ownership unchanged.
    pub(super) fn publish_session_configuration(&self) {
        let pending = self.pending_session_reconfiguration.lock().take();
        if let Some(pending) = pending {
            pending.publish(&self.session_manager().expect("prepared session manager"));
        }
    }

    pub(super) fn client_identity(&self) -> &crate::ProviderClientIdentity {
        &self.client_identity
    }

    pub(crate) fn provider_client_versions_auto_update(&self) -> bool {
        self.state
            .resolution()
            .runtime
            .providers
            .client_versions
            .auto_update
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
        Arc::new(crate::plugins::storage::PluginFileSecretStore::new())
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

    pub(crate) fn default_selection(&self) -> agena_domain::ExecutionSelection {
        self.state.resolution().default_selection.clone()
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

    /// Model ids whose configured route opted out of following the model
    /// catalog (`manual_overrides`); those routes keep their own values.
    fn manual_override_model_ids(
        provider_config: Option<&ResolvedProviderConfig>,
    ) -> std::collections::BTreeSet<String> {
        let Some(configured) = provider_config else {
            return std::collections::BTreeSet::new();
        };
        configured
            .models
            .iter()
            .filter(|(_, model)| model.definition.manual_overrides)
            .map(|(key, _)| {
                key.split_once('/')
                    .map_or_else(|| key.clone(), |(_, model_id)| model_id.to_owned())
            })
            .collect()
    }

    /// Re-apply each route's own saved definition after catalog decoration.
    ///
    /// Catalog decoration can only choose between catalog values and the
    /// provider's live listing; it never sees the route the user saved. A route
    /// that opted out of following the catalog (`manual_overrides`) must keep
    /// its saved values, otherwise an edit is written to the configuration and
    /// then replaced on the very next listing.
    fn apply_saved_route_definitions(&self, provider_id: &str, models: &mut [agena_domain::Model]) {
        let Some(configured) = self.provider_configs().get(provider_id) else {
            return;
        };
        for model in models {
            let Some(adapter_id) = model.adapter_id.as_ref() else {
                continue;
            };
            let route = format!("{}/{}", adapter_id.as_ref(), model.id.as_ref());
            let Some(saved) = configured.models.get(route.as_str()) else {
                continue;
            };
            if !saved.definition.manual_overrides {
                continue;
            }
            let capabilities = model.capabilities.clone();
            let metadata = model.metadata.clone();
            let mut next = agena_provider::apply_configured_definition_as_baseline(
                &saved.definition,
                &capabilities,
                &metadata,
                model.clone(),
            );
            agena_provider::apply_configured_display_name(&saved.definition, &mut next, false);
            next.native_compaction = saved.native_compaction;
            *model = next;
        }
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
        let mut models = decorate_provider_models(
            &catalog_decoration_source(provider.as_ref()),
            &local_provider_record,
            &Self::manual_override_model_ids(Some(configured)),
            models,
        );
        self.apply_saved_route_definitions(provider_id, &mut models);
        Ok(models)
    }

    pub(crate) async fn discover_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<agena_provider::ProviderModelDiscovery, AppError> {
        let mut discovery = self
            .state
            .services()
            .providers
            .discover_models(provider_id)
            .await?;
        let adapter_ids = self
            .provider_configs()
            .get(provider_id)
            .map(agena_runtime::configured_enabled_adapter_ids)
            .unwrap_or_default();
        if let Some(mut record) = self
            .state
            .services()
            .model_catalog
            .effective_provider_record(&adapter_ids)
        {
            // Catalog data enriches discovered/configured routes; it cannot
            // assert that this account exposes an otherwise unlisted model.
            record.appendable_model_ids.clear();
            let provider = self
                .state
                .services()
                .providers
                .get(provider_id)
                .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
            let source = catalog_decoration_source(provider.as_ref());
            let overrides =
                Self::manual_override_model_ids(self.provider_configs().get(provider_id));
            for entry in &mut discovery.models {
                let mut models = decorate_provider_models(
                    &source,
                    &record,
                    &overrides,
                    vec![entry.model.clone()],
                );
                entry.model = models.remove(0);
            }
        }
        Ok(discovery)
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
        let mut models = decorate_provider_models(
            &catalog_decoration_source(provider.as_ref()),
            &provider_record,
            &Self::manual_override_model_ids(self.provider_configs().get(provider_id)),
            models,
        );
        self.apply_saved_route_definitions(provider_id, &mut models);
        Ok(models)
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

    /// All execution/session owners must retire before final host draining.
    /// Reload itself keeps using the lease-aware generation guards.
    pub(crate) async fn shutdown_plugins(
        &self,
    ) -> Result<(), agena_plugin_host::PluginShutdownReport> {
        self.plugin_shutdown.shutdown_and_wait().await
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

#[cfg(test)]
mod global_plugin_tests {
    use super::*;
    use agena_plugin_host::sdk::host_api::HostClient;

    #[tokio::test]
    async fn global_and_project_plugin_hosts_and_mcp_config_are_isolated() {
        let root =
            std::env::temp_dir().join(format!("agena-plugin-scope-{}", uuid::Uuid::new_v4()));
        let project = root.join("project");
        std::fs::create_dir_all(project.join(".agena")).unwrap();
        let config_path = root.join("agena.json");
        std::fs::write(&config_path, serde_json::to_vec(&serde_json::json!({
            "plugins": {"list": {
                "agena.plan": {"enabled": false, "package": {"kind": "static"}},
                "agena.mcp": {"package": {"kind": "static"}, "settings": {"servers": {
                    "global-marker": {"transport": "stdio", "process": {"command": "global-mcp"}}
                }}}
            }}
        })).unwrap()).unwrap();
        std::fs::write(project.join(".agena/agena.json"), serde_json::to_vec(&serde_json::json!({
            "plugins": {"list": {
                "agena.plan": {"enabled": true, "package": {"kind": "static"}},
                "agena.mcp": {"package": {"kind": "static"}, "settings": {"servers": {
                    "project-marker": {"transport": "stdio", "process": {"command": "project-mcp"}}
                }}}
            }}
        })).unwrap()).unwrap();
        let loader = ConfigLoader::default();
        let request = LoadConfigRequest {
            config_path: Some(config_path),
            workspace_root: Some(project.clone()),
            ..Default::default()
        };
        let mut global = loader.load_global(&request).unwrap();
        let mut merged = loader.load(&request).unwrap();
        for resolution in [&mut global, &mut merged] {
            resolution.config.plugins = agena_runtime::merge_bundled_plugin_config(
                resolution.config.plugins.clone(),
                agena_bundled_plugins::plugins::sources::bundled_plugin_entries(),
            )
            .unwrap();
        }
        let global_mcp =
            agena_runtime_config::mcp_config_from_plugins(&global.config.plugins).unwrap();
        let project_mcp =
            agena_runtime_config::mcp_config_from_plugins(&merged.config.plugins).unwrap();
        assert!(global_mcp.servers.contains_key("global-marker"));
        assert!(!global_mcp.servers.contains_key("project-marker"));
        assert!(project_mcp.servers.contains_key("project-marker"));
        let global_client = super::super::host_client::RuntimeHostClient::candidate(
            1,
            agena_runtime::config_resolution_json_value(&global).unwrap(),
        );
        let project_client = super::super::host_client::RuntimeHostClient::candidate(
            1,
            agena_runtime::config_resolution_json_value(&merged).unwrap(),
        );
        let global_config = global_client.read_config(None).await.unwrap().to_string();
        let project_config = project_client.read_config(None).await.unwrap().to_string();
        assert!(global_config.contains("global-marker"));
        assert!(!global_config.contains("project-marker"));
        assert!(project_config.contains("project-marker"));
        // Keep the host fixture focused on a single bundled marker; unrelated
        // plugins may consult process-wide configuration during initialization.
        global
            .config
            .plugins
            .list
            .retain(|id, _| id == "agena.plan");
        merged
            .config
            .plugins
            .list
            .retain(|id, _| id == "agena.plan");
        let callback = super::super::callback_server::PluginCallbackServer::start()
            .await
            .unwrap();
        let global_host = build_plugin_services(
            agena_runtime::PluginCompositionInputs {
                plugin_config: global.config.plugins,
                workspace_root: &root,
                previous_host: None,
                previous_config: None,
                mcp_manager: None,
                callback_base_url: callback.base_url.clone(),
                callback_dispatcher: callback.dispatcher.clone(),
                host_client: global_client,
            },
            None,
        )
        .await
        .unwrap();
        let project_host = build_plugin_services(
            agena_runtime::PluginCompositionInputs {
                plugin_config: merged.config.plugins,
                workspace_root: &project,
                previous_host: None,
                previous_config: None,
                mcp_manager: None,
                callback_base_url: callback.base_url.clone(),
                callback_dispatcher: callback.dispatcher.clone(),
                host_client: project_client,
            },
            None,
        )
        .await
        .unwrap();
        assert!(!Arc::ptr_eq(&global_host, &project_host));
        assert!(
            !global_host
                .plugins()
                .iter()
                .any(|plugin| plugin.key().to_string() == "agena.plan")
        );
        assert!(
            project_host
                .plugins()
                .iter()
                .any(|plugin| plugin.key().to_string() == "agena.plan")
        );
        global_host.shutdown_report().await;
        project_host.shutdown_report().await;
        let _ = std::fs::remove_dir_all(root);
    }
}
