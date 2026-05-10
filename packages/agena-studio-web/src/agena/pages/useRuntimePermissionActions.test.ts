import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { PermissionRuleResource, SessionExecutionResource } from '../lib/agenaApi'
import type { SettingsTab } from './runtimePageStateModel'
import type { RuntimePermissionDraft } from './useRuntimePermissionActions'
import { useRuntimePermissionActions } from './useRuntimePermissionActions'

function createRule(overrides: Partial<PermissionRuleResource> = {}): PermissionRuleResource {
  return {
    id: 7,
    action_key: 'Bash:ls',
    subject_kind: 'builtin_tool',
    tool_name: 'bash',
    qualifier: 'ls *',
    path_access_kind: null,
    workspace_root: null,
    target_path: null,
    mode: 'ask',
    scope: 'workspace',
    source: 'api',
    created_at: '2026-05-10T00:00:00Z',
    updated_at: '2026-05-10T00:00:00Z',
    ...overrides,
  }
}

function createExecution(): SessionExecutionResource {
  return {
    session: {
      id: 12,
      workspace_id: 3,
      title: 'Demo',
      version: 1,
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      message_count: 1,
      child_session_count: 0,
    },
    blocked: false,
    run_state: 'idle',
    execution: {
      allowed_tools: [],
    },
    pending_permission_requests: [],
    pending_user_input_requests: [],
  }
}

function createState() {
  const calls: string[] = []
  const state = {
    actionError: ref(''),
    actionMessage: ref(''),
    activeSettingsTab: ref<SettingsTab>('providers'),
    editingPermissionRuleId: ref<number | null>(null),
    permissionDraft: {
      subjectKind: 'builtin_tool',
      toolName: 'bash',
      qualifier: 'git status *',
      pathAccessKind: 'read',
      workspaceRoot: '',
      targetPath: '',
      scope: 'workspace',
      sessionId: '',
      mode: 'ask',
    } as RuntimePermissionDraft,
    selectedSessionId: ref<number | null>(12),
    sessionExecution: ref<SessionExecutionResource | null>(null),
    load: async () => {
      calls.push('load')
    },
    loadSessionExecution: async (sessionId: number) => {
      calls.push(`loadSessionExecution:${sessionId}`)
    },
  }
  return { calls, state }
}

describe('useRuntimePermissionActions', () => {
  test('formats and edits permission rules', () => {
    const { state } = createState()
    const actions = useRuntimePermissionActions(state, {
      createPermissionRule: async () => createRule(),
      deletePermissionRule: async () => createRule(),
      replyPermission: async () => createExecution(),
      revokePermissionRule: async () => createRule(),
      updatePermissionRule: async () => createRule(),
    })
    const pathRule = createRule({
      id: 9,
      subject_kind: 'path_access',
      tool_name: null,
      qualifier: null,
      path_access_kind: 'write',
      workspace_root: '/repo',
      target_path: 'src/**',
      scope: 'session',
      session_id: 12,
      operator: 'claude',
      reason: 'needed',
      revoked_at: '2026-05-10T00:01:00Z',
    })

    expect(actions.permissionRuleLabel(pathRule)).toBe('write · src/**')
    expect(actions.permissionRulePreview(pathRule)).toBe('access=write · workspace=/repo · target=src/**')
    expect(actions.permissionRuleFacts(pathRule)).toEqual([
      'scope=session #12',
      'source=api',
      'status=revoked',
      'operator=claude',
      'reason=needed',
      'revoked_at=2026-05-10T00:01:00Z',
    ])

    actions.editPermissionRule(pathRule)

    expect(state.permissionDraft.subjectKind).toBe('path_access')
    expect(state.permissionDraft.targetPath).toBe('src/**')
    expect(state.permissionDraft.scope).toBe('session')
    expect(state.permissionDraft.sessionId).toBe('12')
    expect(state.activeSettingsTab.value).toBe('permissions')
    expect(state.editingPermissionRuleId.value).toBe(9)

    actions.resetPermissionDraft()

    expect(state.permissionDraft.subjectKind).toBe('builtin_tool')
    expect(state.permissionDraft.toolName).toBe('')
    expect(state.editingPermissionRuleId.value).toBe(null)
  })

  test('savePermissionRule creates and updates rules', async () => {
    const { calls, state } = createState()
    const apiCalls: string[] = []
    const actions = useRuntimePermissionActions(state, {
      createPermissionRule: async (input) => {
        apiCalls.push(`create:${input.subjectKind}:${input.toolName}:${input.mode}`)
        return createRule()
      },
      deletePermissionRule: async () => createRule(),
      replyPermission: async () => createExecution(),
      revokePermissionRule: async () => createRule(),
      updatePermissionRule: async (input) => {
        apiCalls.push(`update:${input.id}:${input.subjectKind}:${input.targetPath}:${input.mode}`)
        return createRule({ id: input.id })
      },
    })

    await actions.savePermissionRule()

    expect(apiCalls).toEqual(['create:builtin_tool:bash:ask'])
    expect(calls).toEqual(['load'])
    expect(state.actionMessage.value).toBe('Created permission rule for bash · git status *.')
    expect(state.editingPermissionRuleId.value).toBe(null)

    state.permissionDraft.subjectKind = 'path_access'
    state.permissionDraft.toolName = ''
    state.permissionDraft.qualifier = ''
    state.permissionDraft.pathAccessKind = 'write'
    state.permissionDraft.workspaceRoot = '/repo'
    state.permissionDraft.targetPath = 'src/**'
    state.permissionDraft.scope = 'session'
    state.permissionDraft.sessionId = '42'
    state.permissionDraft.mode = 'allow'
    state.editingPermissionRuleId.value = 11
    calls.length = 0

    await actions.savePermissionRule()

    expect(apiCalls).toEqual([
      'create:builtin_tool:bash:ask',
      'update:11:path_access:src/**:allow',
    ])
    expect(calls).toEqual(['load'])
    expect(state.actionMessage.value).toBe('Updated permission rule for write · src/**.')
  })

  test('revokePermissionRuleAction and approvePermission refresh state', async () => {
    const { calls, state } = createState()
    const actions = useRuntimePermissionActions(state, {
      createPermissionRule: async () => createRule(),
      deletePermissionRule: async (id) => {
        calls.push(`deletePermissionRule:${id}`)
        return createRule({ id })
      },
      replyPermission: async ({ kind }) => {
        calls.push(`replyPermission:${kind}`)
        return createExecution()
      },
      revokePermissionRule: async (id) => {
        calls.push(`revokePermissionRule:${id}`)
        return createRule({ id })
      },
      updatePermissionRule: async () => createRule(),
    })
    const rule = createRule({ id: 7 })

    state.editingPermissionRuleId.value = 7
    await actions.revokePermissionRuleAction(rule)

    expect(calls).toEqual(['revokePermissionRule:7', 'load'])
    expect(state.actionMessage.value).toBe('Revoked permission rule for bash · ls *.')
    expect(state.editingPermissionRuleId.value === null).toBe(true)

    calls.length = 0
    state.editingPermissionRuleId.value = 7
    await actions.deletePermissionRuleAction(rule)

    expect(calls).toEqual(['deletePermissionRule:7', 'load'])
    expect(state.actionMessage.value).toBe('Deleted permission rule for bash · ls *.')
    expect(state.editingPermissionRuleId.value === null).toBe(true)

    calls.length = 0
    await actions.approvePermission('req-1', 'allow_once', 'workspace')

    expect(calls).toEqual(['replyPermission:allow_once', 'loadSessionExecution:12'])
    expect(state.actionMessage.value).toBe('Sent permission reply: allow once.')
    expect(state.sessionExecution.value?.session.id).toBe(12)
  })
})
