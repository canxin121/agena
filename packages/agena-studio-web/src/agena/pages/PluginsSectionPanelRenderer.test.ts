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
      manifest: {
        name: 'Alpha Plugin',
        ui: {
          studio: {
            commands: [
              {
                id: 'summarize',
                title: 'Summarize Workspace',
                description: 'Create a workspace summary.',
                action: { kind: 'invoke_tool', tool: 'summarize' },
              },
            ],
            controls: [
              {
                id: 'mode',
                title: 'Planning Mode',
                kind: 'select',
                value: 'fast',
                options: [
                  { label: 'Fast', value: 'fast' },
                  { label: 'Careful', value: 'careful' },
                ],
                action: { kind: 'invoke_tool', tool: 'set_mode' },
              },
              {
                id: 'enabled',
                title: 'Enabled',
                kind: 'toggle',
                value: true,
                action: { kind: 'invoke_tool', tool: 'set_enabled' },
              },
            ],
            views: [
              {
                id: 'report',
                title: 'Report',
                kind: 'markdown',
                content: 'Current plugin report',
                controls: [
                  {
                    id: 'refresh',
                    title: 'Refresh Report',
                    kind: 'button',
                    action: { kind: 'invoke_tool', tool: 'refresh' },
                  },
                ],
              },
            ],
          },
        },
      },
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
    installed: [
      {
        plugin_id: 'alpha',
        version: '1.0.0',
        kind: 'wasm',
        platform: 'any',
        config_path: '/cfg',
        registry_url: 'https://example.com/index.json',
      },
    ],
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
    expect(html.includes('Summarize Workspace')).toBe(true)
    expect(html.includes('Planning Mode')).toBe(true)
    expect(html.includes('Current plugin report')).toBe(true)
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
