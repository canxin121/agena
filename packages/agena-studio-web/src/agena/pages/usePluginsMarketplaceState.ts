import type { ComputedRef, Ref } from 'vue'

import type {
  MarketplaceInstalledPluginResource,
  MarketplaceOutdatedPluginResource,
  MarketplacePluginResource,
} from '../lib/agenaApi'

export type PluginsMarketplaceStateInput = {
  filteredMarketplacePlugins: ComputedRef<MarketplacePluginResource[]>
  installMarketplacePluginAction: () => void | Promise<void>
  installedMarketplacePluginIds: ComputedRef<Set<string>>
  marketplaceAllowUnverified: Ref<boolean>
  marketplaceCascadeUninstall: Ref<boolean>
  marketplaceInstallSpec: Ref<string>
  marketplaceInstalled: Ref<MarketplaceInstalledPluginResource[]>
  marketplaceLoading: Ref<boolean>
  marketplaceOutdated: Ref<MarketplaceOutdatedPluginResource[]>
  marketplacePlugins: Ref<MarketplacePluginResource[]>
  marketplaceQuery: Ref<string>
  marketplaceRefreshIndex: Ref<boolean>
  marketplaceRegistryId: Ref<string>
  marketplaceRegistryUrl: Ref<string>
  marketplaceRequireSignature: Ref<boolean>
  searchMarketplaceAction: (options?: { refresh?: boolean }) => void | Promise<void>
  syncMarketplaceRegistryAction: () => void | Promise<void>
  uninstallMarketplacePluginAction: (pluginId: string) => void | Promise<void>
  upgradeMarketplacePluginAction: (pluginId?: string) => void | Promise<void>
}

export function usePluginsMarketplaceState(input: PluginsMarketplaceStateInput) {
  return {
    filteredPlugins: input.filteredMarketplacePlugins,
    installAction: input.installMarketplacePluginAction,
    installedPluginIds: input.installedMarketplacePluginIds,
    allowUnverified: input.marketplaceAllowUnverified,
    cascadeUninstall: input.marketplaceCascadeUninstall,
    installSpec: input.marketplaceInstallSpec,
    installed: input.marketplaceInstalled,
    loading: input.marketplaceLoading,
    outdated: input.marketplaceOutdated,
    plugins: input.marketplacePlugins,
    query: input.marketplaceQuery,
    refreshIndex: input.marketplaceRefreshIndex,
    registryId: input.marketplaceRegistryId,
    registryUrl: input.marketplaceRegistryUrl,
    requireSignature: input.marketplaceRequireSignature,
    searchAction: input.searchMarketplaceAction,
    syncRegistryAction: input.syncMarketplaceRegistryAction,
    uninstallAction: input.uninstallMarketplacePluginAction,
    upgradeAction: input.upgradeMarketplacePluginAction,
  }
}
