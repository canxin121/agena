import type { McpConfig, McpConnection, McpConnectionDetail } from './mcpConnectionsApi'

export const MCP_REDACTED = '[REDACTED]'
export type McpDraft = { config: McpConfig; preserveSecrets: string[] }
export type McpPendingAuth = { flow_id: string; connection_id: string; state: string; expires_at: string }
export const MCP_AUTH_STORAGE_KEY = 'agena.mcp.oauth.pending'

export function mcpRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}
export function cloneMcpConfig<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}
export function mcpPointerKey(value: string): string {
  return value.replace(/~/g, '~0').replace(/\//g, '~1')
}
export function mcpValueAt(config: unknown, pointer: string): unknown {
  let current = config
  for (const part of pointer.split('/').slice(1).map((part) => part.replace(/~1/g, '/').replace(/~0/g, '~'))) {
    const record = mcpRecord(current)
    if (!Object.hasOwn(record, part)) return undefined
    current = record[part]
  }
  return current
}
export function changeMcpValue(draft: McpDraft, pointer: string, value: unknown, remove = false): McpDraft {
  const config = cloneMcpConfig(draft.config)
  const parts = pointer.split('/').slice(1).map((part) => part.replace(/~1/g, '/').replace(/~0/g, '~'))
  if (!parts.length) return draft
  let current = config
  for (const part of parts.slice(0, -1)) {
    let child = current[part]
    if (!Object.hasOwn(current, part) || child === null || typeof child !== 'object' || Array.isArray(child)) {
      child = {}
      Object.defineProperty(current, part, { value: child, enumerable: true, configurable: true, writable: true })
    }
    current = child as Record<string, unknown>
  }
  const last = parts.at(-1)!
  if (remove) delete current[last]
  else Object.defineProperty(current, last, { value, enumerable: true, configurable: true, writable: true })
  return {
    config,
    preserveSecrets: draft.preserveSecrets.filter((secret) => secret !== pointer && !secret.startsWith(pointer + '/')),
  }
}
export function preservedMcpSecrets(draft: McpDraft): string[] {
  return draft.preserveSecrets.filter((pointer) => mcpValueAt(draft.config, pointer) === MCP_REDACTED)
}
export function mcpDraftFromDetail(detail: McpConnectionDetail): McpDraft {
  const draft = { config: cloneMcpConfig(detail.config), preserveSecrets: [...detail.secret_paths] }
  draft.preserveSecrets = preservedMcpSecrets(draft)
  return draft
}
export function newMcpConfig(transport: 'stdio' | 'http' = 'stdio'): McpConfig {
  return {
    transport,
    ...(transport === 'stdio' ? { process: { command: '', args: [], env: {} } } : { endpoint: { url: '', headers: {} }, auth: null }),
    enabled: true,
    timeouts: { connect_ms: 20000, request_ms: 60000 },
    tools: { include: [], exclude: [] },
  }
}
export function mcpEndpoint(detail: Pick<McpConnectionDetail, 'config'>): string {
  const config = detail.config
  if (config.transport === 'stdio') return String(mcpRecord(config.process).command || '')
  try {
    const url = new URL(String(mcpRecord(config.endpoint).url || ''))
    return url.origin + url.pathname
  } catch {
    return ''
  }
}
export function filterMcpConnections(connections: McpConnection[], query: string, scope: string, status: string): McpConnection[] {
  const needle = query.trim().toLocaleLowerCase()
  return connections.filter((connection) =>
    (!needle || [connection.name, connection.transport, connection.scope, connection.state].some((part) => part.toLocaleLowerCase().includes(needle))) &&
    (scope === 'all' || connection.scope === scope) &&
    (status === 'all' || connection.state === status || (status === 'attention' && !['connected', 'disabled', 'shadowed'].includes(connection.state)))
  )
}
export function mcpConfigProblems(draft: McpDraft, creating = false): string[] {
  const config = draft.config
  const problems: string[] = []
  const preserved = new Set(creating ? [] : preservedMcpSecrets(draft))
  if (!['stdio', 'http'].includes(String(config.transport))) problems.push('Choose a supported MCP transport.')
  if (config.transport === 'stdio') {
    const process = mcpRecord(config.process)
    if (typeof process.command !== 'string' || !process.command.trim()) problems.push('Enter a command for the stdio connection.')
    if (process.args !== undefined && (!Array.isArray(process.args) || process.args.some((arg) => typeof arg !== 'string'))) problems.push('Arguments must be an array of strings.')
  }
  if (config.transport === 'http') {
    const endpoint = mcpRecord(config.endpoint).url
    if (endpoint !== MCP_REDACTED || !preserved.has('/endpoint/url')) {
      try {
        const url = new URL(String(endpoint || ''))
        if (!['http:', 'https:'].includes(url.protocol)) throw new Error()
      } catch { problems.push('Enter an HTTP or HTTPS MCP endpoint.') }
    }
    const auth = mcpRecord(config.auth)
    if (auth.kind === 'bearer' && (typeof auth.token !== 'string' || !auth.token)) problems.push('Enter a bearer token or select another authentication method.')
    if (auth.kind === 'bearer_from_env' && (typeof auth.env !== 'string' || !auth.env.trim())) problems.push('Enter the environment variable containing the bearer token.')
  }
  for (const pointer of ['/process/env', '/endpoint/headers', '/auth/headers']) {
    const value = mcpValueAt(config, pointer)
    if (value !== undefined && (value === null || typeof value !== 'object' || Array.isArray(value) || Object.values(value).some((entry) => typeof entry !== 'string'))) problems.push('Environment variables and headers must have string values.')
  }
  const timeouts = mcpRecord(config.timeouts)
  if (timeouts.connect_ms !== undefined && (!Number.isInteger(timeouts.connect_ms) || Number(timeouts.connect_ms) < 1 || Number(timeouts.connect_ms) > 300000)) problems.push('Connect timeout must be between 1 and 300000 milliseconds.')
  if (timeouts.request_ms !== undefined && (!Number.isInteger(timeouts.request_ms) || Number(timeouts.request_ms) < 1 || Number(timeouts.request_ms) > 3600000)) problems.push('Request timeout must be between 1 and 3600000 milliseconds.')
  function visit(value: unknown, path: string) {
    if (value === MCP_REDACTED && !preserved.has(path)) problems.push('Replace or remove redacted values that cannot be preserved.')
    else if (value && typeof value === 'object') for (const [key, child] of Object.entries(value)) visit(child, path + '/' + mcpPointerKey(key))
  }
  visit(config, '')
  return [...new Set(problems)]
}
export function parseMcpImport(text: string): Record<string, unknown> {
  const value: unknown = JSON.parse(text)
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Enter a valid JSON object.')
  return value as Record<string, unknown>
}
export function readMcpPendingAuth(raw: string | null): McpPendingAuth | null {
  try {
    const value = JSON.parse(raw || 'null') as McpPendingAuth | null
    if (!value || !['flow_id', 'connection_id', 'state', 'expires_at'].every((key) => typeof value[key as keyof McpPendingAuth] === 'string' && value[key as keyof McpPendingAuth].length > 0)) return null
    if (!Number.isFinite(Date.parse(value.expires_at))) return null
    return { flow_id: value.flow_id, connection_id: value.connection_id, state: value.state, expires_at: value.expires_at }
  } catch { return null }
}
export function mcpCallbackMatches(flow: McpPendingAuth, input: string): boolean {
  try {
    const url = new URL(input)
    return ['http:', 'https:'].includes(url.protocol) && url.searchParams.get('state') === flow.state && Boolean(url.searchParams.get('code')) && !url.searchParams.has('error')
  } catch { return false }
}
