import type { Ref } from 'vue'

import type { PluginInspect, PluginLogEntry, PluginStatus } from '../lib/agenaApi'
import type { SettingsPluginUiPresentationSnapshot } from './runtimePageLoaders'

export type PluginsInstalledStateInput = {
  canTogglePluginConfig: Ref<boolean>
  loadPluginDetails: (pluginId: string) => void | Promise<void>
  openPluginLogsWorkspacePath: () => void
  openPluginManifestInWorkspace: () => void
  pluginLoading: Ref<boolean>
  pluginLogs: Ref<PluginLogEntry[]>
  pluginUiPresentation: Ref<SettingsPluginUiPresentationSnapshot | null>
  plugins: Ref<PluginStatus[]>
  selectedPlugin: Ref<PluginInspect | null>
  setSelectedPluginDisabled: (disabled: boolean) => void | Promise<void>
}

export function usePluginsInstalledState(input: PluginsInstalledStateInput) {
  return {
    canTogglePluginConfig: input.canTogglePluginConfig,
    loadPluginDetails: input.loadPluginDetails,
    openPluginLogsWorkspacePath: input.openPluginLogsWorkspacePath,
    openPluginManifestInWorkspace: input.openPluginManifestInWorkspace,
    pluginLoading: input.pluginLoading,
    pluginLogs: input.pluginLogs,
    pluginUiPresentation: input.pluginUiPresentation,
    plugins: input.plugins,
    selectedPlugin: input.selectedPlugin,
    setSelectedPluginDisabled: input.setSelectedPluginDisabled,
  }
}
