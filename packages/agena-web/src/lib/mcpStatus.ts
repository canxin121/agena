export type McpStatusResponse = {
  operator?: {
    mcp?: {
      servers?: unknown[]
    }
  }
}

export type McpStatusItem = {
  name: string
  status: 'running'
  toolCount: number
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, unknown>
}

export function normalizeMcpStatus(payload: McpStatusResponse | null | undefined): McpStatusItem[] {
  const servers = payload?.operator?.mcp?.servers
  if (!Array.isArray(servers)) return []

  return servers
    .map((entry) => {
      const record = asRecord(entry)
      if (!record) return null
      const name = typeof record.name === 'string' ? record.name.trim() : ''
      const toolCount =
        typeof record.tool_count === 'number' && Number.isFinite(record.tool_count)
          ? Math.max(0, Math.floor(record.tool_count))
          : null
      if (!name || toolCount === null) return null
      return {
        name,
        status: 'running' as const,
        toolCount,
      }
    })
    .filter((item): item is McpStatusItem => item !== null)
    .sort((left, right) => left.name.localeCompare(right.name))
}
