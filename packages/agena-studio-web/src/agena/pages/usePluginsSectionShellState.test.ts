import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createPluginsSectionShellState, usePluginsSectionShellState } from './usePluginsSectionShellState'

describe('usePluginsSectionShellState', () => {
  test('assembles plugins shell state from provided runtime source', () => {
    const shell = createPluginsSectionShellState({
      activePluginsTab: ref('installed'),
      visibleTabs: computed(() => [{ id: 'installed', label: 'Installed' }]),
    })

    expect(shell.activeTab.value).toBe('installed')
    expect(shell.tabs.value[0]?.label).toBe('Installed')
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

    const result = usePluginsSectionShellState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'plugins' })
          return {
            shared,
            state: {
              activePluginsTab: ref('installed'),
              visibleTabs: computed(() => [{ id: 'installed', label: 'Installed' }]),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.shell.activeTab.value).toBe('installed')
  })
})
