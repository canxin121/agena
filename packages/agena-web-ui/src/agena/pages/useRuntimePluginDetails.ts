import { computed, type Ref } from 'vue'

import {
  getPlugin,
  listPluginLogs,
  setSettings,
  type PluginInspect,
  type PluginLogEntry,
  type ConfigSettingsSetRequest,
} from '../lib/agenaApi'
import { mergePluginLogs, pluginLogCursor } from './runtimePageModel'
import type { PluginsTab } from './runtimePageStateModel'

export type RuntimePluginDetailsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  activePluginsTab: Ref<PluginsTab>
  loadPageState: () => Promise<void>
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
  setSettings: (input: ConfigSettingsSetRequest) => Promise<unknown>
  setInterval: (callback: () => void, delayMs: number) => ReturnType<typeof setInterval>
  clearInterval: (timer: ReturnType<typeof setInterval>) => void
}

const defaultDeps: RuntimePluginDetailsDeps = {
  getPlugin,
  listPluginLogs,
  setSettings,
  setInterval: globalThis.setInterval,
  clearInterval: globalThis.clearInterval,
}

export function useRuntimePluginDetails(
  input: RuntimePluginDetailsInput,
  deps: RuntimePluginDetailsDeps = defaultDeps,
) {
  const pluginLogsEnabled = computed(
    () =>
      input.routeSection.value === 'plugins' &&
      input.activePluginsTab.value === 'installed' &&
      !!input.selectedPluginId.value,
  )
  const canTogglePluginConfig = computed(() => {
    const plugin = input.selectedPlugin.value
    return !!plugin?.entry && typeof plugin.entry === 'object'
  })

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

  function pluginConfigPath(pluginId: string): string {
    return `plugins.list.${JSON.stringify(pluginId)}`
  }

  function clonePluginEntry(entry: Record<string, unknown>, disabled: boolean): Record<string, unknown> {
    const next = JSON.parse(JSON.stringify(entry)) as Record<string, unknown>
    next.disabled = disabled
    return next
  }

  async function setSelectedPluginDisabled(disabled: boolean) {
    const plugin = input.selectedPlugin.value
    const pluginId = plugin?.status.plugin_id?.trim() || ''
    const entry =
      plugin?.entry && typeof plugin.entry === 'object' && !Array.isArray(plugin.entry) ? plugin.entry : null
    if (!pluginId || !entry || !canTogglePluginConfig.value) return
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.setSettings({
        path: pluginConfigPath(pluginId),
        value: clonePluginEntry(entry as Record<string, unknown>, disabled),
        validate: true,
        reload: true,
      })
      input.actionMessage.value = disabled
        ? `Disabled plugin ${pluginId}; config kept and runtime reloaded.`
        : `Enabled plugin ${pluginId}; config kept and runtime reloaded.`
      await input.loadPageState()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
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
      const [plugin, logs] = await Promise.all([deps.getPlugin(pluginId), deps.listPluginLogs(pluginId, { limit: 50 })])
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
    canTogglePluginConfig,
    loadPluginDetails,
    refreshPluginLogsIncrementally,
    setSelectedPluginDisabled,
    stopPluginLogPolling,
    syncPluginLogPolling,
  }
}
