import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

const panels = {
  installed: {
    plugins: [
      {
        plugin_id: 'alpha',
        kind: 'builtin',
        state: 'running',
        restart_count: 2,
        last_error: '',
      },
    ],
    selectedPlugin: {
      status: {
        plugin_id: 'alpha',
        kind: 'builtin',
        state: 'running',
        pid: 1234,
        last_exit_code: null,
        last_restart_at_ms: 10,
      },
      manifest: { name: 'Alpha Plugin' },
    },
    pluginLogs: [{ seq: 1, level: 'info', target: 'plugin', timestamp_ms: 1, message: 'started' }],
    pluginLoading: false,
    loadPluginDetails: () => {},
    openPluginManifestInWorkspace: () => {},
    openPluginLogsWorkspacePath: () => {},
  },
  marketplace: {
    registryUrl: 'https://example.com/index.json',
    registryId: 'default',
    query: 'alpha',
    loading: false,
    installSpec: 'alpha',
    allowUnverified: false,
    requireSignature: true,
    refreshIndex: false,
    cascadeUninstall: false,
    filteredPlugins: [
      {
        plugin_id: 'alpha',
        name: 'Alpha Plugin',
        description: 'Alpha description',
        latest_version: '1.2.3',
        latest_kind: 'wasm',
        latest_platform: 'any',
        version_count: 3,
        homepage: 'https://example.com/alpha',
      },
    ],
    plugins: [{ plugin_id: 'alpha' }],
    installed: [{ plugin_id: 'alpha', version: '1.0.0', kind: 'wasm', platform: 'any', config_path: '/cfg', registry_url: 'https://example.com/index.json' }],
    outdated: [{ plugin_id: 'alpha', installed_version: '1.0.0', latest_version: '1.2.3' }],
    installedPluginIds: new Set(['alpha']),
    searchAction: () => {},
    syncRegistryAction: () => {},
    upgradeAction: () => {},
    installAction: () => {},
    uninstallAction: () => {},
  },
}

describe('PluginsSectionPanelRenderer', () => {
  test('renders installed plugins content', async () => {
    const html = await renderVueSsr('/src/agena/pages/PluginsSectionPanelRenderer.vue', {
      activeTab: 'installed',
      panels,
    })

    expect(html.includes('Plugins')).toBe(true)
    expect(html.includes('Plugin Detail')).toBe(true)
    expect(html.includes('Alpha Plugin')).toBe(true)
    expect(html.includes('started')).toBe(true)
  })

  test('renders marketplace content for non-installed tab', async () => {
    const html = await renderVueSsr('/src/agena/pages/PluginsSectionPanelRenderer.vue', {
      activeTab: 'marketplace',
      panels,
    })

    expect(html.includes('Marketplace')).toBe(true)
    expect(html.includes('Search Registry')).toBe(true)
    expect(html.includes('Alpha Plugin')).toBe(true)
    expect(html.includes('Outdated')).toBe(true)
  })
})
