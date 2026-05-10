import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createPluginsInstalledPanelState, usePluginsInstalledPageState } from './usePluginsInstalledPageState'

describe('usePluginsInstalledPageState', () => {
  test('assembles installed panel state from provided plugins source', () => {
    const installed = createPluginsInstalledPanelState({
      loadPluginDetails: async () => {},
      openPluginLogsWorkspacePath: () => {},
      openPluginManifestInWorkspace: () => {},
      pluginLoading: ref(false),
      pluginLogs: ref([]),
      plugins: ref([]),
      selectedPlugin: ref(null),
    })

    expect(installed.pluginLoading.value).toBe(false)
    expect(installed.plugins.value).toEqual([])
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/plugins/installed' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = usePluginsInstalledPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'plugins' })
          return {
            shared,
            state: {
              loadPluginDetails: async () => {},
              openPluginLogsWorkspacePath: () => {},
              openPluginManifestInWorkspace: () => {},
              pluginLoading: ref(false),
              pluginLogs: ref([]),
              plugins: ref([]),
              selectedPlugin: ref(null),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.installed.plugins.value).toEqual([])
  })
})
