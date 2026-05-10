import type { Ref } from 'vue'

import type { PluginInspect, PluginLogEntry, PluginStatus } from '../lib/agenaApi'

export type PluginsInstalledStateInput = {
  loadPluginDetails: (pluginId: string) => void | Promise<void>
  openPluginLogsWorkspacePath: () => void
  openPluginManifestInWorkspace: () => void
  pluginLoading: Ref<boolean>
  pluginLogs: Ref<PluginLogEntry[]>
  plugins: Ref<PluginStatus[]>
  selectedPlugin: Ref<PluginInspect | null>
}

export function usePluginsInstalledState(input: PluginsInstalledStateInput) {
  return {
    loadPluginDetails: input.loadPluginDetails,
    openPluginLogsWorkspacePath: input.openPluginLogsWorkspacePath,
    openPluginManifestInWorkspace: input.openPluginManifestInWorkspace,
    pluginLoading: input.pluginLoading,
    pluginLogs: input.pluginLogs,
    plugins: input.plugins,
    selectedPlugin: input.selectedPlugin,
  }
}
