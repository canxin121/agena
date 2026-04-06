import { emitAuthRequired, extractAuthRequiredMessageFromBodyText } from '@/lib/authEvents'
import { apiJson, apiUrl } from '@/lib/api'
import { buildActiveUiAuthHeaders } from '@/lib/uiAuthToken'

export type StudioHealth = {
  status: string
  generation: number
  loadedAt: string
  workspaceRoot: string
  configPath: string
  configFound: boolean
  activeMode?: string | null
  providerIds: string[]
  sessionRuntimeAvailable: boolean
}

export type RuntimeStatus = {
  generation: number
  loaded_at: string
  workspace_root: string
  config_path: string
  config_found: boolean
  active_mode?: string | null
  auth_store_path: string
  provider_ids: string[]
  plugin_count: number
  session_runtime_available: boolean
  watch_paths: string[]
  reload: {
    enabled: boolean
    interval_secs: number
  }
  janitor: {
    enabled: boolean
    interval_secs: number
  }
}

export type RuntimeReloadResponse = {
  cause: string
  previous_generation: number
  generation: number
  loaded_at: string
}

export type ProviderSummary = {
  provider_id: string
  default_model: string
  default_model_ref: string
}

export type AuthProvider = {
  provider_id: string
  configured: boolean
  credential_present: boolean
  credential_type?: string | null
  key_preview?: string | null
  expires_at?: string | null
  expired?: boolean | null
  account_id?: string | null
  enterprise_url?: string | null
}

export type WorkspaceResource = {
  id: number
  path: string
  created_at: string
  updated_at: string
  session_count?: number | null
}

export type SessionResource = {
  id: number
  parent_id?: number | null
  workspace_id: number
  title: string
  version: number
  created_at: string
  updated_at: string
  message_count: number
  child_session_count: number
  last_message_at?: string | null
}

export type MessagePart = {
  id: number
  message_id: number
  part_index: number
  status: string
  kind: string
  name?: string | null
  summary?: string | null
  has_detail?: boolean
  operation_id?: string | null
  created_at: string
  content?: Record<string, unknown> | null
}

export type MessageResource = {
  id: number
  session_id: number
  role: 'user' | 'assistant' | 'system' | 'tool'
  state: string
  created_at: string
  updated_at: string
  metadata: Record<string, unknown>
  usage?: Record<string, unknown> | null
  finish?: string | null
  part_count: number
  parts?: MessagePart[] | null
}

export type PermissionRequest = {
  request_id: string
  session_id?: number | null
  action: Record<string, unknown>
  reason: string
  created_at: string
}

export type UserInputQuestion = {
  id: string
  header?: string
  question: string
  options?: Array<{
    label: string
    description?: string
  }>
  multiple?: boolean
  allow_custom?: boolean
}

export type UserInputRequest = {
  request_id: string
  session_id?: number | null
  questions: UserInputQuestion[]
  created_at: string
}

export type SessionExecutionResource = {
  session: SessionResource
  blocked: boolean
  run_state: 'idle' | 'awaiting_model' | string
  latest_event_seq?: number | null
  pending_permission_requests: PermissionRequest[]
  pending_user_input_requests: UserInputRequest[]
}

export type SessionEventRecord = {
  event_id?: number | null
  session_id: number
  seq: number
  event_type: string
  payload: Record<string, unknown>
  causation_id?: number | null
  correlation_id?: number | null
  created_at: string
}

export type SessionEventStreamHandle = {
  close: () => void
}

type PaginatedResponse<T> = {
  items: T[]
  page?: {
    limit: number
    returned: number
    has_more: boolean
    next_cursor?: string | null
    order: 'asc' | 'desc'
  }
}

async function collectPagedItems<T>(
  fetchPage: (cursor?: string) => Promise<PaginatedResponse<T>>,
  options?: {
    merge?: 'append' | 'prepend'
    maxPages?: number
    resourceName?: string
  },
): Promise<T[]> {
  const merge = options?.merge ?? 'append'
  const maxPages = Math.max(1, Math.trunc(options?.maxPages ?? 100))
  const resourceName = options?.resourceName ?? 'paged resource'
  let cursor: string | undefined
  let items: T[] = []
  const seenCursors = new Set<string>()

  for (let pageIndex = 0; pageIndex < maxPages; pageIndex += 1) {
    const response = await fetchPage(cursor)
    const chunk = response.items ?? []
    items = merge === 'prepend' ? chunk.concat(items) : items.concat(chunk)

    const nextCursor = response.page?.next_cursor ?? undefined
    if (!response.page?.has_more || !nextCursor) {
      return items
    }

    if (seenCursors.has(nextCursor)) {
      throw new Error(`Pagination cursor repeated while loading ${resourceName}`)
    }
    seenCursors.add(nextCursor)
    cursor = nextCursor
  }

  throw new Error(`Pagination exceeded ${maxPages} pages while loading ${resourceName}`)
}

function normalizeSseBuffer(buffer: string): string {
  return buffer.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
}

function parseSseEventBlock(block: string): {
  event: string
  id: string
  data: string
} {
  let event = 'message'
  let id = ''
  const data: string[] = []

  for (const rawLine of block.split('\n')) {
    if (!rawLine || rawLine.startsWith(':')) continue

    const separator = rawLine.indexOf(':')
    const field = separator >= 0 ? rawLine.slice(0, separator) : rawLine
    const value = separator >= 0 ? rawLine.slice(separator + 1).replace(/^ /, '') : ''

    switch (field) {
      case 'event':
        event = value || 'message'
        break
      case 'id':
        id = value
        break
      case 'data':
        data.push(value)
        break
      default:
        break
    }
  }

  return {
    event,
    id,
    data: data.join('\n'),
  }
}

function extractErrorCode(bodyText: string): string {
  const txt = String(bodyText || '').trim()
  if (!txt) return ''

  try {
    const parsed = JSON.parse(txt) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return ''
    const record = parsed as Record<string, unknown>
    if (typeof record.code === 'string') return record.code.trim()

    const nested = record.error
    if (nested && typeof nested === 'object' && !Array.isArray(nested)) {
      const nestedCode = (nested as Record<string, unknown>).code
      if (typeof nestedCode === 'string') return nestedCode.trim()
    }
  } catch {
    // ignore non-json payloads
  }

  return ''
}

export async function fetchStudioHealth(): Promise<StudioHealth> {
  return await apiJson<StudioHealth>('/health')
}

export async function fetchRuntimeStatus(): Promise<RuntimeStatus> {
  return await apiJson<RuntimeStatus>('/api/v1/runtime')
}

export async function reloadRuntime(): Promise<RuntimeReloadResponse> {
  return await apiJson<RuntimeReloadResponse>('/api/v1/runtime/reload', {
    method: 'POST',
  })
}

export async function listProviders(): Promise<ProviderSummary[]> {
  return await apiJson<ProviderSummary[]>('/api/v1/providers')
}

export async function listAuthProviders(): Promise<AuthProvider[]> {
  return await apiJson<AuthProvider[]>('/api/v1/auth/providers')
}

export async function setProviderApiKey(providerId: string, apiKey: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}/api-key`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ api_key: apiKey }),
  })
}

export async function deleteProviderCredential(providerId: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}`, {
    method: 'DELETE',
  })
}

export async function refreshProviderCredential(providerId: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}/refresh`, {
    method: 'POST',
  })
}

export async function listWorkspaces(): Promise<WorkspaceResource[]> {
  return await collectPagedItems(
    (cursor) =>
      apiJson<PaginatedResponse<WorkspaceResource>>(
        `/api/v1/workspaces?limit=100${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`,
      ),
    { resourceName: 'workspaces' },
  )
}

export async function resolveWorkspace(path: string, createIfMissing: boolean): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>('/api/v1/workspaces/resolve', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      path,
      create_if_missing: createIfMissing,
    }),
  })
}

export async function createWorkspace(path: string): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>('/api/v1/workspaces', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path }),
  })
}

export async function listSessions(workspaceId: number): Promise<SessionResource[]> {
  return await collectPagedItems(
    (cursor) =>
      apiJson<PaginatedResponse<SessionResource>>(
        `/api/v1/sessions?workspace_id=${encodeURIComponent(String(workspaceId))}&limit=100${
          cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''
        }`,
      ),
    { resourceName: 'sessions' },
  )
}

export async function createSession(input: {
  workspaceId: number
  title: string
  parentId?: number | null
}): Promise<SessionResource> {
  return await apiJson<SessionResource>('/api/v1/sessions', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      workspace_id: input.workspaceId,
      title: input.title,
      parent_id: input.parentId ?? null,
    }),
  })
}

export async function getSessionState(sessionId: number): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${sessionId}/state`)
}

export async function listMessages(sessionId: number): Promise<MessageResource[]> {
  return await collectPagedItems(
    (cursor) =>
      apiJson<PaginatedResponse<MessageResource>>(
        `/api/v1/sessions/${sessionId}/messages?parts=full&limit=100${
          cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''
        }`,
      ),
    { merge: 'prepend', maxPages: 1000, resourceName: 'session messages' },
  )
}

export function streamSessionEvents(
  sessionId: number,
  options: {
    afterSeq?: number | null
    pollIntervalMs?: number
    onEvent: (event: SessionEventRecord) => void
    onError?: (error: Error) => void
    onOpen?: () => void
  },
): SessionEventStreamHandle {
  const controller = new AbortController()
  const decoder = new TextDecoder()
  const pollIntervalMs = Math.max(50, Math.trunc(options.pollIntervalMs ?? 250))
  let closed = false
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  let afterSeq = Math.max(0, Math.trunc(options.afterSeq ?? 0))

  const scheduleReconnect = (delayMs: number) => {
    if (closed || reconnectTimer) return
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null
      void connect()
    }, delayMs)
  }

  const close = () => {
    closed = true
    controller.abort()
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
  }

  const handleEventBlock = (block: string) => {
    const parsed = parseSseEventBlock(block)
    if (!parsed.data) return

    if (parsed.event === 'error') {
      options.onError?.(new Error(parsed.data))
      return
    }

    if (parsed.event !== 'session_event') return

    const record = JSON.parse(parsed.data) as SessionEventRecord
    if (typeof record.seq === 'number' && Number.isFinite(record.seq)) {
      afterSeq = Math.max(afterSeq, record.seq)
    } else if (parsed.id) {
      const seq = Number(parsed.id)
      if (Number.isFinite(seq)) {
        afterSeq = Math.max(afterSeq, seq)
      }
    }
    options.onEvent(record)
  }

  const readResponseStream = async (response: Response) => {
    const reader = response.body?.getReader()
    if (!reader) {
      throw new Error('Session event stream response body is unavailable')
    }

    let buffer = ''
    while (!closed) {
      const { done, value } = await reader.read()
      buffer = normalizeSseBuffer(buffer + decoder.decode(value ?? new Uint8Array(), { stream: !done }))

      let boundary = buffer.indexOf('\n\n')
      while (boundary >= 0) {
        const block = buffer.slice(0, boundary).trim()
        buffer = buffer.slice(boundary + 2)
        if (block) {
          handleEventBlock(block)
        }
        boundary = buffer.indexOf('\n\n')
      }

      if (done) {
        const trailing = buffer.trim()
        if (trailing) {
          handleEventBlock(trailing)
        }
        return
      }
    }
  }

  const connect = async () => {
    if (closed) return

    try {
      const authHeaders = buildActiveUiAuthHeaders()
      const url = new URL(apiUrl(`/api/v1/sessions/${sessionId}/events/stream`))
      if (afterSeq > 0) {
        url.searchParams.set('after_seq', String(afterSeq))
      }
      url.searchParams.set('poll_interval_ms', String(pollIntervalMs))

      const response = await fetch(url.toString(), {
        method: 'GET',
        signal: controller.signal,
        credentials: authHeaders.authorization ? 'omit' : 'include',
        headers: {
          accept: 'text/event-stream',
          ...(authHeaders.authorization ? authHeaders : {}),
        },
      })

      if (!response.ok) {
        const bodyText = await response.text().catch(() => '')
        const extractedMessage = extractAuthRequiredMessageFromBodyText(bodyText)
        const message = extractedMessage || bodyText.trim() || `Request failed (${response.status})`
        const code = extractErrorCode(bodyText)
        const isUiAuthRequired =
          response.status === 401 &&
          (code === 'auth_required' || message.trim().toLowerCase() === 'ui authentication required')
        if (isUiAuthRequired) {
          emitAuthRequired({
            message,
            status: response.status,
            code: code || 'auth_required',
            url: url.toString(),
          })
        }
        throw new Error(message)
      }

      options.onOpen?.()
      await readResponseStream(response)

      if (!closed) {
        scheduleReconnect(250)
      }
    } catch (error) {
      if (closed || controller.signal.aborted) return
      options.onError?.(error instanceof Error ? error : new Error(String(error)))
      scheduleReconnect(1_000)
    }
  }

  void connect()

  return { close }
}

export async function submitTurn(input: {
  sessionId: number
  text: string
  providerId?: string
  modelId?: string
}): Promise<SessionExecutionResource> {
  const body: Record<string, unknown> = {
    parts: [
      {
        type: 'text',
        text: input.text,
      },
    ],
  }

  if (input.providerId && input.modelId) {
    body.model = {
      provider_id: input.providerId,
      model_id: input.modelId,
    }
  }

  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/turns`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

export async function replyPermission(input: {
  sessionId: number
  requestId: string
  kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always'
  reason?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/permission-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      reply: {
        request_id: input.requestId,
        kind: input.kind,
        ...(input.reason ? { reason: input.reason } : {}),
      },
    }),
  })
}

export async function replyUserInput(input: {
  sessionId: number
  requestId: string
  answers: Record<string, string[]>
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/user-input-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      reply: {
        request_id: input.requestId,
        kind: 'submit',
        answers: input.answers,
      },
    }),
  })
}

export async function cancelUserInput(input: {
  sessionId: number
  requestId: string
  reason?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/user-input-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      reply: {
        request_id: input.requestId,
        kind: 'cancel',
        ...(input.reason ? { reason: input.reason } : {}),
      },
    }),
  })
}
