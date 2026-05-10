import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type {
  MarketplaceInstalledPluginResource,
  MarketplaceOutdatedPluginResource,
  MarketplacePluginResource,
} from '../lib/agenaApi'
import { createRuntimeMarketplaceActions } from './useRuntimeMarketplaceActions'

function createState() {
  const state = {
    actionError: ref(''),
    actionMessage: ref(''),
    marketplaceAllowUnverified: ref(true),
    marketplaceCascadeUninstall: ref(true),
    marketplaceInstallSpec: ref('demo/plugin'),
    marketplaceInstalled: ref<MarketplaceInstalledPluginResource[]>([]),
    marketplaceLoading: ref(false),
    marketplaceOutdated: ref<MarketplaceOutdatedPluginResource[]>([]),
    marketplacePlugins: ref<MarketplacePluginResource[]>([]),
    marketplaceQuery: ref('demo'),
    marketplaceRefreshIndex: ref(true),
    marketplaceRegistryId: ref('custom'),
    marketplaceRegistryUrl: ref('https://registry.example.test'),
    marketplaceRequireSignature: ref(false),
  }
  const calls: string[] = []
  const actions = createRuntimeMarketplaceActions(state, async () => {
    calls.push('load')
  }, {
    listMarketplaceInstalledPlugins: async () => {
      calls.push('listMarketplaceInstalledPlugins')
      return [{
        plugin_id: 'demo/plugin',
        version: '1.0.0',
        kind: 'wasm',
        platform: 'any',
        binary_path: '/plugins/demo/plugin.wasm',
        config_path: '/plugins/demo/plugin.json',
        installed_at: '2026-05-10T00:00:00Z',
        registry_id: 'custom',
        registry_url: 'https://registry.example.test',
        archive_extracted: false,
      }]
    },
    listMarketplaceOutdatedPlugins: async () => {
      calls.push('listMarketplaceOutdatedPlugins')
      return [{ plugin_id: 'demo/plugin', installed_version: '1.0.0', latest_version: '1.1.0' }]
    },
    searchMarketplacePlugins: async ({ registryId, registryUrl, query, refresh }) => {
      calls.push(`searchMarketplacePlugins:${registryId}:${registryUrl}:${query}:${refresh ? 'refresh' : 'cached'}`)
      return [{
        plugin_id: 'demo/plugin',
        name: 'Demo Plugin',
        latest_version: '1.1.0',
        description: 'Demo',
        version_count: 2,
      }]
    },
    syncMarketplaceRegistry: async ({ registryId, registryUrl }) => {
      calls.push(`syncMarketplaceRegistry:${registryId}:${registryUrl}`)
      return { registry_id: registryId || 'default', registry_url: registryUrl, plugin_count: 3 }
    },
    installMarketplacePlugin: async ({ spec, registryId, registryUrl, allowUnverified, requireSignature, refresh }) => {
      calls.push(`installMarketplacePlugin:${spec}:${registryId}:${registryUrl}:${allowUnverified}:${requireSignature}:${refresh}`)
      return {
        plugin_id: spec,
        version: '1.1.0',
        kind: 'wasm',
        artifact_path: '/downloads/demo-plugin-1.1.0.wasm',
        config_path: '/plugins/demo/plugin.json',
        dry_run: false,
      }
    },
    uninstallMarketplacePlugin: async ({ pluginId, cascade }) => {
      calls.push(`uninstallMarketplacePlugin:${pluginId}:${cascade}`)
      return [{ plugin_id: pluginId, version: '1.1.0', config_path: '/plugins/demo/plugin.json' }]
    },
    upgradeMarketplacePlugins: async (options) => {
      calls.push(`upgradeMarketplacePlugins:${'pluginId' in options ? options.pluginId : 'all'}`)
      return [{
        plugin_id: 'demo/plugin',
        previous_version: '1.0.0',
        installed_version: '1.1.0',
        upgraded: true,
        outcome: {
          plugin_id: 'demo/plugin',
          version: '1.1.0',
          kind: 'wasm',
          artifact_path: '/downloads/demo-plugin-1.1.0.wasm',
          config_path: '/plugins/demo/plugin.json',
          dry_run: false,
        },
      }]
    },
  })

  return { actions, calls, state }
}

describe('useRuntimeMarketplaceActions', () => {
  test('loadMarketplacePanel hydrates marketplace state', async () => {
    const { actions, calls, state } = createState()

    await actions.loadMarketplacePanel()

    expect(calls).toEqual([
      'listMarketplaceInstalledPlugins',
      'listMarketplaceOutdatedPlugins',
      'searchMarketplacePlugins:custom:https://registry.example.test:demo:cached',
    ])
    expect(state.marketplaceInstalled.value.map((item) => item.plugin_id)).toEqual(['demo/plugin'])
    expect(state.marketplaceOutdated.value.map((item) => item.plugin_id)).toEqual(['demo/plugin'])
    expect(state.marketplacePlugins.value.map((item) => item.plugin_id)).toEqual(['demo/plugin'])
    expect(state.marketplaceLoading.value).toBe(false)
  })

  test('sync/install/uninstall/upgrade actions refresh marketplace and load plugin state', async () => {
    const { actions, calls, state } = createState()

    await actions.syncMarketplaceRegistryAction()
    await actions.installMarketplacePluginAction()
    await actions.uninstallMarketplacePluginAction('demo/plugin')
    await actions.upgradeMarketplacePluginAction('demo/plugin')

    expect(calls).toEqual([
      'syncMarketplaceRegistry:custom:https://registry.example.test',
      'listMarketplaceInstalledPlugins',
      'listMarketplaceOutdatedPlugins',
      'searchMarketplacePlugins:custom:https://registry.example.test:demo:refresh',
      'installMarketplacePlugin:demo/plugin:custom:https://registry.example.test:true:false:true',
      'load',
      'listMarketplaceInstalledPlugins',
      'listMarketplaceOutdatedPlugins',
      'searchMarketplacePlugins:custom:https://registry.example.test:demo:cached',
      'uninstallMarketplacePlugin:demo/plugin:true',
      'load',
      'listMarketplaceInstalledPlugins',
      'listMarketplaceOutdatedPlugins',
      'searchMarketplacePlugins:custom:https://registry.example.test:demo:cached',
      'upgradeMarketplacePlugins:demo/plugin',
      'load',
      'listMarketplaceInstalledPlugins',
      'listMarketplaceOutdatedPlugins',
      'searchMarketplacePlugins:custom:https://registry.example.test:demo:refresh',
    ])
    expect(state.marketplaceInstallSpec.value).toBe('')
    expect(state.actionMessage.value).toBe('Upgraded demo/plugin.')
    expect(state.marketplaceLoading.value).toBe(false)
  })

  test('upgrade all reports when everything is already current', async () => {
    const { state } = createState()
    const calls: string[] = []
    const actions = createRuntimeMarketplaceActions(state, async () => {
      calls.push('load')
    }, {
      listMarketplaceInstalledPlugins: async () => [],
      listMarketplaceOutdatedPlugins: async () => [],
      searchMarketplacePlugins: async () => [],
      syncMarketplaceRegistry: async () => ({
        registry_id: 'default',
        registry_url: 'https://registry.example.test',
        plugin_count: 0,
      }),
      installMarketplacePlugin: async () => ({
        plugin_id: 'demo/plugin',
        version: '1.0.0',
        kind: 'wasm',
        artifact_path: '/downloads/demo-plugin-1.0.0.wasm',
        config_path: '/plugins/demo/plugin.json',
        dry_run: true,
      }),
      uninstallMarketplacePlugin: async () => [],
      upgradeMarketplacePlugins: async () => {
        calls.push('upgradeMarketplacePlugins:all')
        return [{
          plugin_id: 'demo/plugin',
          previous_version: '1.0.0',
          installed_version: '1.0.0',
          upgraded: false,
          outcome: null,
        }]
      },
    })

    await actions.upgradeMarketplacePluginAction()

    expect(calls).toEqual([
      'upgradeMarketplacePlugins:all',
      'load',
    ])
    expect(state.actionMessage.value).toBe('Marketplace plugins are already up to date.')
  })

  test('skips search and install when registry inputs are missing', async () => {
    const { actions, calls, state } = createState()
    state.marketplaceRegistryUrl.value = ' '
    state.marketplaceInstallSpec.value = ' '

    await actions.loadMarketplacePanel()
    await actions.installMarketplacePluginAction()
    await actions.syncMarketplaceRegistryAction()

    expect(calls).toEqual([
      'listMarketplaceInstalledPlugins',
      'listMarketplaceOutdatedPlugins',
    ])
    expect(state.marketplacePlugins.value).toEqual([])
  })
})
