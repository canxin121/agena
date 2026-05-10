import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'
import type { RouteLocationNormalizedLoaded } from 'vue-router'

import type { PluginsTab, RuntimeRouteSection, RuntimeTab, SettingsTab } from './runtimePageStateModel'
import { useRuntimeRouteState } from './useRuntimeRouteState'

function createState() {
  const calls: Array<{ path: string; query: Record<string, string> }> = []
  const state = {
    activePluginsTab: ref<PluginsTab>('installed'),
    activeSettingsTab: ref<SettingsTab>('providers'),
    activeTab: ref<RuntimeTab>('overview'),
    routePath: ref('/runtime/workflow'),
    routeQuery: { search: '1' } as RouteLocationNormalizedLoaded['query'],
    routeSection: ref<RuntimeRouteSection>('runtime'),
  }
  const routeState = useRuntimeRouteState(state, {
    router: {
      replace: async (value) => {
        calls.push(value as { path: string; query: Record<string, string> })
      },
    },
  })

  return { calls, routeState, state }
}

describe('useRuntimeRouteState', () => {
  test('syncs active tabs from route path and legacy query by section', () => {
    const { routeState, state } = createState()

    routeState.syncTabsFromRoute()
    expect(state.activeTab.value).toBe('workflow')

    state.routeSection.value = 'settings'
    state.routePath.value = '/settings'
    state.routeQuery = { settingsTab: 'desktop' }
    routeState.syncTabsFromRoute()
    expect(state.activeSettingsTab.value).toBe('desktop')

    state.routeSection.value = 'plugins'
    state.routePath.value = '/plugins'
    state.routeQuery = { pluginsTab: 'marketplace' }
    routeState.syncTabsFromRoute()
    expect(state.activePluginsTab.value).toBe('marketplace')
  })

  test('updates route path with preserved non-legacy query', async () => {
    const { calls, routeState, state } = createState()

    state.routeSection.value = 'settings'
    state.routeQuery = { search: '1', settingsTab: 'desktop', tab: 'workflow' }
    await routeState.updateRoutePath('permissions')

    expect(calls).toEqual([
      { path: '/settings/permissions', query: { search: '1' } },
    ])
  })
})
