export type PermissionRuleScope = 'effective' | 'global' | 'workspace' | 'session'

export type PermissionRuleScopeRecord = {
  scope: string
  workspace_id?: number | null
  session_id?: number | null
  revoked_at?: string | null
}

export function permissionRuleMatchesScope(
  rule: PermissionRuleScopeRecord,
  scope: PermissionRuleScope,
  workspaceId: number | null,
  sessionId: number | null,
): boolean {
  if (scope === 'global') return rule.scope === 'global'
  if (scope === 'workspace') {
    return workspaceId !== null && rule.scope === 'workspace' && Number(rule.workspace_id) === workspaceId
  }
  if (scope === 'session') {
    return sessionId !== null && rule.scope === 'session' && Number(rule.session_id) === sessionId
  }
  if (rule.revoked_at) return false
  return (
    rule.scope === 'global' ||
    (workspaceId !== null && rule.scope === 'workspace' && Number(rule.workspace_id) === workspaceId) ||
    (sessionId !== null && rule.scope === 'session' && Number(rule.session_id) === sessionId)
  )
}
