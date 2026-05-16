import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createSettingsPermissionsPanelState, useSettingsPermissionsPageState } from './useSettingsPermissionsPageState'

describe('useSettingsPermissionsPageState', () => {
  test('assembles permissions panel state from provided settings source', () => {
    const permissions = createSettingsPermissionsPanelState({
      permissionDraft: {
        subjectKind: 'tool',
        toolName: 'bash',
        qualifier: '',
        pathAccessKind: 'read',
        workspaceRoot: '',
        targetPath: '',
        scope: 'session',
        sessionId: '',
        mode: 'allow',
      },
      editPermissionRule: () => {},
      editingPermissionRuleId: ref(null),
      filteredPermissionRules: computed(() => []),
      permissionModeFilter: ref('all'),
      permissionRuleFacts: () => [],
      permissionRuleLabel: () => 'rule',
      permissionRulePreview: () => 'preview',
      permissionScopeFilter: ref('all'),
      permissionSearch: ref(''),
      permissionStatusFilter: ref('all'),
      permissionSubjectFilter: ref('all'),
      savePermissionRule: async () => {},
      resetPermissionDraft: () => {},
      revokePermissionRuleAction: async () => {},
      deletePermissionRuleAction: async () => {},
    })

    expect(permissions.draft.toolName).toBe('bash')
    expect(permissions.filteredRules.value).toEqual([])
    expect(permissions.ruleLabel({} as never)).toBe('rule')
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/settings/permissions' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useSettingsPermissionsPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'settings' })
          return {
            shared,
            state: {
              permissionDraft: {
                subjectKind: 'tool',
                toolName: 'bash',
                qualifier: '',
                pathAccessKind: 'read',
                workspaceRoot: '',
                targetPath: '',
                scope: 'session',
                sessionId: '',
                mode: 'allow',
              },
              editPermissionRule: () => {},
              editingPermissionRuleId: ref(null),
              filteredPermissionRules: computed(() => []),
              permissionModeFilter: ref('all'),
              permissionRuleFacts: () => [],
              permissionRuleLabel: () => 'rule',
              permissionRulePreview: () => 'preview',
              permissionScopeFilter: ref('all'),
              permissionSearch: ref(''),
              permissionStatusFilter: ref('all'),
              permissionSubjectFilter: ref('all'),
              savePermissionRule: async () => {},
              resetPermissionDraft: () => {},
              revokePermissionRuleAction: async () => {},
              deletePermissionRuleAction: async () => {},
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.permissions.filteredRules.value).toEqual([])
  })
})
