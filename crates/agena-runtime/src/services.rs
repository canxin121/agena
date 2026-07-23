use std::sync::Arc;

/// Generic service bundle owned by the runtime composition layer.
///
/// Concrete service implementations stay outside this crate; runtime owns
/// their snapshot-scoped handles and lifecycle guards.
#[derive(Clone)]
pub(crate) struct RuntimeServiceBundle<
    Providers,
    CatalogSource,
    ModelCatalog,
    Plugins,
    Agents,
    Sessions,
    Mcp,
    Lsp,
> {
    pub(crate) providers: Providers,
    pub(crate) catalog_source_providers: CatalogSource,
    pub(crate) model_catalog: ModelCatalog,
    pub(crate) plugins: Plugins,
    pub(crate) agents: Agents,
    pub(crate) session_manager: Sessions,
    pub(crate) mcp_manager: Mcp,
    pub(crate) lsp_registry: Lsp,
    pub(crate) _lsp_registration: Option<Arc<crate::AbortOnDrop>>,
    pub(crate) _event_bridge: Option<Arc<crate::AbortOnDrop>>,
    pub(crate) _plugin_shutdown: Option<Arc<crate::CallbackOnDrop>>,
}

impl<Providers, CatalogSource, ModelCatalog, Plugins, Agents, Sessions, Mcp, Lsp>
    RuntimeServiceBundle<
        Providers,
        CatalogSource,
        ModelCatalog,
        Plugins,
        Agents,
        Sessions,
        Mcp,
        Lsp,
    >
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        providers: Providers,
        catalog_source_providers: CatalogSource,
        model_catalog: ModelCatalog,
        plugins: Plugins,
        agents: Agents,
        session_manager: Sessions,
        mcp_manager: Mcp,
        lsp_registry: Lsp,
        lsp_registration: Option<Arc<crate::AbortOnDrop>>,
        event_bridge: Option<Arc<crate::AbortOnDrop>>,
        plugin_shutdown: Option<Arc<crate::CallbackOnDrop>>,
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
            _lsp_registration: lsp_registration,
            _event_bridge: event_bridge,
            _plugin_shutdown: plugin_shutdown,
        }
    }
}
