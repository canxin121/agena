use std::{path::PathBuf, sync::Arc};

use agena_provider::ProviderCatalog;
use agena_storage::{MemoryRepository, PermissionRuleRepository, WorkspaceRepository};

use crate::{
    ModelCatalogRuntimeService, PluginRuntimeService, RuntimeActivityService,
    RuntimeAuthenticationService, RuntimeConfigSettingsService, RuntimeConfigurationService,
    RuntimeControlService, RuntimeDraftAuthenticationService, RuntimeLiveSignalService,
    RuntimeStatusService, RuntimeToolExecutionService, SessionExecutionCommandService,
    SessionExecutionControl, SessionPluginCommandService, SessionQueryService,
    SessionToolExecutionService,
};

/// Runtime-owned result of assembling the capabilities required by the
/// application layer.
///
/// Full transcript/session orchestration remains a concrete adapter concern,
/// but all application-facing session capabilities are exposed as stable
/// ports. Upper layers therefore do not need to unpack a runtime snapshot or
/// name a concrete session manager merely to construct their application
/// handle.
#[derive(Clone)]
pub struct RuntimeApplicationServices {
    pub workspace_root: PathBuf,
    /// Contract-backed repositories assembled with the concrete Runtime.
    /// Upper layers receive these ports without constructing SQLite adapters.
    pub repositories: Option<RuntimeApplicationRepositories>,
    pub provider_catalog: Arc<dyn ProviderCatalog>,
    pub model_catalog: Arc<dyn ModelCatalogRuntimeService>,
    pub plugins: Arc<dyn PluginRuntimeService>,
    pub configuration: Arc<dyn RuntimeConfigurationService>,
    pub config_settings: Arc<dyn RuntimeConfigSettingsService>,
    pub control: Arc<dyn RuntimeControlService>,
    pub authentication: Arc<dyn RuntimeAuthenticationService>,
    pub draft_authentication: Arc<dyn RuntimeDraftAuthenticationService>,
    pub status: Arc<dyn RuntimeStatusService>,
    pub tools: Arc<dyn RuntimeToolExecutionService>,
    pub activities: Option<Arc<dyn RuntimeActivityService>>,
    /// Ephemeral runtime live signals (background activities, plugin events,
    /// tool-registry changes). Session data live updates are `SessionChange`
    /// on the sealed facade, not this stream (14.3).
    pub live_signals: Option<Arc<dyn RuntimeLiveSignalService>>,
    /// Sealed session store facade (14.1): the only path application surfaces
    /// use to read/write session data. Session data live updates are
    /// `SessionChange` on this facade's notification bus, never an event log.
    pub session_store: Option<Arc<dyn agena_storage::store::SessionStore>>,
    pub session_queries: Option<Arc<dyn SessionQueryService>>,
    pub execution_control: Option<Arc<dyn SessionExecutionControl>>,
    pub execution_commands: Option<Arc<dyn SessionExecutionCommandService>>,
    pub tool_execution: Option<Arc<dyn SessionToolExecutionService>>,
    pub plugin_commands: Option<Arc<dyn SessionPluginCommandService>>,
}

/// Storage contracts that accompany a composed Runtime application service
/// bundle. This carries only storage traits; SeaORM and SQLite adapter types
/// remain inside Runtime composition.
#[derive(Clone)]
pub struct RuntimeApplicationRepositories {
    pub memory: Arc<dyn MemoryRepository>,
    pub workspace: Arc<dyn WorkspaceRepository>,
    pub permission_rules: Arc<dyn PermissionRuleRepository>,
}

/// Typed composition input for the application-facing Runtime port bundle.
/// Concrete composition adapters provide the port implementations; Runtime
/// owns the stable assembly consumed by entrypoints.
pub(crate) struct RuntimeApplicationServiceCompositionInputs {
    pub(crate) workspace_root: PathBuf,
    pub(crate) repositories: Option<RuntimeApplicationRepositories>,
    pub(crate) provider_catalog: Arc<dyn ProviderCatalog>,
    pub(crate) model_catalog: Arc<dyn ModelCatalogRuntimeService>,
    pub(crate) plugins: Arc<dyn PluginRuntimeService>,
    pub(crate) configuration: Arc<dyn RuntimeConfigurationService>,
    pub(crate) config_settings: Arc<dyn RuntimeConfigSettingsService>,
    pub(crate) control: Arc<dyn RuntimeControlService>,
    pub(crate) authentication: Arc<dyn RuntimeAuthenticationService>,
    pub(crate) draft_authentication: Arc<dyn RuntimeDraftAuthenticationService>,
    pub(crate) status: Arc<dyn RuntimeStatusService>,
    pub(crate) tools: Arc<dyn RuntimeToolExecutionService>,
    pub(crate) activities: Option<Arc<dyn RuntimeActivityService>>,
    pub(crate) live_signals: Option<Arc<dyn RuntimeLiveSignalService>>,
    pub(crate) session_store: Option<Arc<dyn agena_storage::store::SessionStore>>,
    pub(crate) session_queries: Option<Arc<dyn SessionQueryService>>,
    pub(crate) execution_control: Option<Arc<dyn SessionExecutionControl>>,
    pub(crate) execution_commands: Option<Arc<dyn SessionExecutionCommandService>>,
    pub(crate) tool_execution: Option<Arc<dyn SessionToolExecutionService>>,
    pub(crate) plugin_commands: Option<Arc<dyn SessionPluginCommandService>>,
}

pub(crate) fn compose_runtime_application_services(
    inputs: RuntimeApplicationServiceCompositionInputs,
) -> RuntimeApplicationServices {
    RuntimeApplicationServices {
        workspace_root: inputs.workspace_root,
        repositories: inputs.repositories,
        provider_catalog: inputs.provider_catalog,
        model_catalog: inputs.model_catalog,
        plugins: inputs.plugins,
        configuration: inputs.configuration,
        config_settings: inputs.config_settings,
        control: inputs.control,
        authentication: inputs.authentication,
        draft_authentication: inputs.draft_authentication,
        status: inputs.status,
        tools: inputs.tools,
        activities: inputs.activities,
        live_signals: inputs.live_signals,
        session_store: inputs.session_store,
        session_queries: inputs.session_queries,
        execution_control: inputs.execution_control,
        execution_commands: inputs.execution_commands,
        tool_execution: inputs.tool_execution,
        plugin_commands: inputs.plugin_commands,
    }
}
