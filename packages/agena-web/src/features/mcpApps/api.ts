import { apiJson } from '@/lib/api'

export type McpAppDocument = {
  name: string
  revision: string
  html: string
  arguments: Record<string, unknown>
  tool_result: Record<string, unknown>
}

export type McpAppRequest =
  | { method: 'tools/call'; params: { name: string; arguments: Record<string, unknown> } }
  | { method: 'resources/read'; params: { uri: string } }

function appPath(sessionId: string, partId: string) {
  return `/api/v1/sessions/${encodeURIComponent(sessionId)}/parts/${encodeURIComponent(partId)}/mcp-app`
}

export function readMcpApp(sessionId: string, partId: string, signal: AbortSignal) {
  return apiJson<McpAppDocument>(appPath(sessionId, partId), { signal, cache: 'no-store', redirect: 'error' })
}

export function callMcpApp(
  sessionId: string,
  partId: string,
  revision: string,
  request: McpAppRequest,
  signal: AbortSignal,
) {
  return apiJson<Record<string, unknown>>(appPath(sessionId, partId), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'If-Match': `"${revision}"` },
    body: JSON.stringify(request),
    signal,
    redirect: 'error',
  })
}
