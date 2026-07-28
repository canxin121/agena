import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { usePluginsInstalledState } from './usePluginsInstalledState'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type PluginsInstalledPageStateSource = {
  canTogglePluginConfig: Parameters<typeof usePluginsInstalledState>[0]['canTogglePluginConfig']
  loadPluginDetails: Parameters<typeof usePluginsInstalledState>[0]['loadPluginDetails']
  openPluginLogsWorkspacePath: Parameters<typeof usePluginsInstalledState>[0]['openPluginLogsWorkspacePath']
  openPluginManifestInWorkspace: Parameters<typeof usePluginsInstalledState>[0]['openPluginManifestInWorkspace']
  pluginLoading: Parameters<typeof usePluginsInstalledState>[0]['pluginLoading']
  pluginLogs: Parameters<typeof usePluginsInstalledState>[0]['pluginLogs']
  pluginUiPresentation: Parameters<typeof usePluginsInstalledState>[0]['pluginUiPresentation']
  plugins: Parameters<typeof usePluginsInstalledState>[0]['plugins']
  selectedPlugin: Parameters<typeof usePluginsInstalledState>[0]['selectedPlugin']
  setSelectedPluginDisabled: Parameters<typeof usePluginsInstalledState>[0]['setSelectedPluginDisabled']
}

export type PluginsInstalledPageStateDeps = {
  useRuntimeSectionState: (input: { route: RouteLocationNormalizedLoaded; router: Router; section: 'plugins' }) => {
    shared: RuntimeSectionSharedState
    state: PluginsInstalledPageStateSource
  }
}

const defaultDeps: PluginsInstalledPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & PluginsInstalledPageStateSource>(
      input,
    ) as {
      shared: RuntimeSectionSharedState
      state: PluginsInstalledPageStateSource
    },
}

export function createPluginsInstalledPanelState(state: PluginsInstalledPageStateSource) {
  return usePluginsInstalledState({
    canTogglePluginConfig: state.canTogglePluginConfig,
    loadPluginDetails: state.loadPluginDetails,
    openPluginLogsWorkspacePath: state.openPluginLogsWorkspacePath,
    openPluginManifestInWorkspace: state.openPluginManifestInWorkspace,
    pluginLoading: state.pluginLoading,
    pluginLogs: state.pluginLogs,
    pluginUiPresentation: state.pluginUiPresentation,
    plugins: state.plugins,
    selectedPlugin: state.selectedPlugin,
    setSelectedPluginDisabled: state.setSelectedPluginDisabled,
  })
}

export function usePluginsInstalledPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: PluginsInstalledPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'plugins' })
  const installed = createPluginsInstalledPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    installed,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
  }
}
