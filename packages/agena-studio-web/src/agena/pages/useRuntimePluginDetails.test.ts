import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { PluginInspect, PluginLogEntry } from '../lib/agenaApi'
import type { PluginsTab, RuntimeRouteSection } from './runtimePageStateModel'
import { useRuntimePluginDetails } from './useRuntimePluginDetails'

function createState() {
  return {
    actionError: ref(''),
    activePluginsTab: ref<PluginsTab>('installed'),
    pluginLoading: ref(false),
    pluginLogs: ref<PluginLogEntry[]>([{ seq: 3, plugin_id: 'demo/plugin', level: 'info', target: 'plugin', message: 'existing', timestamp_ms: 1 }]),
    pluginLogPollTimer: ref<ReturnType<typeof setInterval> | null>(null),
    routeSection: ref<RuntimeRouteSection>('plugins'),
    selectedPlugin: ref<PluginInspect | null>(null),
    selectedPluginId: ref('demo/plugin'),
  }
}

describe('useRuntimePluginDetails', () => {
  test('loadPluginDetails hydrates plugin details and starts polling', async () => {
    const state = createState()
    const calls: string[] = []
    const timers: Array<() => void> = []
    const timerToken = {} as ReturnType<typeof setInterval>
    const pluginDetails = useRuntimePluginDetails(state, {
      clearInterval: () => {
        calls.push('clearInterval')
      },
      getPlugin: async (pluginId) => {
        calls.push(`getPlugin:${pluginId}`)
        return {
          status: {
            plugin_id: pluginId,
            kind: 'wasm',
            state: 'ready',
            restart_count: 0,
          },
          manifest: { metadata: { name: 'Demo Plugin' } },
        }
      },
      listPluginLogs: async (pluginId, options) => {
        calls.push(`listPluginLogs:${pluginId}:${options?.afterSeq ?? 'initial'}`)
        if (options?.afterSeq != null) {
          return [{ seq: 4, plugin_id: pluginId, level: 'warn', target: 'plugin', message: 'new', timestamp_ms: 4 }]
        }
        return [{ seq: 2, plugin_id: pluginId, level: 'info', target: 'plugin', message: 'older', timestamp_ms: 2 }]
      },
      setInterval: (fn) => {
        calls.push('setInterval')
        timers.push(fn)
        return timerToken
      },
    })

    await pluginDetails.loadPluginDetails('demo/plugin')

    expect(calls).toEqual([
      'getPlugin:demo/plugin',
      'listPluginLogs:demo/plugin:initial',
      'setInterval',
    ])
    expect(state.selectedPlugin.value?.status.plugin_id).toBe('demo/plugin')
    expect(state.pluginLogs.value.map((entry) => entry.seq)).toEqual([2])
    expect(state.pluginLogPollTimer.value != null).toBe(true)

    await timers[0]?.()

    expect(calls).toEqual([
      'getPlugin:demo/plugin',
      'listPluginLogs:demo/plugin:initial',
      'setInterval',
      'listPluginLogs:demo/plugin:2',
    ])
    expect(state.pluginLogs.value.map((entry) => entry.seq)).toEqual([2, 4])
  })

  test('loadPluginDetails clears state when plugin id is empty', async () => {
    const state = createState()
    const timerToken = {} as ReturnType<typeof setInterval>
    state.selectedPlugin.value = {
      status: {
        plugin_id: 'demo/plugin',
        kind: 'wasm',
        state: 'ready',
        restart_count: 0,
      },
      manifest: { metadata: { name: 'Demo Plugin' } },
    }
    state.pluginLogPollTimer.value = timerToken

    const calls: string[] = []
    const pluginDetails = useRuntimePluginDetails(state, {
      clearInterval: () => {
        calls.push('clearInterval')
      },
      getPlugin: async () => {
        throw new Error('should not load plugin')
      },
      listPluginLogs: async () => {
        throw new Error('should not load logs')
      },
      setInterval: () => {
        throw new Error('should not start polling')
      },
    })

    await pluginDetails.loadPluginDetails('')

    expect(calls).toEqual(['clearInterval'])
    expect(state.selectedPlugin.value === null).toBe(true)
    expect(state.pluginLogs.value).toEqual([])
    expect(state.pluginLogPollTimer.value === null).toBe(true)
  })

  test('syncPluginLogPolling skips polling outside installed plugins tab', () => {
    const state = createState()
    const timerToken = {} as ReturnType<typeof setInterval>
    state.activePluginsTab.value = 'marketplace'
    state.pluginLogPollTimer.value = timerToken

    const calls: string[] = []
    const pluginDetails = useRuntimePluginDetails(state, {
      clearInterval: () => {
        calls.push('clearInterval')
      },
      getPlugin: async () => {
        throw new Error('unused')
      },
      listPluginLogs: async () => {
        throw new Error('unused')
      },
      setInterval: () => {
        calls.push('setInterval')
        return 123 as unknown as ReturnType<typeof setInterval>
      },
    })

    pluginDetails.syncPluginLogPolling()

    expect(calls).toEqual(['clearInterval'])
    expect(state.pluginLogPollTimer.value === null).toBe(true)
  })
})
