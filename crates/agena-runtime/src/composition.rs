use std::sync::Arc;

use sea_orm::DatabaseConnection;

/// Typed inputs crossing from runtime snapshot resolution into model-catalog
/// composition. The runtime crate owns the shape; concrete provider, plugin,
/// and database implementations remain supplied by the composition owner.
pub(crate) struct ModelCatalogCompositionInputs<Providers, ConfigPath, Plugins, Database> {
    pub(crate) providers: Providers,
    pub(crate) config_path: ConfigPath,
    pub(crate) plugins: Plugins,
    pub(crate) database: Database,
}

/// Typed inputs for plugin-host composition. The runtime owns the boundary;
/// concrete host/configuration types remain supplied by the caller.
pub(crate) struct PluginCompositionInputs<
    PluginConfig,
    Workspace,
    PreviousHost,
    PreviousConfig,
    Mcp,
> {
    pub(crate) plugin_config: PluginConfig,
    pub(crate) workspace_root: Workspace,
    pub(crate) previous_host: PreviousHost,
    pub(crate) previous_config: PreviousConfig,
    pub(crate) mcp_manager: Mcp,
}

/// Typed inputs for session/service composition. Concrete session, provider,
/// permission, and tool implementations remain outside this contract crate.
pub(crate) struct SessionCompositionInputs<
    Existing,
    Database,
    Providers,
    Plugins,
    Lsp,
    Workspace,
    Config,
> {
    pub(crate) existing: Existing,
    pub(crate) database: Database,
    pub(crate) providers: Providers,
    pub(crate) plugins: Plugins,
    pub(crate) lsp_registry: Lsp,
    pub(crate) workspace_root: Workspace,
    pub(crate) config: Config,
    pub(crate) mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    /// Background-process registry installed into every tool executor built
    /// for this session manager.
    pub(crate) monitor_registry: Option<Arc<dyn crate::MonitorService>>,
    /// Dedicated scheduler database connection; `None` degrades the scheduler
    /// to its in-memory store.
    pub(crate) scheduler_database: Option<Arc<DatabaseConnection>>,
}

/// Default upper bound for concurrently executing tools in one session.
pub(crate) const DEFAULT_MAX_CONCURRENT_TOOLS: usize = 32;

/// Resolved session behavior passed into a concrete session-manager adapter.
///
/// This is configuration data only. Runtime composes the concrete session
/// manager, provider registry, and tool executor from it.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeSessionBuildConfig {
    pub(crate) default_selection: agena_domain::ExecutionSelection,
    pub(crate) permission: agena_domain::PermissionConfig,
    pub(crate) auto_compaction: agena_domain::SessionAutoCompactionConfig,
    pub(crate) cache_limits: agena_domain::SessionCacheLimits,
    pub(crate) max_concurrent_tools: usize,
    /// Cap on model turns within one stable run; `None` uses the session
    /// manager's fallback (`DEFAULT_MAX_MODEL_TURNS`). Not yet wired to TOML;
    /// runtime keeps the default so behavior matches the session default.
    pub(crate) max_turns: Option<usize>,
}

/// Project the resolved configuration values needed by session composition.
/// Runtime owns this value-only mapping so snapshot code does not reconstruct
/// session policy.
pub(crate) fn session_build_config_from_resolved(
    config: &crate::ResolvedConfig,
) -> RuntimeSessionBuildConfig {
    RuntimeSessionBuildConfig {
        default_selection: config.default_selection.clone(),
        permission: config.permission.clone(),
        auto_compaction: agena_domain::SessionAutoCompactionConfig {
            enabled: config.session.compaction.auto,
            reserved_tokens: config.session.compaction.reserved_tokens,
        },
        cache_limits: agena_domain::SessionCacheLimits::default(),
        max_concurrent_tools: DEFAULT_MAX_CONCURRENT_TOOLS,
        // `None` falls back to `DEFAULT_MAX_MODEL_TURNS` (500) in
        // agena-runtime-session; `Some(0)` means unlimited (handled in
        // `replies_execution.rs`).
        max_turns: config.session.max_turns,
    }
}

/// Typed inputs for tool-executor construction.
pub(crate) struct ToolCompositionInputs<Plugins, Lsp, Workspace, Session> {
    pub(crate) plugins: Plugins,
    pub(crate) lsp_registry: Lsp,
    pub(crate) workspace_root: Workspace,
    pub(crate) session_manager: Session,
    /// Background-process registry installed into the tool executor. When
    /// `None` the executor lazily builds its default registry.
    pub(crate) monitor_registry: Option<Arc<dyn crate::MonitorService>>,
    /// Dedicated scheduler database connection, threaded to the session
    /// scheduler. `None` uses the scheduler's in-memory store.
    pub(crate) scheduler_database: Option<Arc<DatabaseConnection>>,
}

/// Typed inputs for database connection/schema composition. Runtime owns the
/// shape and reuse/initialization choreography; concrete URL resolution,
/// tracing, and schema code remain with the composition owner.
pub(crate) struct DatabaseCompositionInputs<Connection, DatabaseUrl, Tracing> {
    pub(crate) database_connection: Connection,
    pub(crate) database_url: DatabaseUrl,
    pub(crate) database_path: Option<std::path::PathBuf>,
    pub(crate) initialize_schema: bool,
    pub(crate) tracing: Tracing,
}

/// Fully assembled inputs for one immutable runtime snapshot. The Runtime owns
/// snapshot metadata/state assembly while a concrete composition adapter
/// supplies its resolved configuration and service implementations.
pub(crate) struct RuntimeSnapshotCompositionInputs<Resolution, Services> {
    pub(crate) generation: u64,
    pub(crate) resolution: Resolution,
    pub(crate) services: Services,
    pub(crate) tasks: crate::RuntimeTaskState,
}

pub(crate) fn compose_runtime_snapshot_state<Resolution, Services>(
    inputs: RuntimeSnapshotCompositionInputs<Resolution, Services>,
) -> crate::RuntimeSnapshotState<Resolution, Services> {
    crate::RuntimeSnapshotState::new(
        inputs.generation,
        inputs.resolution,
        inputs.services,
        inputs.tasks,
    )
}

/// Start the long-lived maintenance loops owned by one composed runtime.
/// Concrete adapters supply the loop futures; Runtime owns their task-control
/// registration and shared shutdown lifecycle.
pub(crate) fn spawn_runtime_maintenance_loops<Janitor, Reload>(
    task_control: &crate::TaskControl,
    janitor: Janitor,
    reload: Reload,
) where
    Janitor: std::future::Future<Output = ()> + Send + 'static,
    Reload: std::future::Future<Output = ()> + Send + 'static,
{
    task_control.spawn(janitor);
    task_control.spawn(reload);
}

/// Runtime policy for model-catalog cache freshness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelCatalogRuntimeConfig {
    pub(crate) cache_max_age_secs: u64,
}

impl Default for ModelCatalogRuntimeConfig {
    fn default() -> Self {
        Self {
            cache_max_age_secs: 60 * 60 * 24 * 7,
        }
    }
}
