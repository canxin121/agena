import { describe, expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'

import type { DesktopUpdateProgress } from '../../lib/desktopConfig'
import type { PluginsTab, RuntimeRouteSection, RuntimeTab, SettingsTab } from './runtimePageStateModel'
import { useRuntimeRouteLifecycle } from './useRuntimeRouteLifecycle'

function createState() {
  const calls: string[] = []
  const state = {
    activePluginsTab: ref<PluginsTab>('installed'),
    activeSettingsTab: ref<SettingsTab>('providers'),
    activeTab: ref<RuntimeTab>('overview'),
    desktopEnabled: ref(true),
    desktopUpdate: ref<DesktopUpdateProgress | null>(null),
    desktopUpdateRunning: ref(false),
    load: async () => {
      calls.push('load')
    },
    loadDesktopPanel: async () => {
      calls.push('loadDesktopPanel')
    },
    loadMarketplacePanel: async () => {
      calls.push('loadMarketplacePanel')
    },
    routePath: ref('/runtime'),
    routeSection: ref<RuntimeRouteSection>('runtime'),
    stopPluginLogPolling: () => {
      calls.push('stopPluginLogPolling')
    },
    syncPluginLogPolling: () => {
      calls.push('syncPluginLogPolling')
    },
    syncTabsFromRoute: () => {
      calls.push('syncTabsFromRoute')
    },
    updateRoutePath: async (tab: string) => {
      calls.push(`updateRoutePath:${tab}`)
    },
  }

  return { calls, state }
}

describe('useRuntimeRouteLifecycle', () => {
  test('syncs runtime tab path while in runtime section', async () => {
    const { calls, state } = createState()
    const scope = effectScope()
    scope.run(() => {
      useRuntimeRouteLifecycle(state, { registerComponentLifecycle: false })
    })

    state.activeTab.value = 'workflow'
    await Promise.resolve()

    expect(calls.includes('stopPluginLogPolling')).toBe(true)
    expect(calls.includes('updateRoutePath:workflow')).toBe(true)

    scope.stop()
  })

  test('syncs plugin/settings watchers and desktop update state', async () => {
    const { calls, state } = createState()
    const scope = effectScope()
    scope.run(() => {
      useRuntimeRouteLifecycle(state, { registerComponentLifecycle: false })
    })

    state.routeSection.value = 'plugins'
    state.activePluginsTab.value = 'marketplace'
    await Promise.resolve()

    state.activeSettingsTab.value = 'desktop'
    state.routeSection.value = 'settings'
    state.desktopUpdate.value = {
      running: true,
      kind: 'service',
      phase: 'download',
      message: 'downloading',
      downloadedBytes: 5,
      totalBytes: 10,
      error: null,
    }
    state.desktopUpdate.value = {
      running: false,
      kind: '',
      phase: '',
      message: '',
      downloadedBytes: 0,
      totalBytes: null,
      error: null,
    }
    state.routePath.value = '/settings/desktop'
    await Promise.resolve()

    expect(calls.includes('syncPluginLogPolling')).toBe(true)
    expect(calls.includes('loadMarketplacePanel')).toBe(true)
    expect(calls.includes('loadDesktopPanel')).toBe(true)
    expect(calls.includes('syncTabsFromRoute')).toBe(true)
    expect(state.desktopUpdateRunning.value).toBe(false)

    scope.stop()
  })
})
