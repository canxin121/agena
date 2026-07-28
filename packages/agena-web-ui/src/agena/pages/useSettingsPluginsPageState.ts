import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useSettingsPluginsState } from './useSettingsPluginsState'

export type SettingsPluginsPageStateSource = {
  settingsPlugins: Parameters<typeof useSettingsPluginsState>[0]['settingsPlugins']
}

export type SettingsPluginsPanelStateSource = Pick<
  RuntimeSectionSharedState,
  'actionError' | 'actionMessage' | 'load'
> &
  SettingsPluginsPageStateSource

export type SettingsPluginsPageStateDeps = {
  useRuntimeSectionState: (input: { route: RouteLocationNormalizedLoaded; router: Router; section: 'settings' }) => {
    shared: RuntimeSectionSharedState
    state: SettingsPluginsPageStateSource
  }
}

const defaultDeps: SettingsPluginsPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & SettingsPluginsPageStateSource>(
      input,
    ) as {
      shared: RuntimeSectionSharedState
      state: SettingsPluginsPageStateSource
    },
}

export function createSettingsPluginsPanelState(state: SettingsPluginsPanelStateSource) {
  return useSettingsPluginsState({
    actionError: state.actionError,
    actionMessage: state.actionMessage,
    load: state.load,
    settingsPlugins: state.settingsPlugins,
  })
}

export function useSettingsPluginsPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: SettingsPluginsPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'settings' })
  const plugins = createSettingsPluginsPanelState({ ...shared, ...state })

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    plugins,
  }
}
