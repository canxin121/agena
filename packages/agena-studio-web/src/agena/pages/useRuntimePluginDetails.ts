import { computed, type Ref } from 'vue'

import {
  getPlugin,
  listPluginLogs,
  type PluginInspect,
  type PluginLogEntry,
} from '../lib/agenaApi'
import { mergePluginLogs, pluginLogCursor } from './runtimePageModel'
import type { PluginsTab } from './runtimePageStateModel'

export type RuntimePluginDetailsInput = {
  actionError: Ref<string>
  activePluginsTab: Ref<PluginsTab>
  pluginLoading: Ref<boolean>
  pluginLogs: Ref<PluginLogEntry[]>
  pluginLogPollTimer: Ref<ReturnType<typeof setInterval> | null>
  routeSection: Ref<'runtime' | 'settings' | 'plugins'>
  selectedPlugin: Ref<PluginInspect | null>
  selectedPluginId: Ref<string>
}

export type RuntimePluginDetailsDeps = {
  getPlugin: typeof getPlugin
  listPluginLogs: typeof listPluginLogs
  setInterval: (callback: () => void, delayMs: number) => ReturnType<typeof setInterval>
  clearInterval: (timer: ReturnType<typeof setInterval>) => void
}

const defaultDeps: RuntimePluginDetailsDeps = {
  getPlugin,
  listPluginLogs,
  setInterval: globalThis.setInterval,
  clearInterval: globalThis.clearInterval,
}

export function useRuntimePluginDetails(
  input: RuntimePluginDetailsInput,
  deps: RuntimePluginDetailsDeps = defaultDeps,
) {
  const pluginLogsEnabled = computed(
    () => input.routeSection.value === 'plugins' && input.activePluginsTab.value === 'installed' && !!input.selectedPluginId.value,
  )

  function stopPluginLogPolling() {
    if (!input.pluginLogPollTimer.value) return
    deps.clearInterval(input.pluginLogPollTimer.value)
    input.pluginLogPollTimer.value = null
  }

  async function refreshPluginLogsIncrementally() {
    const pluginId = input.selectedPluginId.value
    if (!pluginId) return
    const afterSeq = pluginLogCursor(input.pluginLogs.value)
    const incoming = await deps.listPluginLogs(pluginId, {
      limit: 50,
      ...(afterSeq != null ? { afterSeq } : {}),
    })
    if (!incoming.length) return
    input.pluginLogs.value = mergePluginLogs(input.pluginLogs.value, incoming)
  }

  function syncPluginLogPolling() {
    stopPluginLogPolling()
    if (!pluginLogsEnabled.value) return
    input.pluginLogPollTimer.value = deps.setInterval(() => {
      void refreshPluginLogsIncrementally()
    }, 1_500)
  }

  async function loadPluginDetails(pluginId: string) {
    if (!pluginId) {
      input.selectedPlugin.value = null
      input.pluginLogs.value = []
      stopPluginLogPolling()
      return
    }
    input.pluginLoading.value = true
    input.actionError.value = ''
    try {
      const [plugin, logs] = await Promise.all([
        deps.getPlugin(pluginId),
        deps.listPluginLogs(pluginId, { limit: 50 }),
      ])
      input.selectedPluginId.value = pluginId
      input.selectedPlugin.value = plugin
      input.pluginLogs.value = logs
      syncPluginLogPolling()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.pluginLoading.value = false
    }
  }

  return {
    loadPluginDetails,
    refreshPluginLogsIncrementally,
    stopPluginLogPolling,
    syncPluginLogPolling,
  }
}
