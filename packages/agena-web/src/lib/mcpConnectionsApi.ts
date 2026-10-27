import { apiJson } from './api'

export type McpScope = 'global' | 'workspace'
export type McpConfig = Record<string, unknown>
export type McpConnection = {
  id: string
  name: string
  scope: McpScope
  transport: string
  enabled: boolean
  effective: boolean
  state: string
  tool_count: number
  revision: string
  error: string | null
}
export type McpConnectionDetail = McpConnection & {
  config: McpConfig
  secret_paths: string[]
  protocol_version: string | null
  server_info: Record<string, unknown> | null
  capabilities: Record<string, unknown> | null
  oauth_health: Record<string, unknown> | null
}
export type McpCatalog = {
  connections: McpConnection[]
  workspace_root: string
  global_path: string
  workspace_path: string
  bridge_enabled: boolean
}
export type McpMutationResult = {
  saved: boolean
  connection: McpConnectionDetail | null
  warning: string | null
  reload_task_id: string | null
}
export type McpTestResult = {
  ok: boolean
  elapsed_ms: number
  tool_count: number
  protocol_version: string | null
  server_info: Record<string, unknown> | null
  capabilities: Record<string, unknown> | null
  error: string | null
}
export type McpCapabilityKind = 'tools' | 'resources' | 'resource_templates' | 'prompts'
export type McpCapabilityPage = {
  kind: string
  items: Record<string, unknown>[]
  next_cursor: string | null
}
export type McpImportPreview = {
  revision: string
  candidates: Array<{
    name: string
    config: McpConfig
    secret_paths: string[]
    valid: boolean
    errors: string[]
    conflict: boolean
  }>
  errors: string[]
}
export type McpImportResult = {
  installed: McpConnection[]
  warning: string | null
  reload_task_id: string | null
}
export type McpAuthFlow = { flow_id: string; authorization_url: string; expires_at: string }

const root = '/api/v1/mcp-connections'
const connectionPath = (id: string) => root + '/' + encodeURIComponent(id)
const json = <T>(url: string, method: string, body: unknown, signal?: AbortSignal) =>
  apiJson<T>(url, {
    method,
    signal,
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })

// MCP connections belong to the running server and its workspace. They are not
// rebound to a directory selected in another Web pane.
export const mcpConnectionsApi = {
  setBridgeEnabled: (enabled: boolean, scope: McpScope) =>
    json<McpCatalog>(root + '/bridge/enabled', 'PUT', { enabled, scope }),
  list: (signal?: AbortSignal) => apiJson<McpCatalog>(root, { signal }),
  get: (id: string, signal?: AbortSignal) => apiJson<McpConnectionDetail>(connectionPath(id), { signal }),
  create: (name: string, scope: McpScope, config: McpConfig) =>
    json<McpMutationResult>(root, 'POST', { name, scope, config }),
  update: (id: string, config: McpConfig, preserve_secrets: string[], expected_revision: string) =>
    json<McpMutationResult>(connectionPath(id), 'PUT', { config, preserve_secrets, expected_revision }),
  remove: (id: string, expectedRevision: string) =>
    apiJson<McpMutationResult>(connectionPath(id) + '?expected_revision=' + encodeURIComponent(expectedRevision), {
      method: 'DELETE',
    }),
  setEnabled: (id: string, enabled: boolean, expected_revision: string) =>
    json<McpMutationResult>(connectionPath(id) + '/enabled', 'PUT', { enabled, expected_revision }),
  reconnect: (id: string) => json<McpConnectionDetail>(connectionPath(id) + '/reconnect', 'POST', {}),
  test: (id: string) => json<McpTestResult>(connectionPath(id) + '/test', 'POST', {}),
  capabilities: (id: string, kind: McpCapabilityKind, cursor?: string | null, signal?: AbortSignal) => {
    const query = new URLSearchParams({ kind })
    if (cursor) query.set('cursor', cursor)
    return apiJson<McpCapabilityPage>(connectionPath(id) + '/capabilities?' + query, { signal })
  },
  readResource: (id: string, uri: string) =>
    json<Record<string, unknown>>(connectionPath(id) + '/resource', 'POST', { uri }),
  getPrompt: (id: string, name: string, args: Record<string, string>) =>
    json<Record<string, unknown>>(connectionPath(id) + '/prompt', 'POST', { name, arguments: args }),
  previewImport: (scope: McpScope, document: Record<string, unknown>, signal?: AbortSignal) =>
    json<McpImportPreview>(root + '/import/preview', 'POST', { scope, document }, signal),
  import: (scope: McpScope, document: Record<string, unknown>, selected: string[], replace: boolean, expected_revision: string) =>
    json<McpImportResult>(root + '/import', 'POST', { scope, document, selected, replace, expected_revision }),
  setCredential: (id: string, token: string) =>
    json<McpConnectionDetail>(connectionPath(id) + '/credential', 'PUT', { token }),
  removeCredential: (id: string) => apiJson<McpConnectionDetail>(connectionPath(id) + '/credential', { method: 'DELETE' }),
  startAuth: (id: string, redirect_uri: string) =>
    json<McpAuthFlow>(connectionPath(id) + '/auth/start', 'POST', { redirect_uri }),
  completeAuth: (id: string, flow_id: string, callback_url: string) =>
    json<McpConnectionDetail>(connectionPath(id) + '/auth/complete', 'POST', { flow_id, callback_url }),
  logout: (id: string, flowId?: string) =>
    apiJson<McpConnectionDetail>(connectionPath(id) + '/auth' + (flowId ? '?flow_id=' + encodeURIComponent(flowId) : ''), {
      method: 'DELETE',
    }),
}
