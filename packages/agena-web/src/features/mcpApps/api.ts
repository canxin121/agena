import { apiJson, apiResponse } from '@/lib/api'

export type McpAppDocument = {
  name: string
  revision: string
  tool_catalog_revision?: string
  resource_catalog_revision?: string
  html: string
  ui?: unknown
  arguments: Record<string, unknown>
  tool_result: Record<string, unknown>
}

export type McpAppRequest =
  | { method: 'tools/list' | 'resources/list' | 'resources/templates/list'; params: { cursor?: string } }
  | { method: 'tools/call'; params: { name: string; arguments: Record<string, unknown> } }
  | { method: 'resources/read'; params: { uri: string } }

export type McpAppServerRequest =
  | { method: 'tools/call'; params: { name: string; arguments: Record<string, unknown>; task?: { ttl?: number } } }
  | { method: 'tasks/get' | 'tasks/result' | 'tasks/cancel'; params: { taskId: string } }
  | { method: 'tasks/update'; params: { taskId: string; inputResponses: Record<string, unknown> } }
  | { method: 'resources/read'; params: { uri: string } }
  | { method: 'sampling/createMessage'; params: Record<string, unknown> }

export type McpAppCatalog = 'tools' | 'resources'

export type McpAppViewSource = { sessionId: string; partId: string; revision: string }
export type McpAppViewTool = {
  name: string
  title?: string
  description?: string
  inputSchema: Record<string, unknown>
  outputSchema?: Record<string, unknown>
  execution?: { taskSupport: 'forbidden' | 'optional' | 'required' }
}
export type McpAppViewTaskMethod = 'tasks/get' | 'tasks/result' | 'tasks/update' | 'tasks/cancel'
export type McpAppViewInputMethod = 'elicitation/create' | 'sampling/createMessage'
export type McpAppPrompt = {
  title: string
  question: {
    question: string
    options?: Array<{ label: string; description?: string; custom?: boolean }>
    allow_custom?: boolean
  }
}
export type McpAppViewPrompt = { id: string; prompt: McpAppPrompt }
export type McpAppViewCall = { id: string } & (
  | {
      name: string
      arguments: Record<string, unknown>
      task?: Record<string, unknown>
      requestState?: string
      inputResponses?: Record<string, unknown>
    }
  | { method: Exclude<McpAppViewTaskMethod, 'tasks/update'>; taskId: string }
  | { method: 'tasks/update'; taskId: string; inputResponses: Record<string, unknown> }
  | { method: McpAppViewInputMethod; params: Record<string, unknown> }
  | { prompt: McpAppPrompt }
)
export type McpAppViewReply =
  | { status: 'result'; result: Record<string, unknown> }
  | { status: 'error'; message: string }
  | { status: 'input'; answer: string | null }

function appPath(sessionId: string, partId: string) {
  return `/api/v1/sessions/${encodeURIComponent(sessionId)}/parts/${encodeURIComponent(partId)}/mcp-app`
}

export function readMcpApp(sessionId: string, partId: string, signal: AbortSignal) {
  return apiJson<McpAppDocument>(appPath(sessionId, partId), { signal, cache: 'no-store', redirect: 'error' })
}

export function watchMcpAppCatalog(
  sessionId: string,
  partId: string,
  revision: string,
  catalog: McpAppCatalog,
  after: string,
  signal: AbortSignal,
) {
  return apiJson<{ revision: string; retry_after_ms: number }>(
    appPath(sessionId, partId) + `/${catalog}/changes?after=` + encodeURIComponent(after),
    { headers: { 'If-Match': `"${revision}"` }, signal, cache: 'no-store', redirect: 'error' },
  )
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

function appViewPath(source: McpAppViewSource, view?: string) {
  const path = appPath(source.sessionId, source.partId) + '/views'
  return view ? path + '/' + encodeURIComponent(view) : path
}

function appViewRequest(source: McpAppViewSource, method: string, signal?: AbortSignal, body?: unknown): RequestInit {
  const headers = { 'If-Match': `"${source.revision}"`, 'Content-Type': 'application/json' }
  return {
    method,
    headers,
    signal,
    body: body === undefined ? undefined : JSON.stringify(body),
    cache: 'no-store',
    redirect: 'error',
  }
}

export function openMcpAppView(
  source: McpAppViewSource,
  tools: McpAppViewTool[],
  taskExtension: boolean,
  signal: AbortSignal,
) {
  return apiJson<{ view: string }>(
    appViewPath(source),
    appViewRequest(source, 'POST', signal, { tools, ...(taskExtension && { task_extension: true }) }),
  )
}

export function updateMcpAppView(source: McpAppViewSource, view: string, tools: McpAppViewTool[], signal: AbortSignal) {
  return apiResponse(appViewPath(source, view), appViewRequest(source, 'PUT', signal, { tools }))
}

export function pollMcpAppView(source: McpAppViewSource, view: string, after: number | undefined, signal: AbortSignal) {
  const query = after === undefined ? '' : `?after=${after}`
  return apiJson<{ revision: number; calls: McpAppViewCall[] }>(
    appViewPath(source, view) + '/calls' + query,
    appViewRequest(source, 'GET', signal),
  )
}

export function callMcpAppServer(
  source: McpAppViewSource,
  view: string,
  request: McpAppServerRequest,
  signal: AbortSignal,
) {
  return apiJson<Record<string, unknown>>(
    appViewPath(source, view) + '/server',
    appViewRequest(source, 'POST', signal, request),
  )
}

export function replyToMcpAppView(
  source: McpAppViewSource,
  view: string,
  call: string,
  reply: McpAppViewReply,
  signal: AbortSignal,
) {
  return apiResponse(
    appViewPath(source, view) + '/calls/' + encodeURIComponent(call),
    appViewRequest(source, 'POST', signal, reply),
  )
}

export function closeMcpAppView(source: McpAppViewSource, view: string) {
  return apiResponse(appViewPath(source, view), { ...appViewRequest(source, 'DELETE'), keepalive: true })
}
