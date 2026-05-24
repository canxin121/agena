import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useSettingsPermissionsState } from './useSettingsPermissionsState'

export type SettingsPermissionsPageStateSource = {
  permissionConfig: Parameters<typeof useSettingsPermissionsState>[0]['permissionConfig']
}

export type SettingsPermissionsPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'settings'
  }) => {
    shared: RuntimeSectionSharedState
    state: SettingsPermissionsPageStateSource
  }
}

const defaultDeps: SettingsPermissionsPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & SettingsPermissionsPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: SettingsPermissionsPageStateSource
    },
}

export function createSettingsPermissionsPanelState(
  state: SettingsPermissionsPageStateSource,
  shared: RuntimeSectionSharedState,
) {
  return useSettingsPermissionsState({
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    permissionConfig: state.permissionConfig,
  })
}

export function useSettingsPermissionsPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: SettingsPermissionsPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'settings' })
  const permissions = createSettingsPermissionsPanelState(state, shared)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    permissions,
  }
}
