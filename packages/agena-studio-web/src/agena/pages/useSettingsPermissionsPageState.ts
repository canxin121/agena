import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useSettingsPermissionsState } from './useSettingsPermissionsState'

export type SettingsPermissionsPageStateSource = {
  permissionDraft: Parameters<typeof useSettingsPermissionsState>[0]['permissionDraft']
  editPermissionRule: Parameters<typeof useSettingsPermissionsState>[0]['editPermissionRule']
  editingPermissionRuleId: Parameters<typeof useSettingsPermissionsState>[0]['editingPermissionRuleId']
  filteredPermissionRules: Parameters<typeof useSettingsPermissionsState>[0]['filteredPermissionRules']
  permissionModeFilter: Parameters<typeof useSettingsPermissionsState>[0]['permissionModeFilter']
  permissionRuleFacts: Parameters<typeof useSettingsPermissionsState>[0]['permissionRuleFacts']
  permissionRuleLabel: Parameters<typeof useSettingsPermissionsState>[0]['permissionRuleLabel']
  permissionRulePreview: Parameters<typeof useSettingsPermissionsState>[0]['permissionRulePreview']
  permissionScopeFilter: Parameters<typeof useSettingsPermissionsState>[0]['permissionScopeFilter']
  permissionSearch: Parameters<typeof useSettingsPermissionsState>[0]['permissionSearch']
  permissionStatusFilter: Parameters<typeof useSettingsPermissionsState>[0]['permissionStatusFilter']
  permissionSubjectFilter: Parameters<typeof useSettingsPermissionsState>[0]['permissionSubjectFilter']
  savePermissionRule: Parameters<typeof useSettingsPermissionsState>[0]['savePermissionRule']
  resetPermissionDraft: Parameters<typeof useSettingsPermissionsState>[0]['resetPermissionDraft']
  revokePermissionRuleAction: Parameters<typeof useSettingsPermissionsState>[0]['revokePermissionRuleAction']
  deletePermissionRuleAction: Parameters<typeof useSettingsPermissionsState>[0]['deletePermissionRuleAction']
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

export function createSettingsPermissionsPanelState(state: SettingsPermissionsPageStateSource) {
  return useSettingsPermissionsState({
    permissionDraft: state.permissionDraft,
    editPermissionRule: state.editPermissionRule,
    editingPermissionRuleId: state.editingPermissionRuleId,
    filteredPermissionRules: state.filteredPermissionRules,
    permissionModeFilter: state.permissionModeFilter,
    permissionRuleFacts: state.permissionRuleFacts,
    permissionRuleLabel: state.permissionRuleLabel,
    permissionRulePreview: state.permissionRulePreview,
    permissionScopeFilter: state.permissionScopeFilter,
    permissionSearch: state.permissionSearch,
    permissionStatusFilter: state.permissionStatusFilter,
    permissionSubjectFilter: state.permissionSubjectFilter,
    savePermissionRule: state.savePermissionRule,
    resetPermissionDraft: state.resetPermissionDraft,
    revokePermissionRuleAction: state.revokePermissionRuleAction,
    deletePermissionRuleAction: state.deletePermissionRuleAction,
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
  const permissions = createSettingsPermissionsPanelState(state)

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
