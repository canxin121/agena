import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { createPluginsInstalledPanelState } from './usePluginsInstalledPageState'
import { createPluginsMarketplacePanelState } from './usePluginsMarketplacePageState'
import { useRuntimeSectionState } from './useRuntimeSectionState'
import { useSectionPanelRegistry } from './useSectionPanelRegistry'
import { createPluginsSectionShellState } from './usePluginsSectionShellState'

export function usePluginsPageState(input: {
  route: RouteLocationNormalizedLoaded
  router: Router
}) {
  const { shared, state } = useRuntimeSectionState({ ...input, section: 'plugins' })

  const installed = createPluginsInstalledPanelState({
    canTogglePluginConfig: state.canTogglePluginConfig,
    loadPluginDetails: state.loadPluginDetails,
    openPluginLogsWorkspacePath: state.openPluginLogsWorkspacePath,
    openPluginManifestInWorkspace: state.openPluginManifestInWorkspace,
    pluginLoading: state.pluginLoading,
    pluginLogs: state.pluginLogs,
    pluginUiPresentation: state.pluginUiPresentation,
    plugins: state.plugins,
    selectedPlugin: state.selectedPlugin,
    setSelectedPluginDisabled: state.setSelectedPluginDisabled,
  })

  const marketplace = createPluginsMarketplacePanelState({
    filteredMarketplacePlugins: state.filteredMarketplacePlugins,
    installMarketplacePluginAction: state.installMarketplacePluginAction,
    installedMarketplacePluginIds: state.installedMarketplacePluginIds,
    marketplaceAllowUnverified: state.marketplaceAllowUnverified,
    marketplaceCascadeUninstall: state.marketplaceCascadeUninstall,
    marketplaceInstallSpec: state.marketplaceInstallSpec,
    marketplaceInstalled: state.marketplaceInstalled,
    marketplaceLoading: state.marketplaceLoading,
    marketplaceOutdated: state.marketplaceOutdated,
    marketplacePlugins: state.marketplacePlugins,
    marketplaceQuery: state.marketplaceQuery,
    marketplaceRefreshIndex: state.marketplaceRefreshIndex,
    marketplaceRegistryId: state.marketplaceRegistryId,
    marketplaceRegistryUrl: state.marketplaceRegistryUrl,
    marketplaceRequireSignature: state.marketplaceRequireSignature,
    searchMarketplaceAction: state.searchMarketplaceAction,
    syncMarketplaceRegistryAction: state.syncMarketplaceRegistryAction,
    uninstallMarketplacePluginAction: state.uninstallMarketplacePluginAction,
    upgradeMarketplacePluginAction: state.upgradeMarketplacePluginAction,
  })

  const shell = createPluginsSectionShellState({
    activePluginsTab: state.activePluginsTab,
    visibleTabs: state.visibleTabs,
  })

  const panelRegistry = useSectionPanelRegistry({
    activeTab: shell.activeTab,
    panels: {
      installed,
      marketplace,
    },
  })

  return {
    activePluginsTab: shell.activeTab,
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    currentPanel: panelRegistry.currentPanel,
    installed,
    load: shared.load,
    loading: shared.loading,
    marketplace,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    panels: panelRegistry.panels,
    tabs: shell.tabs,
  }
}
