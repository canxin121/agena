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
    maintenance: RuntimeMaintenance,
}

#[derive(Clone)]
struct RuntimeServices {
    providers: Arc<ProviderRegistry>,
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
        let mcp_manager = if resolution.config.mcp.servers.is_empty() {
            None
        } else {
            Some(build_mcp_manager(&resolution.config.mcp).await)
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
        let provider_client =
            ProviderRegistry::build_http_client(resolution.config.provider_http_client_config())?;
        let mut model_catalog_config = ModelCatalogConfig::for_workspace_root(workspace_root);
        model_catalog_config.remote_url =
            resolution.config.runtime.model_catalog.remote_url.clone();
        model_catalog_config.fallback_url =
            resolution.config.runtime.model_catalog.fallback_url.clone();
        model_catalog_config.cache_max_age_secs =
            resolution.config.runtime.model_catalog.cache_max_age_secs;
        let model_catalog = Arc::new(ModelCatalogService::new(
            provider_client,
            ModelCatalogStore::new(model_catalog_config),
        )?);
        let mut catalog_snapshot = model_catalog.snapshot();
        if let Ok(snapshot) = model_catalog.refresh_if_stale_on_startup().await {
            catalog_snapshot = snapshot;
        }
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
        let lsp_registry = if resolution.config.lsp.servers.is_empty() {
            None
        } else {
            Some(build_lsp_registry(workspace_root, &resolution.config.lsp))
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
            model_catalog,
            plugins,
            agents,
            session_manager,
            mcp_manager,
            lsp_registry,
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
            .effective_provider_record(provider_id, &adapter_ids)
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
    agents: crate::agents::SubagentRegistry,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
) -> Arc<SessionManager> {
    let processor = build_session_processor(providers, Arc::clone(&plugins));
    let config = session_manager_config(resolution);

    if let Some(manager) = existing {
        let executor = build_tool_executor(
            plugins,
            agents.clone(),
            lsp_registry,
            workspace_root,
            resolution,
            Some(Arc::clone(&manager)),
        );
        manager.reconfigure(processor, executor, config);
        return manager;
    }

    let bootstrap_executor = build_tool_executor(
        Arc::clone(&plugins),
        agents.clone(),
        lsp_registry.clone(),
        workspace_root,
        resolution,
        None,
    );
    let manager = Arc::new(
        SessionManager::new(db.as_ref().clone(), processor.clone(), bootstrap_executor)
            .with_config(config.clone()),
    );
    let executor = build_tool_executor(
        plugins,
        agents,
        lsp_registry,
        workspace_root,
        resolution,
        Some(Arc::clone(&manager)),
    );
    manager.reconfigure(processor, executor, config);
    manager
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
    agents: crate::agents::SubagentRegistry,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
    session_manager: Option<Arc<SessionManager>>,
) -> ToolExecutor {
    let agent = build_profile_agent(
        "build",
        crate::agent::PermissionConfig::default(),
        resolution,
    );
    let worktree_registry = crate::tool::worktree_registry_for_executor();

    // Drop any orphan worktrees left over from a previously-crashed
    // session so a clean startup does not accumulate
    // .agena/worktrees/<slug> directories indefinitely. Stale = no live
    // session and not registered with `git worktree list`.
    let pruned = crate::tool::worktree_prune_stale(workspace_root, &worktree_registry);
    if !pruned.is_empty() {
        tracing::info!(
            target: "agena::runtime::worktree",
            removed = pruned.len(),
            "pruned stale worktree directories at startup"
        );
    }

    let mut executor = ToolExecutor::new(workspace_root.to_path_buf(), agent)
        .with_plugin_manager(plugins)
        .with_subagent_registry(agents)
        .with_web_search_backend(resolution.config.web.search.resolve())
        .with_plan_registry(crate::tool::plan_registry_for_executor())
        .with_worktree_registry(worktree_registry);

    if let Some(manager) = session_manager {
        executor = executor.with_scheduler(build_scheduler(manager));
    }

    if let Some(registry) = lsp_registry {
        executor = executor.with_lsp_registry(registry);
    }

    executor
}

fn build_lsp_registry(
    workspace_root: &Path,
    config: &crate::config::LspConfig,
) -> Arc<agena_lsp::LspRegistry> {
    use agena_lsp::{LspRegistry, LspServerSpec};

    let registry = Arc::new(LspRegistry::new(
        workspace_root.to_path_buf(),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    ));

    let registry_for_register = registry.clone();
    let entries: Vec<(String, crate::config::LspServerConfig)> = config
        .servers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    tokio::spawn(async move {
        for (name, entry) in entries {
            let env = entry
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let spec = LspServerSpec {
                name: name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                env,
                file_extensions: entry.file_extensions.clone(),
                root_markers: entry.root_markers.clone(),
                initialization_options: entry.initialization_options.clone(),
            };
            registry_for_register.register(spec).await;
            tracing::info!(
                target: "agena::lsp",
                "registered LSP server '{name}' (lazy-spawn)"
            );
        }
    });

    registry
}

async fn build_mcp_manager(
    config: &crate::config::McpConfig,
) -> Arc<agena_mcp_client::McpConnectionManager> {
    use agena_mcp_client::{
        FileTokenStore, HttpTransportMode, McpConnectionManager, ServerSpec, TokenStore,
    };

    let mut manager = McpConnectionManager::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    // Best-effort: open the on-disk token store so HttpAuth::BearerFromStore
    // can resolve. A missing file is fine; a corrupt one is logged and the
    // store is left unset (runtime continues, just without token lookup).
    match FileTokenStore::open_default() {
        Ok(store) => {
            manager.set_token_store(Arc::new(store) as Arc<dyn TokenStore>);
        }
        Err(err) => {
            tracing::warn!(
                target: "agena::mcp",
                "failed to open default token store: {err}"
            );
        }
    }

    let manager = Arc::new(manager);
    // Failures only disable that one server — the rest of the runtime keeps booting.
    for (name, entry) in &config.servers {
        let manager = manager.clone();
        let name = name.clone();
        let spec = match entry {
            crate::config::McpServerConfig::Stdio {
                command,
                args,
                env,
                cwd,
            } => ServerSpec::Stdio {
                command: command.clone(),
                args: args.clone(),
                env: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                cwd: cwd.clone(),
            },
            crate::config::McpServerConfig::Http {
                url,
                mode,
                headers,
                auth,
            } => {
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
                    crate::config::McpHttpMode::StreamableHttp => HttpTransportMode::StreamableHttp,
                };
                let auth = auth.as_ref().map(|cfg| match cfg {
                    crate::config::McpHttpAuthConfig::Bearer { token } => {
                        agena_mcp_client::HttpAuth::Bearer(token.clone())
                    }
                    crate::config::McpHttpAuthConfig::BearerFromEnv { env } => {
                        agena_mcp_client::HttpAuth::BearerFromEnv(env.clone())
                    }
                    crate::config::McpHttpAuthConfig::BearerFromStore => {
                        agena_mcp_client::HttpAuth::BearerFromStore
                    }
                    crate::config::McpHttpAuthConfig::Custom { headers } => {
                        agena_mcp_client::HttpAuth::Custom(
                            headers
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),
                        )
                    }
                });
                ServerSpec::Http {
                    url: parsed,
                    mode,
                    headers: headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    auth,
                }
            }
        };
        if let Err(e) = manager.add_server(&name, spec).await {
            tracing::warn!(
                target: "agena::mcp",
                "failed to connect MCP server '{name}': {e}"
            );
        } else {
            tracing::info!(target: "agena::mcp", "connected MCP server '{name}'");
        }
    }
    manager
}

fn session_manager_config(resolution: &ConfigResolution) -> SessionManagerConfig {
    let defaults = SessionManagerConfig::default();
    SessionManagerConfig {
        cache_max_sessions: resolution.config.runtime.session_cache.max_sessions,
        cache_ttl: Duration::from_secs(resolution.config.runtime.session_cache.ttl_secs),
        cache_max_bytes: resolution.config.runtime.session_cache.max_bytes,
        max_turn_loops: defaults.max_turn_loops,
        doom_loop: defaults.doom_loop,
        default_agent: resolution.config.runtime.default_agent.clone(),
        permission: resolution.config.permission.clone(),
    }
}

fn register_config_agents(
    registry: &crate::agents::SubagentRegistry,
    _resolution: &ConfigResolution,
    agents: &std::collections::BTreeMap<String, crate::config::AgentConfig>,
) {
    for (name, config) in agents {
        if config.disabled || name.trim().is_empty() {
            continue;
        }
        registry.register_runtime(crate::agents::AgentProfile {
            name: name.trim().to_string(),
            frontmatter: crate::agents::AgentFrontmatter {
                description: config.description.clone(),
                mode: config.mode,
                hidden: config.hidden,
                color: config.color.clone(),
                temperature: config.temperature,
                max_output_tokens: config.max_output_tokens,
                steps: config.steps,
                allowed_tools: config.allowed_tools.clone(),
                permission: config.permission.clone(),
                model: config.model.clone(),
                aliases: config.aliases.clone(),
            },
            prompt: config.prompt.trim().to_string(),
            source_path: None,
            scope: crate::agents::AgentScope::Project,
        });
    }
}

fn build_profile_agent(
    name: impl Into<String>,
    permission: crate::agent::PermissionConfig,
    _resolution: &ConfigResolution,
) -> Agent {
    let agent = Agent::new(name, crate::permission::PermissionPolicy::allow_all());
    match agent.try_with_permission_config(&permission) {
        Ok(agent) => agent,
        Err(err) => {
            tracing::warn!(
                target: "agena::config::permission",
                "ignoring invalid agent permission config: {err}; falling back to allow_all"
            );
            Agent::new("build", crate::permission::PermissionPolicy::allow_all())
        }
    }
}

fn collect_watch_paths(resolution: &ConfigResolution) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_watch_path(&mut paths, resolution.meta.config_path.clone());

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

/// Build a process-wide cron scheduler backed by the active SessionManager.
fn build_scheduler(session_manager: Arc<SessionManager>) -> Arc<agena_scheduler::Scheduler> {
    use std::time::Duration;

    struct SessionSink {
        session_manager: Arc<SessionManager>,
    }

    impl SessionSink {
        fn notify_job_result(
            &self,
            job: &agena_scheduler::ScheduledJob,
            result: &agena_scheduler::JobDeliveryResult,
        ) {
            let status = match result.status {
                agena_scheduler::JobRunStatus::Submitted => "submitted",
                agena_scheduler::JobRunStatus::Skipped => "skipped",
                agena_scheduler::JobRunStatus::Failed => "failed",
            };
            let title = format!("Scheduled job {status}");
            let message = result
                .error_message
                .clone()
                .unwrap_or_else(|| job.prompt.clone());
            let payload = serde_json::json!({
                "job_id": job.id,
                "prompt": job.prompt,
                "owner_session_id": job.owner_session_id,
                "status": status,
                "error_message": result.error_message,
                "next_fire_at": job.next_fire_at,
                "last_fired_at": job.last_fired_at,
            });
            self.session_manager.tool_executor().broadcast_notification(
                "scheduled_job",
                result.session_id.or(job.owner_session_id),
                title,
                message,
                payload,
            );
        }
    }

    #[async_trait::async_trait]
    impl agena_scheduler::JobSink for SessionSink {
        async fn deliver(
            &self,
            job: &agena_scheduler::ScheduledJob,
        ) -> agena_scheduler::JobDeliveryResult {
            let result = if let Some(session_id) = job.owner_session_id {
                if self.session_manager.is_turn_active(session_id).await {
                    agena_scheduler::JobDeliveryResult::skipped(
                        Some(session_id),
                        "session already has an active turn",
                    )
                } else {
                    let session = match self.session_manager.get_session(session_id).await {
                        Ok(session) => session,
                        Err(err) => {
                            let result = agena_scheduler::JobDeliveryResult::failed(
                                Some(session_id),
                                err.to_string(),
                            );
                            self.notify_job_result(job, &result);
                            return result;
                        }
                    };

                    if session.blocked() {
                        agena_scheduler::JobDeliveryResult::skipped(
                            Some(session_id),
                            "session is blocked on permission or user input",
                        )
                    } else {
                        let options = match self
                            .session_manager
                            .resolve_scheduled_run_options(session_id)
                            .await
                        {
                            Ok(options) => options,
                            Err(err) => {
                                let result = agena_scheduler::JobDeliveryResult::failed(
                                    Some(session_id),
                                    err.to_string(),
                                );
                                self.notify_job_result(job, &result);
                                return result;
                            }
                        };

                        match self
                            .session_manager
                            .submit_user_turn(crate::session::SessionUserTurnRequest {
                                session_id,
                                options,
                                parts: vec![crate::message::PartContent::text(job.prompt.clone())],
                            })
                            .await
                        {
                            Ok(_) => {
                                agena_scheduler::JobDeliveryResult::submitted(Some(session_id))
                            }
                            Err(err) => agena_scheduler::JobDeliveryResult::failed(
                                Some(session_id),
                                err.to_string(),
                            ),
                        }
                    }
                }
            } else {
                agena_scheduler::JobDeliveryResult::failed(
                    None,
                    "scheduled job has no owner_session_id",
                )
            };
            self.notify_job_result(job, &result);
            result
        }
    }

    let sched = agena_scheduler::scheduler::build_in_memory(
        Arc::new(SessionSink { session_manager }),
        Duration::from_secs(10),
    );
    sched.start();
    sched
}

#[cfg(test)]
mod tests {
    use super::build_profile_agent;
    use crate::config::{ConfigLoader, LoadConfigRequest, ProcessEnvironment};
    use crate::permission::{AccessKind, PermissionDecision};
    use std::path::Path;

    #[test]
    fn build_profile_agent_uses_only_profile_permission() {
        let config_path = std::env::temp_dir().join(format!(
            "agena-runtime-snapshot-empty-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&config_path, "").expect("test config should be written");

        let resolution = ConfigLoader::<ProcessEnvironment>::new(ProcessEnvironment)
            .load(&LoadConfigRequest {
                config_path: Some(config_path.clone()),
                ..LoadConfigRequest::default()
            })
            .expect("default config should load");

        let agent = build_profile_agent(
            "build",
            crate::agent::PermissionConfig::default(),
            &resolution,
        );

        assert_eq!(
            agent.authorize_path_access(
                AccessKind::Write,
                Path::new("/workspace"),
                Path::new("/workspace/file.txt"),
            ),
            PermissionDecision::Allow
        );
    }
}
