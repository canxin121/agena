export type PermissionActionView = {
  title: string
  details: string[]
}

export type PermissionExplainability = {
  summary: string | null
  details: string[]
}

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

export function permissionRiskLabel(action: Record<string, unknown>): string {
  const kind = readString(action.kind)
  if (kind === 'builtin_tool') {
    const toolName = readString(action.tool_name) || ''
    return toolName === 'bash' ? 'mutable tool execution' : 'tool access'
  }
  if (kind === 'path_access') {
    const accessKind = readString(action.access_kind) || ''
    if (accessKind === 'write') return 'workspace write'
    if (accessKind === 'external_directory') return 'external directory access'
    return 'workspace read'
  }
  return 'permission request'
}

export function permissionActionView(action: Record<string, unknown>): PermissionActionView {
  const kind = readString(action.kind)
  if (kind === 'builtin_tool') {
    const toolName = readString(action.tool_name) || 'tool'
    const qualifier = readString(action.qualifier)
    return {
      title: qualifier ? `${toolName} · ${qualifier}` : toolName,
      details: [
        'kind=builtin_tool',
        `tool=${toolName}`,
        ...(qualifier ? [`qualifier=${qualifier}`] : []),
      ],
    }
  }
  if (kind === 'path_access') {
    const accessKind = readString(action.access_kind) || 'path_access'
    const workspaceRoot = readString(action.workspace_root)
    const targetPath = readString(action.target_path)
    return {
      title: `${accessKind} · ${targetPath || 'path'}`,
      details: [
        'kind=path_access',
        `access=${accessKind}`,
        ...(workspaceRoot ? [`workspace=${workspaceRoot}`] : []),
        ...(targetPath ? [`target=${targetPath}`] : []),
      ],
    }
  }
  return {
    title: kind || 'permission_request',
    details: Object.entries(action).map(([key, value]) => `${key}=${typeof value === 'string' ? value : JSON.stringify(value)}`),
  }
}

export function permissionReplyPreview(scope?: 'session' | 'workspace' | 'global'): string {
  if (scope === 'session') return 'This remembers the decision only for the current session.'
  if (scope === 'workspace') return 'This remembers the decision for new sessions in the same workspace.'
  if (scope === 'global') return 'This remembers the decision across every workspace in this runtime.'
  return 'This applies only to the current request.'
}

export function permissionExplainability(input: {
  source?: string | null
  scope?: 'session' | 'workspace' | 'global' | null
  operator?: string | null
}): PermissionExplainability {
  const details: string[] = []
  if (input.source) details.push(`source=${input.source}`)
  if (input.scope) details.push(`scope=${input.scope}`)
  if (input.operator) details.push(`operator=${input.operator}`)
  if (details.length === 0) return { summary: null, details }

  const fragments: string[] = []
  if (input.source === 'permission_reply') {
    fragments.push('Matched a remembered permission reply')
  } else if (input.source === 'api') {
    fragments.push('Managed by the HTTP API')
  } else if (input.source === 'static_policy') {
    fragments.push('Matched the static permission policy')
  } else if (input.source) {
    fragments.push(`Source: ${input.source}`)
  }
  if (input.scope) {
    fragments.push(`scope=${input.scope}`)
  }
  if (input.operator) {
    fragments.push(`operator=${input.operator}`)
  }
  return {
    summary: fragments.join(' · '),
    details,
  }
}
