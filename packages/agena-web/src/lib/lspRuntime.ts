export type LspRuntimeListResponse = {
  operator?: {
    lsp?: {
      servers?: unknown[]
    }
  }
}

export type LspRuntimeItem = {
  id: string
  name: string
  status: 'configured'
  transport: string
  fileExtensions: string[]
  rootMarkers: string[]
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, unknown>
}

function readString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function readStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.map(readString).filter(Boolean)
}

export function normalizeLspRuntimeList(payload: LspRuntimeListResponse): LspRuntimeItem[] {
  const servers = payload.operator?.lsp?.servers
  if (!Array.isArray(servers)) return []

  return servers
    .map((raw) => {
      const record = asRecord(raw)
      if (!record) return null
      const name = readString(record.name)
      const command = readString(record.command)
      if (!name || !command) return null
      return {
        id: name,
        name,
        status: 'configured' as const,
        transport: command,
        fileExtensions: readStringArray(record.file_extensions),
        rootMarkers: readStringArray(record.root_markers),
      }
    })
    .filter((item): item is LspRuntimeItem => item !== null)
    .sort((left, right) => left.name.localeCompare(right.name))
}
