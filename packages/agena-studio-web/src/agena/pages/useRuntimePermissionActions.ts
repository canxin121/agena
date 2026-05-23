import type { Ref } from 'vue'

import {
  createPermissionRule,
  deletePermissionRule,
  replyPermission,
  revokePermissionRule,
  updatePermissionRule,
  type PermissionRequest,
  type PermissionMode,
  type PermissionRuleResource,
  type PermissionSubjectKind,
  type SessionExecutionResource,
} from '../lib/agenaApi'

export type RuntimePermissionDraft = {
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

export type RuntimePermissionActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  activeSettingsTab: Ref<'providers' | 'agents' | 'plugins' | 'permissions' | 'desktop'>
  editingPermissionRuleId: Ref<number | null>
  interactiveRequestInFlight: Record<string, boolean>
  load: () => Promise<void>
  loadSessionExecution: (sessionId: number) => Promise<void>
  permissionDraft: RuntimePermissionDraft
  selectedSessionId: Ref<number | null>
  sessionExecution: Ref<SessionExecutionResource | null>
}

export type RuntimePermissionActionsDeps = {
  createPermissionRule: typeof createPermissionRule
  deletePermissionRule: typeof deletePermissionRule
  replyPermission: typeof replyPermission
  revokePermissionRule: typeof revokePermissionRule
  updatePermissionRule: typeof updatePermissionRule
}

const defaultDeps: RuntimePermissionActionsDeps = {
  createPermissionRule,
  deletePermissionRule,
  replyPermission,
  revokePermissionRule,
  updatePermissionRule,
}

function buildPermissionRuleStub(id: number, draft: RuntimePermissionDraft): PermissionRuleResource {
  return {
    id,
    action_key: '',
    subject_kind: draft.subjectKind,
    tool_name: draft.subjectKind === 'tool' ? draft.toolName.trim() || null : null,
    qualifier: draft.subjectKind === 'tool' ? draft.qualifier.trim() || null : null,
    path_access_kind: draft.subjectKind === 'path_access' ? draft.pathAccessKind || null : null,
    workspace_root: draft.subjectKind === 'path_access' ? draft.workspaceRoot.trim() || null : null,
    target_path: draft.subjectKind === 'path_access' ? draft.targetPath.trim() || null : null,
    network_target: draft.subjectKind === 'network_access' ? draft.networkTarget.trim() || null : null,
    network_host: null,
    network_port:
      draft.subjectKind === 'network_access' && draft.networkPort.trim() ? Number(draft.networkPort.trim()) : null,
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
  function interactiveRequestKey(sessionId: number, requestId: string): string {
    return `${sessionId}:${requestId}`
  }

  function beginInteractiveRequest(sessionId: number, requestId: string): boolean {
    const key = interactiveRequestKey(sessionId, requestId)
    if (input.interactiveRequestInFlight[key]) return false
    input.interactiveRequestInFlight[key] = true
    return true
  }

  function finishInteractiveRequest(sessionId: number, requestId: string) {
    delete input.interactiveRequestInFlight[interactiveRequestKey(sessionId, requestId)]
  }

  function isInteractiveRequestBusy(requestId: string): boolean {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return false
    return !!input.interactiveRequestInFlight[interactiveRequestKey(sessionId, requestId)]
  }

  function resetPermissionDraft() {
    input.permissionDraft.subjectKind = 'tool'
    input.permissionDraft.toolName = ''
    input.permissionDraft.qualifier = ''
    input.permissionDraft.pathAccessKind = 'read'
    input.permissionDraft.workspaceRoot = ''
    input.permissionDraft.targetPath = ''
    input.permissionDraft.networkTarget = ''
    input.permissionDraft.networkPort = ''
    input.permissionDraft.scope = 'workspace'
    input.permissionDraft.sessionId = ''
    input.permissionDraft.mode = 'ask'
    input.editingPermissionRuleId.value = null
  }

  function permissionRuleLabel(rule: PermissionRuleResource): string {
    if (rule.subject_kind === 'tool') {
      return rule.qualifier?.trim() ? `${rule.tool_name} · ${rule.qualifier}` : rule.tool_name || rule.action_key
    }
    if (rule.subject_kind === 'path_access') {
      return `${rule.path_access_kind || 'path'} · ${rule.target_path || rule.action_key}`
    }
    if (rule.subject_kind === 'network_access') {
      const target = rule.network_target || rule.network_host || rule.action_key
      return rule.network_port == null ? `network · ${target}` : `network · ${target}:${rule.network_port}`
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
    if (rule.subject_kind === 'tool') {
      const qualifier = rule.qualifier?.trim()
      return qualifier ? `tool=${rule.tool_name} · qualifier=${qualifier}` : `tool=${rule.tool_name}`
    }
    if (rule.subject_kind === 'network_access') {
      return [
        `target=${rule.network_target || rule.network_host || 'network'}`,
        rule.network_port == null ? null : `port=${rule.network_port}`,
      ]
        .filter(Boolean)
        .join(' · ')
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
    input.permissionDraft.subjectKind =
      rule.subject_kind === 'path_access'
        ? 'path_access'
        : rule.subject_kind === 'network_access'
          ? 'network_access'
          : 'tool'
    input.permissionDraft.toolName = rule.tool_name || ''
    input.permissionDraft.qualifier = rule.qualifier || ''
    input.permissionDraft.pathAccessKind = rule.path_access_kind || 'read'
    input.permissionDraft.workspaceRoot = rule.workspace_root || ''
    input.permissionDraft.targetPath = rule.target_path || ''
    input.permissionDraft.networkTarget = rule.network_target || rule.network_host || ''
    input.permissionDraft.networkPort = rule.network_port == null ? '' : String(rule.network_port)
    input.permissionDraft.scope =
      rule.scope === 'session' ? 'session' : rule.scope === 'global' ? 'global' : 'workspace'
    input.permissionDraft.sessionId = rule.session_id == null ? '' : String(rule.session_id)
    input.permissionDraft.mode = rule.mode
    input.editingPermissionRuleId.value = rule.id
    input.activeSettingsTab.value = 'permissions'
  }

  function editPermissionRequest(request: PermissionRequest) {
    const action = request.action || {}
    const kind = typeof action.kind === 'string' ? action.kind : ''
    input.permissionDraft.subjectKind =
      kind === 'path_access' ? 'path_access' : kind === 'network_access' ? 'network_access' : 'tool'
    input.permissionDraft.toolName = typeof action.tool_name === 'string' ? action.tool_name : ''
    input.permissionDraft.qualifier = typeof action.qualifier === 'string' ? action.qualifier : ''
    input.permissionDraft.pathAccessKind =
      typeof action.access_kind === 'string'
        ? action.access_kind
        : typeof action.path_access_kind === 'string'
          ? action.path_access_kind
          : 'read'
    input.permissionDraft.workspaceRoot = typeof action.workspace_root === 'string' ? action.workspace_root : ''
    input.permissionDraft.targetPath = typeof action.target_path === 'string' ? action.target_path : ''
    input.permissionDraft.networkTarget =
      typeof action.target === 'string'
        ? action.target
        : typeof action.network_target === 'string'
          ? action.network_target
          : typeof action.host === 'string'
            ? action.host
            : ''
    const networkPort =
      typeof action.port === 'number' ? action.port : typeof action.port === 'string' ? Number(action.port) : undefined
    input.permissionDraft.networkPort =
      networkPort === undefined || !Number.isFinite(networkPort) ? '' : String(networkPort)
    input.permissionDraft.scope =
      request.scope === 'session'
        ? 'session'
        : request.scope === 'global'
          ? 'global'
          : request.session_id != null
            ? 'session'
            : 'workspace'
    input.permissionDraft.sessionId =
      input.permissionDraft.scope === 'session' && request.session_id != null ? String(request.session_id) : ''
    input.permissionDraft.mode = 'allow'
    input.editingPermissionRuleId.value = null
    input.activeSettingsTab.value = 'permissions'
  }

  async function savePermissionRule() {
    const toolName = input.permissionDraft.toolName.trim()
    const qualifier = input.permissionDraft.qualifier.trim()
    const targetPath = input.permissionDraft.targetPath.trim()
    const networkTarget = input.permissionDraft.networkTarget.trim()
    const networkPortText = input.permissionDraft.networkPort.trim()
    if (input.permissionDraft.subjectKind === 'tool' && !toolName) return
    if (input.permissionDraft.subjectKind === 'path_access' && !targetPath) return
    if (input.permissionDraft.subjectKind === 'network_access' && !networkTarget) return

    const networkPort =
      input.permissionDraft.subjectKind === 'network_access' && networkPortText ? Number(networkPortText) : undefined
    if (networkPort !== undefined && (!Number.isFinite(networkPort) || networkPort < 0 || networkPort > 65535)) return

    const payload = {
      subjectKind: input.permissionDraft.subjectKind,
      toolName: input.permissionDraft.subjectKind === 'tool' ? toolName : undefined,
      qualifier: input.permissionDraft.subjectKind === 'tool' && qualifier ? qualifier : undefined,
      pathAccessKind:
        input.permissionDraft.subjectKind === 'path_access' ? input.permissionDraft.pathAccessKind : undefined,
      workspaceRoot:
        input.permissionDraft.subjectKind === 'path_access' && input.permissionDraft.workspaceRoot.trim()
          ? input.permissionDraft.workspaceRoot.trim()
          : undefined,
      targetPath: input.permissionDraft.subjectKind === 'path_access' ? targetPath : undefined,
      networkTarget: input.permissionDraft.subjectKind === 'network_access' ? networkTarget : undefined,
      networkPort,
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

  async function deletePermissionRuleAction(rule: PermissionRuleResource) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      await deps.deletePermissionRule(rule.id)
      input.actionMessage.value = `Deleted permission rule for ${permissionRuleLabel(rule)}.`
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
    if (!beginInteractiveRequest(sessionId, requestId)) return
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
    } finally {
      finishInteractiveRequest(sessionId, requestId)
    }
  }

  return {
    approvePermission,
    editPermissionRequest,
    editPermissionRule,
    deletePermissionRuleAction,
    isInteractiveRequestBusy,
    permissionRuleFacts,
    permissionRuleLabel,
    permissionRulePreview,
    resetPermissionDraft,
    revokePermissionRuleAction,
    savePermissionRule,
  }
}
