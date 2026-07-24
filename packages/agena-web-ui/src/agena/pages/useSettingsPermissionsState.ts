import type { ComputedRef, Ref } from 'vue'

import type {
  ConfigSettingsReadResponse,
  PermissionMode,
  PermissionRuleResource,
  PermissionSubjectKind,
} from '../lib/agenaApi'
import type { RuntimePermissionDraft } from './useRuntimePermissionActions'

export type SettingsPermissionsStateInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  permissionConfig: Ref<ConfigSettingsReadResponse | null>
  editingPermissionRuleId: Ref<number | null>
  filteredPermissionRules: ComputedRef<PermissionRuleResource[]>
  permissionDraft: RuntimePermissionDraft
  permissionModeFilter: Ref<'all' | PermissionMode>
  permissionRuleFacts: (rule: PermissionRuleResource) => string[]
  permissionRuleLabel: (rule: PermissionRuleResource) => string
  permissionRulePreview: (rule: PermissionRuleResource) => string
  permissionScopeFilter: Ref<'all' | 'session' | 'workspace' | 'global'>
  permissionSearch: Ref<string>
  permissionStatusFilter: Ref<'all' | 'active' | 'revoked'>
  permissionSubjectFilter: Ref<'all' | PermissionSubjectKind>
  deletePermissionRuleAction: (rule: PermissionRuleResource) => void | Promise<void>
  editPermissionRule: (rule: PermissionRuleResource) => void
  resetPermissionDraft: () => void
  revokePermissionRuleAction: (rule: PermissionRuleResource) => void | Promise<void>
  savePermissionRule: () => void | Promise<void>
}

export function useSettingsPermissionsState(input: SettingsPermissionsStateInput) {
  return {
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    load: input.load,
    permissionConfig: input.permissionConfig,
    editingPermissionRuleId: input.editingPermissionRuleId,
    filteredPermissionRules: input.filteredPermissionRules,
    permissionDraft: input.permissionDraft,
    permissionModeFilter: input.permissionModeFilter,
    permissionRuleFacts: input.permissionRuleFacts,
    permissionRuleLabel: input.permissionRuleLabel,
    permissionRulePreview: input.permissionRulePreview,
    permissionScopeFilter: input.permissionScopeFilter,
    permissionSearch: input.permissionSearch,
    permissionStatusFilter: input.permissionStatusFilter,
    permissionSubjectFilter: input.permissionSubjectFilter,
    deletePermissionRuleAction: input.deletePermissionRuleAction,
    editPermissionRule: input.editPermissionRule,
    resetPermissionDraft: input.resetPermissionDraft,
    revokePermissionRuleAction: input.revokePermissionRuleAction,
    savePermissionRule: input.savePermissionRule,
  }
}
