import type { Ref } from 'vue'

import {
  createPermissionRule,
  replyPermission,
  revokePermissionRule,
  updatePermissionRule,
  type PermissionMode,
  type PermissionRuleResource,
  type SessionExecutionResource,
} from '../lib/agenaApi'

export type RuntimePermissionDraft = {
  subjectKind: 'builtin_tool' | 'path_access'
  toolName: string
  qualifier: string
  pathAccessKind: string
  workspaceRoot: string
  targetPath: string
  scope: 'session' | 'workspace' | 'global'
  sessionId: string
  mode: PermissionMode
}

export type RuntimePermissionActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  activeSettingsTab: Ref<'providers' | 'permissions' | 'desktop'>
  editingPermissionRuleId: Ref<number | null>
  load: () => Promise<void>
  loadSessionExecution: (sessionId: number) => Promise<void>
  permissionDraft: RuntimePermissionDraft
  selectedSessionId: Ref<number | null>
  sessionExecution: Ref<SessionExecutionResource | null>
}

export type RuntimePermissionActionsDeps = {
  createPermissionRule: typeof createPermissionRule
  replyPermission: typeof replyPermission
  revokePermissionRule: typeof revokePermissionRule
  updatePermissionRule: typeof updatePermissionRule
}

const defaultDeps: RuntimePermissionActionsDeps = {
  createPermissionRule,
  replyPermission,
  revokePermissionRule,
  updatePermissionRule,
}

function buildPermissionRuleStub(
  id: number,
  draft: RuntimePermissionDraft,
): PermissionRuleResource {
  return {
    id,
    action_key: '',
    subject_kind: draft.subjectKind,
    tool_name: draft.subjectKind === 'builtin_tool' ? draft.toolName.trim() || null : null,
    qualifier: draft.subjectKind === 'builtin_tool' ? draft.qualifier.trim() || null : null,
    path_access_kind: draft.subjectKind === 'path_access' ? draft.pathAccessKind || null : null,
    workspace_root: draft.subjectKind === 'path_access' ? draft.workspaceRoot.trim() || null : null,
    target_path: draft.subjectKind === 'path_access' ? draft.targetPath.trim() || null : null,
    mode: draft.mode,
    scope: draft.scope,
    source: 'api',
    created_at: '',
    updated_at: '',
  }
}

export function useRuntimePermissionActions(
  input: RuntimePermissionActionsInput,
  deps: RuntimePermissionActionsDeps = defaultDeps,
) {
  function resetPermissionDraft() {
    input.permissionDraft.subjectKind = 'builtin_tool'
    input.permissionDraft.toolName = ''
    input.permissionDraft.qualifier = ''
    input.permissionDraft.pathAccessKind = 'read'
    input.permissionDraft.workspaceRoot = ''
    input.permissionDraft.targetPath = ''
    input.permissionDraft.scope = 'workspace'
    input.permissionDraft.sessionId = ''
    input.permissionDraft.mode = 'ask'
    input.editingPermissionRuleId.value = null
  }

  function permissionRuleLabel(rule: PermissionRuleResource): string {
    if (rule.subject_kind === 'builtin_tool') {
      return rule.qualifier?.trim() ? `${rule.tool_name} · ${rule.qualifier}` : rule.tool_name || rule.action_key
    }
    if (rule.subject_kind === 'path_access') {
      return `${rule.path_access_kind || 'path'} · ${rule.target_path || rule.action_key}`
    }
    return rule.action_key
  }

  function permissionRuleScopeLabel(rule: PermissionRuleResource): string {
    if (rule.scope === 'session') {
      return rule.session_id == null ? 'session' : `session #${rule.session_id}`
    }
    if (rule.scope === 'workspace') {
      return rule.workspace_id == null ? 'workspace' : `workspace #${rule.workspace_id}`
    }
    if (rule.scope === 'global') {
      return 'global'
    }
    return rule.scope
  }

  function permissionRuleFacts(rule: PermissionRuleResource): string[] {
    const facts = [
      `scope=${permissionRuleScopeLabel(rule)}`,
      `source=${rule.source}`,
      `status=${rule.revoked_at ? 'revoked' : 'active'}`,
    ]
    if (rule.operator) facts.push(`operator=${rule.operator}`)
    if (rule.reason) facts.push(`reason=${rule.reason}`)
    if (rule.revoked_at) facts.push(`revoked_at=${rule.revoked_at}`)
    if (rule.revoked_reason) facts.push(`revoked_reason=${rule.revoked_reason}`)
    if (rule.revoked_by) facts.push(`revoked_by=${rule.revoked_by}`)
    return facts
  }

  function permissionRulePreview(rule: PermissionRuleResource): string {
    if (rule.subject_kind === 'builtin_tool') {
      const qualifier = rule.qualifier?.trim()
      return qualifier ? `tool=${rule.tool_name} · qualifier=${qualifier}` : `tool=${rule.tool_name}`
    }
    return [
      `access=${rule.path_access_kind || 'path_access'}`,
      rule.workspace_root ? `workspace=${rule.workspace_root}` : null,
      rule.target_path ? `target=${rule.target_path}` : null,
    ]
      .filter(Boolean)
      .join(' · ')
  }

  function editPermissionRule(rule: PermissionRuleResource) {
    input.permissionDraft.subjectKind = rule.subject_kind === 'path_access' ? 'path_access' : 'builtin_tool'
    input.permissionDraft.toolName = rule.tool_name || ''
    input.permissionDraft.qualifier = rule.qualifier || ''
    input.permissionDraft.pathAccessKind = rule.path_access_kind || 'read'
    input.permissionDraft.workspaceRoot = rule.workspace_root || ''
    input.permissionDraft.targetPath = rule.target_path || ''
    input.permissionDraft.scope = rule.scope === 'session' ? 'session' : rule.scope === 'global' ? 'global' : 'workspace'
    input.permissionDraft.sessionId = rule.session_id == null ? '' : String(rule.session_id)
    input.permissionDraft.mode = rule.mode
    input.editingPermissionRuleId.value = rule.id
    input.activeSettingsTab.value = 'permissions'
  }

  async function savePermissionRule() {
    const toolName = input.permissionDraft.toolName.trim()
    const qualifier = input.permissionDraft.qualifier.trim()
    const targetPath = input.permissionDraft.targetPath.trim()
    if (input.permissionDraft.subjectKind === 'builtin_tool' && !toolName) return
    if (input.permissionDraft.subjectKind === 'path_access' && !targetPath) return

    const payload = {
      subjectKind: input.permissionDraft.subjectKind,
      toolName: input.permissionDraft.subjectKind === 'builtin_tool' ? toolName : undefined,
      qualifier: input.permissionDraft.subjectKind === 'builtin_tool' && qualifier ? qualifier : undefined,
      pathAccessKind: input.permissionDraft.subjectKind === 'path_access' ? input.permissionDraft.pathAccessKind : undefined,
      workspaceRoot:
        input.permissionDraft.subjectKind === 'path_access' && input.permissionDraft.workspaceRoot.trim()
          ? input.permissionDraft.workspaceRoot.trim()
          : undefined,
      targetPath: input.permissionDraft.subjectKind === 'path_access' ? targetPath : undefined,
      scope: input.permissionDraft.scope,
      sessionId:
        input.permissionDraft.scope === 'session' && input.permissionDraft.sessionId.trim()
          ? Number(input.permissionDraft.sessionId.trim())
          : undefined,
      mode: input.permissionDraft.mode,
    } as const

    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      if (input.editingPermissionRuleId.value) {
        await deps.updatePermissionRule({
          id: input.editingPermissionRuleId.value,
          ...payload,
        })
        input.actionMessage.value = `Updated permission rule for ${permissionRuleLabel(buildPermissionRuleStub(input.editingPermissionRuleId.value, input.permissionDraft))}.`
      } else {
        await deps.createPermissionRule(payload)
        input.actionMessage.value = `Created permission rule for ${permissionRuleLabel(buildPermissionRuleStub(0, input.permissionDraft))}.`
      }
      resetPermissionDraft()
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function revokePermissionRuleAction(rule: PermissionRuleResource) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      await deps.revokePermissionRule(rule.id)
      input.actionMessage.value = `Revoked permission rule for ${permissionRuleLabel(rule)}.`
      if (input.editingPermissionRuleId.value === rule.id) resetPermissionDraft()
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function approvePermission(
    requestId: string,
    kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
    scope?: 'session' | 'workspace' | 'global',
  ) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      input.sessionExecution.value = await deps.replyPermission({
        sessionId,
        requestId,
        kind,
        scope,
      })
      input.actionMessage.value = `Sent permission reply: ${kind.replaceAll('_', ' ')}.`
      await input.loadSessionExecution(sessionId)
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  return {
    approvePermission,
    editPermissionRule,
    permissionRuleFacts,
    permissionRuleLabel,
    permissionRulePreview,
    resetPermissionDraft,
    revokePermissionRuleAction,
    savePermissionRule,
  }
}
