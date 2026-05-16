import type { ComputedRef, Ref } from 'vue'

import type { PermissionMode, PermissionRuleResource, PermissionSubjectKind } from '../lib/agenaApi'

export type SettingsPermissionsStateInput = {
  permissionDraft: {
    subjectKind: PermissionSubjectKind
    toolName: string
    qualifier: string
    pathAccessKind: string
    workspaceRoot: string
    targetPath: string
    networkTarget: string
    networkPort: string
    scope: 'session' | 'workspace' | 'global'
    sessionId: string
    mode: PermissionMode
  }
  editPermissionRule: (rule: PermissionRuleResource) => void
  editingPermissionRuleId: Ref<number | null>
  filteredPermissionRules: ComputedRef<PermissionRuleResource[]>
  permissionModeFilter: Ref<PermissionMode | 'all'>
  permissionRuleFacts: (rule: PermissionRuleResource) => string[]
  permissionRuleLabel: (rule: PermissionRuleResource) => string
  permissionRulePreview: (rule: PermissionRuleResource) => string
  permissionScopeFilter: Ref<'all' | 'session' | 'workspace' | 'global'>
  permissionSearch: Ref<string>
  permissionStatusFilter: Ref<'all' | 'active' | 'revoked'>
  permissionSubjectFilter: Ref<'all' | PermissionSubjectKind>
  savePermissionRule: () => void | Promise<void>
  resetPermissionDraft: () => void
  revokePermissionRuleAction: (rule: PermissionRuleResource) => void | Promise<void>
  deletePermissionRuleAction: (rule: PermissionRuleResource) => void | Promise<void>
}

export function useSettingsPermissionsState(input: SettingsPermissionsStateInput) {
  return {
    deleteRuleAction: input.deletePermissionRuleAction,
    draft: input.permissionDraft,
    editRule: input.editPermissionRule,
    editingRuleId: input.editingPermissionRuleId,
    filteredRules: input.filteredPermissionRules,
    modeFilter: input.permissionModeFilter,
    ruleFacts: input.permissionRuleFacts,
    ruleLabel: input.permissionRuleLabel,
    rulePreview: input.permissionRulePreview,
    scopeFilter: input.permissionScopeFilter,
    search: input.permissionSearch,
    statusFilter: input.permissionStatusFilter,
    subjectFilter: input.permissionSubjectFilter,
    saveRule: input.savePermissionRule,
    resetDraft: input.resetPermissionDraft,
    revokeRuleAction: input.revokePermissionRuleAction,
  }
}
