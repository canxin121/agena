import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { usePluginsMarketplaceState } from './usePluginsMarketplaceState'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type PluginsMarketplacePageStateSource = {
  filteredMarketplacePlugins: Parameters<typeof usePluginsMarketplaceState>[0]['filteredMarketplacePlugins']
  installMarketplacePluginAction: Parameters<typeof usePluginsMarketplaceState>[0]['installMarketplacePluginAction']
  installedMarketplacePluginIds: Parameters<typeof usePluginsMarketplaceState>[0]['installedMarketplacePluginIds']
  marketplaceAllowUnverified: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceAllowUnverified']
  marketplaceCascadeUninstall: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceCascadeUninstall']
  marketplaceInstallSpec: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceInstallSpec']
  marketplaceInstalled: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceInstalled']
  marketplaceLoading: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceLoading']
  marketplaceOutdated: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceOutdated']
  marketplacePlugins: Parameters<typeof usePluginsMarketplaceState>[0]['marketplacePlugins']
  marketplaceQuery: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceQuery']
  marketplaceRefreshIndex: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceRefreshIndex']
  marketplaceRegistryId: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceRegistryId']
  marketplaceRegistryUrl: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceRegistryUrl']
  marketplaceRequireSignature: Parameters<typeof usePluginsMarketplaceState>[0]['marketplaceRequireSignature']
  searchMarketplaceAction: Parameters<typeof usePluginsMarketplaceState>[0]['searchMarketplaceAction']
  syncMarketplaceRegistryAction: Parameters<typeof usePluginsMarketplaceState>[0]['syncMarketplaceRegistryAction']
  uninstallMarketplacePluginAction: Parameters<typeof usePluginsMarketplaceState>[0]['uninstallMarketplacePluginAction']
  upgradeMarketplacePluginAction: Parameters<typeof usePluginsMarketplaceState>[0]['upgradeMarketplacePluginAction']
}

export type PluginsMarketplacePageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'plugins'
  }) => {
    shared: RuntimeSectionSharedState
    state: PluginsMarketplacePageStateSource
  }
}

const defaultDeps: PluginsMarketplacePageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & PluginsMarketplacePageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: PluginsMarketplacePageStateSource
    },
}

export function createPluginsMarketplacePanelState(state: PluginsMarketplacePageStateSource) {
  return usePluginsMarketplaceState({
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
}

export function usePluginsMarketplacePageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: PluginsMarketplacePageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'plugins' })
  const marketplace = createPluginsMarketplacePanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    marketplace,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
  }
}
