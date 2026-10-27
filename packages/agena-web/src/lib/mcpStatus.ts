export type McpStatusResponse = { connections?: unknown[] }
export type McpStatusItem = {
  id: string
  name: string
  scope: string
  status: string
  toolCount: number
  error: string | null
}

export function normalizeMcpStatus(payload: McpStatusResponse | null | undefined): McpStatusItem[] {
  if (!Array.isArray(payload?.connections)) return []
  return payload.connections.flatMap((entry): McpStatusItem[] => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return []
    const record = entry as Record<string, unknown>
    const id = typeof record.id === 'string' ? record.id : ''
    const name = typeof record.name === 'string' ? record.name.trim() : ''
    if (!id || !name) return []
    return [{
      id,
      name,
      scope: typeof record.scope === 'string' ? record.scope : '',
      status: typeof record.state === 'string' && record.state ? record.state : 'disconnected',
      toolCount: typeof record.tool_count === 'number' && Number.isFinite(record.tool_count) ? Math.max(0, Math.floor(record.tool_count)) : 0,
      error: typeof record.error === 'string' ? record.error : null,
    }]
  }).sort((left, right) => left.name.localeCompare(right.name) || left.scope.localeCompare(right.scope))
}
