import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useSettingsAgentsState } from './useSettingsAgentsState'

export type SettingsAgentsPageStateSource = {
  runtime: Parameters<typeof useSettingsAgentsState>[0]['runtime']
}

export type SettingsAgentsPanelStateSource = Pick<RuntimeSectionSharedState, 'actionError' | 'actionMessage' | 'load'> &
  SettingsAgentsPageStateSource

export type SettingsAgentsPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'settings'
  }) => {
    shared: RuntimeSectionSharedState
    state: SettingsAgentsPageStateSource
  }
}

const defaultDeps: SettingsAgentsPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & SettingsAgentsPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: SettingsAgentsPageStateSource
    },
}

export function createSettingsAgentsPanelState(state: SettingsAgentsPanelStateSource) {
  return useSettingsAgentsState({
    actionError: state.actionError,
    actionMessage: state.actionMessage,
    load: state.load,
    runtime: state.runtime,
  })
}

export function useSettingsAgentsPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: SettingsAgentsPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'settings' })
  const agents = createSettingsAgentsPanelState({ ...shared, ...state })

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    agents,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
  }
}
