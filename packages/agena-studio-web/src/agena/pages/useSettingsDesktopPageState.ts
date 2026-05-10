import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useSettingsDesktopState } from './useSettingsDesktopState'

export type SettingsDesktopPageStateSource = {
  backendErrorFacts: Parameters<typeof useSettingsDesktopState>[0]['backendErrorFacts']
  backendUrl: Parameters<typeof useSettingsDesktopState>[0]['backendUrl']
  config: Parameters<typeof useSettingsDesktopState>[0]['config']
  configFacts: Parameters<typeof useSettingsDesktopState>[0]['configFacts']
  enabled: Parameters<typeof useSettingsDesktopState>[0]['enabled']
  form: Parameters<typeof useSettingsDesktopState>[0]['form']
  installerAssetName: Parameters<typeof useSettingsDesktopState>[0]['installerAssetName']
  installerUpdateUrl: Parameters<typeof useSettingsDesktopState>[0]['installerUpdateUrl']
  notice: Parameters<typeof useSettingsDesktopState>[0]['notice']
  runtimeFacts: Parameters<typeof useSettingsDesktopState>[0]['runtimeFacts']
  saving: Parameters<typeof useSettingsDesktopState>[0]['saving']
  serviceUpdateUrl: Parameters<typeof useSettingsDesktopState>[0]['serviceUpdateUrl']
  statusFacts: Parameters<typeof useSettingsDesktopState>[0]['statusFacts']
  updateFacts: Parameters<typeof useSettingsDesktopState>[0]['updateFacts']
  updateProgressPercent: Parameters<typeof useSettingsDesktopState>[0]['updateProgressPercent']
  updateRunning: Parameters<typeof useSettingsDesktopState>[0]['updateRunning']
  loadPanel: Parameters<typeof useSettingsDesktopState>[0]['loadPanel']
  openBackendUrlAction: Parameters<typeof useSettingsDesktopState>[0]['openBackendUrlAction']
  openConfigAction: Parameters<typeof useSettingsDesktopState>[0]['openConfigAction']
  refreshUpdateProgressAction: Parameters<typeof useSettingsDesktopState>[0]['refreshUpdateProgressAction']
  restartBackendAction: Parameters<typeof useSettingsDesktopState>[0]['restartBackendAction']
  runInstallerUpdateAction: Parameters<typeof useSettingsDesktopState>[0]['runInstallerUpdateAction']
  runServiceUpdateAction: Parameters<typeof useSettingsDesktopState>[0]['runServiceUpdateAction']
  saveConfigAction: Parameters<typeof useSettingsDesktopState>[0]['saveConfigAction']
}

export type SettingsDesktopPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'settings'
  }) => {
    shared: RuntimeSectionSharedState
    state: SettingsDesktopPageStateSource
  }
}

const defaultDeps: SettingsDesktopPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & SettingsDesktopPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: SettingsDesktopPageStateSource
    },
}

export function createSettingsDesktopPanelState(state: SettingsDesktopPageStateSource) {
  return useSettingsDesktopState({
    backendErrorFacts: state.backendErrorFacts,
    backendUrl: state.backendUrl,
    config: state.config,
    configFacts: state.configFacts,
    enabled: state.enabled,
    form: state.form,
    installerAssetName: state.installerAssetName,
    installerUpdateUrl: state.installerUpdateUrl,
    notice: state.notice,
    runtimeFacts: state.runtimeFacts,
    saving: state.saving,
    serviceUpdateUrl: state.serviceUpdateUrl,
    statusFacts: state.statusFacts,
    updateFacts: state.updateFacts,
    updateProgressPercent: state.updateProgressPercent,
    updateRunning: state.updateRunning,
    loadPanel: state.loadPanel,
    openBackendUrlAction: state.openBackendUrlAction,
    openConfigAction: state.openConfigAction,
    refreshUpdateProgressAction: state.refreshUpdateProgressAction,
    restartBackendAction: state.restartBackendAction,
    runInstallerUpdateAction: state.runInstallerUpdateAction,
    runServiceUpdateAction: state.runServiceUpdateAction,
    saveConfigAction: state.saveConfigAction,
  })
}

export function useSettingsDesktopPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: SettingsDesktopPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'settings' })
  const desktop = createSettingsDesktopPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    desktop,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
  }
}
