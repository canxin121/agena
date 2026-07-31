import { userErrorMessage } from '@/lib/api'
import type { Ref } from 'vue'

import {
  installMarketplacePlugin,
  listMarketplaceInstalledPlugins,
  listMarketplaceOutdatedPlugins,
  searchMarketplacePlugins,
  syncMarketplaceRegistry,
  uninstallMarketplacePlugin,
  upgradeMarketplacePlugins,
  type MarketplaceInstalledPluginResource,
  type MarketplaceOutdatedPluginResource,
  type MarketplacePluginResource,
} from '../lib/agenaApi'

export type RuntimeMarketplaceActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
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
}

export type RuntimeMarketplaceActionsDeps = {
  installMarketplacePlugin: typeof installMarketplacePlugin
  listMarketplaceInstalledPlugins: typeof listMarketplaceInstalledPlugins
  listMarketplaceOutdatedPlugins: typeof listMarketplaceOutdatedPlugins
  load: () => Promise<void>
  searchMarketplacePlugins: typeof searchMarketplacePlugins
  syncMarketplaceRegistry: typeof syncMarketplaceRegistry
  uninstallMarketplacePlugin: typeof uninstallMarketplacePlugin
  upgradeMarketplacePlugins: typeof upgradeMarketplacePlugins
}

export async function loadMarketplacePanelData(
  input: RuntimeMarketplaceActionsInput,
  deps: Omit<RuntimeMarketplaceActionsDeps, 'load'>,
  options?: { refresh?: boolean },
) {
  const registryUrl = input.marketplaceRegistryUrl.value.trim()
  const registryId = input.marketplaceRegistryId.value.trim() || 'default'
  const [installed, outdated, discovered] = await Promise.all([
    deps.listMarketplaceInstalledPlugins(),
    deps.listMarketplaceOutdatedPlugins(),
    registryUrl
      ? deps.searchMarketplacePlugins({
          registryId,
          registryUrl,
          query: input.marketplaceQuery.value,
          refresh: options?.refresh,
        })
      : Promise.resolve([] as MarketplacePluginResource[]),
  ])

  input.marketplaceInstalled.value = installed
  input.marketplaceOutdated.value = outdated
  input.marketplacePlugins.value = discovered
}

const defaultDepsBase = {
  installMarketplacePlugin,
  listMarketplaceInstalledPlugins,
  listMarketplaceOutdatedPlugins,
  searchMarketplacePlugins,
  syncMarketplaceRegistry,
  uninstallMarketplacePlugin,
  upgradeMarketplacePlugins,
}

export function useRuntimeMarketplaceActions(
  input: RuntimeMarketplaceActionsInput,
  deps: RuntimeMarketplaceActionsDeps,
) {
  async function loadMarketplacePanel(options?: { refresh?: boolean }) {
    input.marketplaceLoading.value = true
    input.actionError.value = ''
    try {
      await loadMarketplacePanelData(input, deps, options)
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    } finally {
      input.marketplaceLoading.value = false
    }
  }

  async function searchMarketplaceAction(options?: { refresh?: boolean }) {
    await loadMarketplacePanel(options)
  }

  async function syncMarketplaceRegistryAction() {
    const registryUrl = input.marketplaceRegistryUrl.value.trim()
    if (!registryUrl) return
    input.marketplaceLoading.value = true
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      const result = await deps.syncMarketplaceRegistry({
        registryId: input.marketplaceRegistryId.value.trim() || 'default',
        registryUrl,
      })
      input.actionMessage.value = result.started
        ? 'Started marketplace registry sync.'
        : 'Marketplace registry sync is already running.'
      await deps.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    } finally {
      input.marketplaceLoading.value = false
    }
  }

  async function installMarketplacePluginAction(specOverride?: string) {
    const spec = String(specOverride || input.marketplaceInstallSpec.value).trim()
    const registryUrl = input.marketplaceRegistryUrl.value.trim()
    if (!spec || !registryUrl) return
    input.marketplaceLoading.value = true
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      const result = await deps.installMarketplacePlugin({
        spec,
        registryId: input.marketplaceRegistryId.value.trim() || 'default',
        registryUrl,
        allowUnverified: input.marketplaceAllowUnverified.value,
        requireSignature: input.marketplaceRequireSignature.value,
        refresh: input.marketplaceRefreshIndex.value,
      })
      if (!specOverride) input.marketplaceInstallSpec.value = ''
      input.actionMessage.value = result.started
        ? `Started ${result.task.title.toLowerCase()}.`
        : `${result.task.title} is already running.`
      await deps.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    } finally {
      input.marketplaceLoading.value = false
    }
  }

  async function uninstallMarketplacePluginAction(pluginId: string) {
    if (!pluginId.trim()) return
    input.marketplaceLoading.value = true
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      const result = await deps.uninstallMarketplacePlugin({
        pluginId,
        cascade: input.marketplaceCascadeUninstall.value,
      })
      input.actionMessage.value = result.started
        ? `Started ${result.task.title.toLowerCase()}.`
        : `${result.task.title} is already running.`
      await deps.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    } finally {
      input.marketplaceLoading.value = false
    }
  }

  async function upgradeMarketplacePluginAction(pluginId?: string) {
    input.marketplaceLoading.value = true
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      const result = await deps.upgradeMarketplacePlugins(
        pluginId
          ? { pluginId }
          : {
              all: true,
            },
      )
      input.actionMessage.value = result.started
        ? `Started ${result.task.title.toLowerCase()}.`
        : `${result.task.title} is already running.`
      await deps.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    } finally {
      input.marketplaceLoading.value = false
    }
  }

  return {
    installMarketplacePluginAction,
    loadMarketplacePanel,
    searchMarketplaceAction,
    syncMarketplaceRegistryAction,
    uninstallMarketplacePluginAction,
    upgradeMarketplacePluginAction,
  }
}

export function createRuntimeMarketplaceActions(
  input: RuntimeMarketplaceActionsInput,
  load: () => Promise<void>,
  deps?: Partial<Omit<RuntimeMarketplaceActionsDeps, 'load'>>,
) {
  return useRuntimeMarketplaceActions(input, {
    ...defaultDepsBase,
    ...deps,
    load,
  })
}
