import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useSettingsProvidersState } from './useSettingsProvidersState'

export type SettingsProvidersPageStateSource = {
  authProviders: Parameters<typeof useSettingsProvidersState>[0]['authProviders']
  drafts: Parameters<typeof useSettingsProvidersState>[0]['drafts']
  saveApiKey: Parameters<typeof useSettingsProvidersState>[0]['saveApiKey']
  refreshCredential: Parameters<typeof useSettingsProvidersState>[0]['refreshCredential']
  clearCredential: Parameters<typeof useSettingsProvidersState>[0]['clearCredential']
}

export type SettingsProvidersPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'settings'
  }) => {
    shared: RuntimeSectionSharedState
    state: SettingsProvidersPageStateSource
  }
}

const defaultDeps: SettingsProvidersPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & SettingsProvidersPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: SettingsProvidersPageStateSource
    },
}

export function createSettingsProvidersPanelState(state: SettingsProvidersPageStateSource) {
  return useSettingsProvidersState({
    authProviders: state.authProviders,
    drafts: state.drafts,
    saveApiKey: state.saveApiKey,
    refreshCredential: state.refreshCredential,
    clearCredential: state.clearCredential,
  })
}

export function useSettingsProvidersPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: SettingsProvidersPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'settings' })
  const providers = createSettingsProvidersPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    providers,
  }
}
