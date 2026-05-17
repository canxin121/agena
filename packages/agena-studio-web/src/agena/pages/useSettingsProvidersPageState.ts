import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useSettingsProvidersState } from './useSettingsProvidersState'

export type SettingsProvidersPageStateSource = {
  catalogEntries: Parameters<typeof useSettingsProvidersState>[0]['catalogEntries']
  authProviders: Parameters<typeof useSettingsProvidersState>[0]['authProviders']
  browserAuthCodeDrafts: Parameters<typeof useSettingsProvidersState>[0]['browserAuthCodeDrafts']
  browserAuthInstanceDrafts: Parameters<typeof useSettingsProvidersState>[0]['browserAuthInstanceDrafts']
  browserAuthStartState: Parameters<typeof useSettingsProvidersState>[0]['browserAuthStartState']
  deviceAuthEnterpriseDrafts: Parameters<typeof useSettingsProvidersState>[0]['deviceAuthEnterpriseDrafts']
  deviceAuthStartState: Parameters<typeof useSettingsProvidersState>[0]['deviceAuthStartState']
  drafts: Parameters<typeof useSettingsProvidersState>[0]['drafts']
  providerModels: Parameters<typeof useSettingsProvidersState>[0]['providerModels']
  providers: Parameters<typeof useSettingsProvidersState>[0]['providers']
  finishBrowserAuth: Parameters<typeof useSettingsProvidersState>[0]['finishBrowserAuth']
  pollDeviceAuth: Parameters<typeof useSettingsProvidersState>[0]['pollDeviceAuth']
  saveApiKey: Parameters<typeof useSettingsProvidersState>[0]['saveApiKey']
  refreshCredential: Parameters<typeof useSettingsProvidersState>[0]['refreshCredential']
  clearCredential: Parameters<typeof useSettingsProvidersState>[0]['clearCredential']
  startBrowserAuth: Parameters<typeof useSettingsProvidersState>[0]['startBrowserAuth']
  startDeviceAuth: Parameters<typeof useSettingsProvidersState>[0]['startDeviceAuth']
}

export type SettingsProvidersPanelStateSource = Pick<
  RuntimeSectionSharedState,
  'actionError' | 'actionMessage' | 'load'
> &
  SettingsProvidersPageStateSource

export type SettingsProvidersPageStateDeps = {
  useRuntimeSectionState: (input: { route: RouteLocationNormalizedLoaded; router: Router; section: 'settings' }) => {
    shared: RuntimeSectionSharedState
    state: SettingsProvidersPageStateSource
  }
}

const defaultDeps: SettingsProvidersPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & SettingsProvidersPageStateSource>(
      input,
    ) as {
      shared: RuntimeSectionSharedState
      state: SettingsProvidersPageStateSource
    },
}

export function createSettingsProvidersPanelState(state: SettingsProvidersPanelStateSource) {
  return useSettingsProvidersState({
    actionError: state.actionError,
    actionMessage: state.actionMessage,
    authProviders: state.authProviders,
    browserAuthCodeDrafts: state.browserAuthCodeDrafts,
    browserAuthInstanceDrafts: state.browserAuthInstanceDrafts,
    browserAuthStartState: state.browserAuthStartState,
    deviceAuthEnterpriseDrafts: state.deviceAuthEnterpriseDrafts,
    deviceAuthStartState: state.deviceAuthStartState,
    drafts: state.drafts,
    catalogEntries: state.catalogEntries,
    load: state.load,
    providerModels: state.providerModels,
    providers: state.providers,
    finishBrowserAuth: state.finishBrowserAuth,
    pollDeviceAuth: state.pollDeviceAuth,
    saveApiKey: state.saveApiKey,
    refreshCredential: state.refreshCredential,
    clearCredential: state.clearCredential,
    startBrowserAuth: state.startBrowserAuth,
    startDeviceAuth: state.startDeviceAuth,
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
  const providers = createSettingsProvidersPanelState({ ...shared, ...state })

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
