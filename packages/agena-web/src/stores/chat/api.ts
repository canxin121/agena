// Agena REST API client for the chat subsystem.
//
// All endpoints live under /api/v1. Ids are numeric; timestamps are RFC3339
// UTC. Error responses are {problem:{id, code, category, ...}} envelopes;
// apiJson throws ApiError with .status / .code on non-2xx.
//
// Endpoint mapping (opencode → agena):
//   GET    /api/session?directory=          → GET    /api/v1/sessions?limit=&cursor=
//   POST   /api/session                     → POST   /api/v1/sessions            {workspace_id?}
//   DELETE /api/session/:id                 → DELETE /api/v1/sessions/{id}
//   PATCH  /api/session/:id {title}         → PUT    /api/v1/sessions/{id}      {title}
//   POST   /api/session/:id/message         → POST   /api/v1/sessions/{id}/messages  {document, options}
//   GET    /api/session/:id/message         → GET    /api/v1/sessions/{id}/parts?limit=
//   POST   /api/session/:id/abort           → POST   /api/v1/sessions/{id}/cancel    {execution_id}
//   POST   /api/session/:id/revert          → POST   /api/v1/sessions/{id}/rewind    {turn_id}
//   POST   /api/session/:id/summarize       → POST   /api/v1/sessions/{id}/compact   {options}
//   POST   /api/permission/:id/reply        → POST   /api/v1/sessions/{id}/permission-replies {reply}
//   POST   /api/question/:id/reply          → POST   /api/v1/sessions/{id}/user-input-replies {reply}
//   GET    /api/session-activity            → GET    /api/v1/activities
//   GET    /api/session/:id/diff            → (no agena endpoint — removed)
//   POST   /api/session/:id/share           → (no agena endpoint — fork instead)
//   GET    /api/session/status              → derived from GET /api/v1/sessions/{id}/state

import { apiJson } from '../../lib/api'
import type { JsonObject, JsonValue } from '@/types/json'
import type { MessageEntry, MessagePart, MessageInfo, Session } from '../../types/chat'

// --- agena wire projections ------------------------------------------------

/** SessionResource — GET /api/v1/sessions, overview items, etc. */
export type AgenaSession = {
  id: number
  parent_id?: number | null
  depth?: number
  root_id?: number
  workspace_id?: number
  title?: string
  version?: number
  relation_kind?: string
  lifecycle_state?: string
  state?: string
  is_subagent?: boolean
  message_count?: number
  child_session_count?: number
  last_message_at?: string | null
  created_at?: string
  updated_at?: string
  [k: string]: JsonValue
}

/** PartResource / SessionTranscriptPart — the shared part wire shape. */
export type AgenaPart = {
  part_id: number
  kind: string
  role: string
  state: string
  content: JsonValue
  summary?: string | null
  created_at_ms?: number
  started_at_ms?: number
  finished_at_ms?: number | null
  parent_part_id?: number | null
  run_id?: number | null
  [k: string]: JsonValue
}

/** SessionPartsResource — GET /api/v1/sessions/{id}/parts. */
export type AgenaSessionParts = {
  session_id: number
  version: number
  parts: AgenaPart[]
}

/** SessionExecutionResource — GET /api/v1/sessions/{id}/state. */
export type AgenaExecutionState = {
  session: AgenaSession
  parts: AgenaPart[]
  workflow_state: 'quiescent' | 'tool_pending' | 'blocked' | string
  active_execution?: { execution_id: string; phase?: string } | null
  latest_event_seq?: number | null
  pending_interactive_requests?: JsonValue[]
  usage?: JsonValue
  [k: string]: JsonValue
}

export type SessionListResponse = {
  sessions: Session[]
  total?: number
  hasMore?: boolean
  nextCursor?: string | null
}

export type MessageListResponse = {
  entries: MessageEntry[]
  hasMore?: boolean
  nextCursor?: string | null
}

export type SendMessageResponse = {
  queued?: boolean
}

export type SessionExecutionStatus = {
  state: string
  workflow_state: string
  active_execution?: { execution_id?: string; phase?: string } | null
  pending_interactive_requests?: JsonValue[]
  running: boolean
}

// --- helpers ---------------------------------------------------------------

function isRecord(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asRecord(value: JsonValue): JsonObject {
  return isRecord(value) ? value : {}
}

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined
}

function str(v: unknown): string {
  return typeof v === 'string' ? v.trim() : ''
}

function toSession(s: unknown): Session | null {
  const rec = asRecord(s as JsonValue)
  if (!rec || typeof rec.id !== 'number') return null
  const id = String(rec.id)
  const title = str(rec.title)
  const lastAt = typeof rec.last_message_at === 'string' ? (rec.last_message_at as string) : null
  const created = str(rec.created_at)
  const updated = str(rec.updated_at)
  return {
    ...(rec as unknown as Session),
    id,
    ...(title ? { title } : {}),
    ...(lastAt ? { last_message_at: lastAt } : {}),
    ...(created ? { created_at: created } : {}),
    ...(updated ? { updated_at: updated } : {}),
  }
}

function entriesFromParts(sessionId: string, parts: JsonValue[]): MessageEntry[] {
  const list: MessageEntry[] = []
  const map = new Map<string, MessageEntry>()
  const order: string[] = []
  const now = Date.now()

  const ensure = (key: string, info: MessageInfo): MessageEntry => {
    let entry = map.get(key)
    if (!entry) {
      entry = { info, parts: [] }
      map.set(key, entry)
      order.push(key)
    }
    return entry
  }

  for (const raw of parts) {
    const part = asRecord(raw)
    const kind = str(part.kind)
    const partId = typeof part.part_id === 'number' ? part.part_id : undefined
    if (typeof partId !== 'number') continue
    const partIdStr = String(partId)
    const role = str(part.role) || 'assistant'
    const content = isRecord(part.content) ? part.content : {}
    const createdMs = typeof part.created_at_ms === 'number' ? part.created_at_ms : now
    const runIdRaw = typeof part.run_id === 'number' ? part.run_id : undefined

    if (kind === 'run') {
      const created = typeof part.created_at_ms === 'number' ? part.created_at_ms : now
      const finished = typeof part.finished_at_ms === 'number' ? part.finished_at_ms : undefined
      const info: MessageInfo = {
        id: partIdStr,
        sessionID: sessionId,
        role,
        runId: partId,
        time: { created, ...(typeof finished === 'number' ? { completed: finished } : {}) },
      }
      // Run marker metadata can carry provider/model identity.
      const providerID = str(content.provider_id)
      const modelID = str(content.model_id)
      const turnId = str(content.turn_id)
      if (providerID) info.providerID = providerID
      if (modelID) info.modelID = modelID
      if (turnId) info.turnId = turnId
      ensure(partIdStr, info)
      continue
    }

    // Content part: attach to its run message (or an orphan message).
    const key = runIdRaw != null ? String(runIdRaw) : partIdStr
    let entry = map.get(key)
    if (!entry) {
      const info: MessageInfo = {
        id: key,
        sessionID: sessionId,
        role,
        runId: runIdRaw,
        time: { created: createdMs },
      }
      entry = ensure(key, info)
    }
    if (!entry.parts) entry.parts = []
    const normalized = normalizeAgenaPart(partIdStr, sessionId, key, raw as JsonValue)
    if (normalized) entry.parts.push(normalized)
  }

  for (const key of order) {
    const entry = map.get(key)
    if (entry && entry.parts) {
      entry.parts.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    }
    if (entry) list.push(entry)
  }
  return list
}

// --- part normalization ----------------------------------------------------

function asObject(value: JsonValue): JsonObject {
  return isRecord(value) ? value : {}
}

function stringField(value: JsonValue, keys: string[]): string {
  const rec = asObject(value)
  for (const key of keys) {
    const v = rec[key]
    if (typeof v === 'string' && v.trim()) return v.trim()
  }
  return ''
}

function arrayStringField(value: JsonValue, key: string): string {
  const rec = asObject(value)
  const raw = rec[key]
  if (Array.isArray(raw)) {
    return raw
      .filter((v): v is string => typeof v === 'string')
      .join('\n')
      .trim()
  }
  return ''
}

/**
 * Map an agena part (PartResource / SessionTranscriptPart) into the frontend
 * MessagePart shape consumed by ToolInvocation.vue / ReasoningInvocation.vue /
 * MessageItem.vue. Content is parsed defensively per part kind:
 *
 *   run           → message marker (handled by entriesFromParts)
 *   text          → type 'text', text = content.text
 *   think         → type 'reasoning', text = summary[] + raw[]
 *   tool_call     → type 'tool', tool = content.name, state {status, input, output, error, metadata}
 *   tool_result   → type 'text' (synthetic) with content.output
 *   file_ref      → type 'file', url/filename/mime from content
 *   paste_ref     → type 'text', text
 *   skill_ref     → type 'tool' (tool='skill')
 *   notice/hook   → type 'tool' (tool=hook/kind, output=summary)
 *   compaction    → type 'compaction', text = summary
 *   system_notification → type 'tool' (tool=operation_kind, output=summary)
 *   unknown       → type 'text' with JSON fallback
 */
export function normalizeAgenaPart(
  partIdStr: string,
  sessionId: string,
  messageId: string,
  raw: JsonValue,
): MessagePart | null {
  const part = asRecord(raw)
  const kind = str(part.kind)
  const state = str(part.state) || 'completed'
  const content = part.content
  const createdMs = typeof part.created_at_ms === 'number' ? part.created_at_ms : Date.now()
  const startedMs = typeof part.started_at_ms === 'number' ? part.started_at_ms : createdMs
  const finishedMs = typeof part.finished_at_ms === 'number' ? part.finished_at_ms : undefined

  const base: MessagePart = {
    id: partIdStr,
    sessionID: sessionId,
    messageID: messageId,
    type: 'text',
    partState: state,
    time: {
      start: startedMs,
      ...(typeof finishedMs === 'number' ? { end: finishedMs } : {}),
    },
  }

  const toStatus = (s: string): 'pending' | 'running' | 'completed' | 'error' => {
    if (s === 'pending') return 'pending'
    if (s === 'in_progress') return 'running'
    if (s === 'completed') return 'completed'
    return 'error'
  }

  switch (kind) {
    case 'text': {
      const text = stringField(content, ['text'])
      if (!text) return null
      return { ...base, type: 'text', text }
    }
    case 'think': {
      const summary = arrayStringField(content, 'summary')
      const rawContent = arrayStringField(content, 'raw')
      const text = summary || rawContent
      if (!text) return null
      return { ...base, type: 'reasoning', text }
    }
    case 'tool_call': {
      const toolName = stringField(content, ['name', 'tool']) || 'unknown'
      const input = asObject(content.input)
      const output =
        stringField(content, ['output']) ||
        stringField(asObject(content.result), ['output', 'text']) ||
        stringField(part.provider_state as JsonValue, ['output'])
      const error =
        stringField(content, ['error']) || stringField(asObject(content.result), ['error', 'message']) || ''
      const metaCandidate = asObject(content.metadata)
      const toolState = {
        status: toStatus(state),
        input,
        ...(output ? { output } : {}),
        ...(error ? { error } : {}),
        ...(Object.keys(metaCandidate).length ? { metadata: metaCandidate } : {}),
      }
      return {
        ...base,
        type: 'tool',
        tool: toolName,
        state: toolState,
        ...(Object.keys(metaCandidate).length ? { metadata: metaCandidate } : {}),
      }
    }
    case 'tool_result': {
      const text = stringField(content, ['output', 'text'])
      if (!text) return null
      return { ...base, type: 'text', text, synthetic: true }
    }
    case 'file_ref': {
      const name = stringField(content, ['name'])
      return {
        ...base,
        type: 'file',
        ...(name ? { filename: name } : {}),
        ...(stringField(content, ['path']) ? { url: stringField(content, ['path']) } : {}),
        ...(stringField(content, ['mime']) ? { mime: stringField(content, ['mime']) } : {}),
      }
    }
    case 'paste_ref': {
      const text = stringField(content, ['text'])
      if (!text) return null
      return { ...base, type: 'text', text, synthetic: true }
    }
    case 'skill_ref': {
      const name = stringField(content, ['skill', 'name']) || 'skill'
      const description = stringField(content, ['description'])
      return {
        ...base,
        type: 'tool',
        tool: 'skill',
        state: {
          status: 'completed',
          input: { name },
          ...(description ? { output: description } : {}),
        },
      }
    }
    case 'notice':
    case 'hook': {
      const hook = stringField(content, ['hook', 'kind']) || kind
      const summary = stringField(content, ['summary', 'detail', 'message'])
      return {
        ...base,
        type: 'tool',
        tool: hook,
        state: {
          status: toStatus(state),
          input: {},
          ...(summary ? { output: summary } : {}),
        },
      }
    }
    case 'compaction': {
      const summary = stringField(content, ['summary', 'detail'])
      return { ...base, type: 'compaction', ...(summary ? { text: summary } : {}) }
    }
    case 'system_notification': {
      const opKind = stringField(content, ['operation_kind']) || 'background'
      const summary = stringField(content, ['summary', 'body', 'detail'])
      return {
        ...base,
        type: 'tool',
        tool: opKind,
        state: { status: toStatus(state), input: {}, ...(summary ? { output: summary } : {}) },
      }
    }
    default: {
      // Unknown kinds: keep as text so nothing disappears from the transcript.
      try {
        const text = JSON.stringify(content)
        if (!text || text === '{}') return null
        return { ...base, type: 'text', text, synthetic: true }
      } catch {
        return null
      }
    }
  }
}

// --- sessions --------------------------------------------------------------

/** GET /api/v1/sessions — flat recent list. */
export async function listSessions(opts?: {
  limit?: number
  cursor?: string
  search?: string
  signal?: AbortSignal
}): Promise<SessionListResponse> {
  const params: string[] = []
  const limit = typeof opts?.limit === 'number' && Number.isFinite(opts.limit) ? Math.floor(opts.limit) : 30
  params.push(`limit=${encodeURIComponent(String(Math.max(1, Math.min(1000, limit))))}`)
  const cursor = typeof opts?.cursor === 'string' ? opts.cursor.trim() : ''
  if (cursor) params.push(`cursor=${encodeURIComponent(cursor)}`)
  const search = typeof opts?.search === 'string' ? opts.search.trim() : ''
  if (search) params.push(`search=${encodeURIComponent(search)}`)
  const suffix = params.length ? `?${params.join('&')}` : ''

  const payload = await apiJson<JsonValue>(`/api/v1/sessions${suffix}`, opts?.signal ? { signal: opts.signal } : undefined)
  const body = asRecord(payload)
  const rawItems = body.items
  const items = Array.isArray(rawItems) ? rawItems : []
  const sessions = items
    .map((s) => toSession(s))
    .filter((s): s is Session => Boolean(s))
  const page = asRecord(body.page)
  const nextCursor = typeof page.next_cursor === 'string' ? (page.next_cursor as string) : null
  return {
    sessions,
    total: typeof body.total === 'number' ? Number(body.total) : undefined,
    hasMore: page.has_more === true,
    nextCursor,
  }
}

/** POST /api/v1/sessions — create a session in the server workspace. */
export async function createSession(workspaceId?: number): Promise<Session> {
  const payload: JsonValue = {}
  if (typeof workspaceId === 'number' && Number.isFinite(workspaceId)) {
    ;(payload as JsonObject).workspace_id = workspaceId
  }
  const created = await apiJson<JsonValue>('/api/v1/sessions', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  })
  const session = toSession(created)
  if (!session) throw new Error('Server did not return a session')
  return session
}

/** DELETE /api/v1/sessions/{id} */
export async function deleteSession(sessionId: string): Promise<void> {
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}`, { method: 'DELETE' })
}

/** PUT /api/v1/sessions/{id} — rename. */
export async function patchSessionTitle(sessionId: string, title: string): Promise<Session> {
  const updated = await apiJson<JsonValue>(`/api/v1/sessions/${encodeURIComponent(sessionId)}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ title }),
  })
  const session = toSession(updated)
  if (!session) throw new Error('Server did not return a session')
  return session
}

/** GET /api/v1/sessions/{id}/state — execution + parts + pending interactions. */
export async function getSessionExecution(sessionId: string): Promise<AgenaExecutionState> {
  return await apiJson<AgenaExecutionState>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/state`)
}

/** GET /api/v1/sessions/{id}/parts — ordered part snapshot (reconnect catch-up). */
export async function getSessionParts(sessionId: string, limit?: number): Promise<AgenaSessionParts> {
  const params = typeof limit === 'number' && Number.isFinite(limit) ? `?limit=${Math.floor(limit)}` : ''
  return await apiJson<AgenaSessionParts>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/parts${params}`)
}

/**
 * Load a session's transcript as MessageEntry[].
 * Uses GET /parts (ordered snapshot) with a hard `limit` for the visible window.
 */
export async function listMessages(sessionId: string, limit: number): Promise<MessageListResponse> {
  const sid = String(sessionId || '').trim()
  if (!sid) return { entries: [], hasMore: false }
  const parts = await getSessionParts(sid, Math.max(20, Math.min(1000, Math.floor(limit || 200))))
  return { entries: entriesFromParts(sid, parts.parts as unknown as JsonValue[]), hasMore: false }
}

export type SendMessagePayload = {
  document: JsonValue[]
  options?: JsonValue
}

/** POST /api/v1/sessions/{id}/messages — submit a composer document + run options. */
export async function sendMessage(
  sessionId: string,
  payload: SendMessagePayload,
): Promise<SendMessageResponse> {
  const resp = await apiJson<JsonValue>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/messages`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  })
  const body = asRecord(resp)
  return { queued: body.queued === true }
}

/** POST /api/v1/sessions/{id}/continue — resume a paused/interrupted run. */
export async function continueSession(sessionId: string, options?: JsonValue): Promise<void> {
  const body: JsonValue = options && Object.keys(asRecord(options)).length ? { options } : {}
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/continue`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

/** POST /api/v1/sessions/{id}/compact — context compaction. */
export async function compactSession(sessionId: string, options?: JsonValue): Promise<void> {
  const body: JsonValue = options && Object.keys(asRecord(options)).length ? { options } : {}
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/compact`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

/** POST /api/v1/sessions/{id}/cancel — abort an active execution. */
export async function cancelSession(sessionId: string, executionId: string): Promise<void> {
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/cancel`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ execution_id: executionId }),
  })
}

/** POST /api/v1/sessions/{id}/rewind — rewind to an earlier turn. */
export async function rewindSession(sessionId: string, turnId: string): Promise<void> {
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/rewind`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ turn_id: turnId }),
  })
}

/** POST /api/v1/sessions/{id}/fork — clone history into a child session. */
export async function forkSession(sessionId: string, opts?: { at_message_id?: number; title?: string }): Promise<Session> {
  const body: JsonValue = {}
  if (typeof opts?.at_message_id === 'number' && Number.isFinite(opts.at_message_id)) {
    ;(body as JsonObject).at_message_id = opts.at_message_id
  }
  const title = typeof opts?.title === 'string' ? opts.title.trim() : ''
  if (title) (body as JsonObject).title = title
  const created = await apiJson<JsonValue>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/fork`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  const session = toSession(created)
  if (!session) throw new Error('Server did not return a forked session')
  return session
}

// --- interactive replies ---------------------------------------------------

/** POST /api/v1/sessions/{id}/permission-replies */
export async function replyPermission(
  sessionId: string,
  requestId: string,
  reply: 'once' | 'always' | 'reject',
  message?: string,
): Promise<boolean> {
  const kind = reply === 'once' ? 'allow_once' : reply === 'always' ? 'allow_always' : 'deny_once'
  const replyBody: JsonValue = { request_id: requestId, kind }
  if (typeof message === 'string' && message.trim()) {
    ;(replyBody as JsonObject).reason = message.trim()
  }
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/permission-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ reply: replyBody }),
  })
  return true
}

/** POST /api/v1/sessions/{id}/user-input-replies */
export async function replyQuestion(
  sessionId: string,
  requestId: string,
  answers: Record<string, string[]>,
): Promise<boolean> {
  const replyBody: JsonValue = { request_id: requestId, kind: 'submit', answers }
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/user-input-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ reply: replyBody }),
  })
  return true
}

/** POST /api/v1/sessions/{id}/user-input-replies — cancel a question. */
export async function rejectQuestion(sessionId: string, requestId: string): Promise<boolean> {
  const replyBody: JsonValue = { request_id: requestId, kind: 'cancel' }
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/user-input-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ reply: replyBody }),
  })
  return true
}

/** POST /api/v1/interactive/{request_id}/present — ack that a prompt was shown. */
export async function presentInteractiveRequest(requestId: string): Promise<void> {
  try {
    await apiJson(`/api/v1/interactive/${encodeURIComponent(requestId)}/present`, { method: 'POST' })
  } catch {
    // Best-effort; presentation ack is advisory.
  }
}

// --- session execution status ----------------------------------------------

/** Best-effort execution status derived from GET /state. */
export async function getSessionExecutionStatus(sessionId: string): Promise<SessionExecutionStatus | null> {
  const sid = String(sessionId || '').trim()
  if (!sid) return null
  try {
    const state = await getSessionExecution(sid)
    const workflow = String(state.workflow_state || '')
    const s = String(state.session?.state || 'ready')
    const active = state.active_execution ?? null
    return {
      state: s,
      workflow_state: workflow,
      active_execution: active,
      pending_interactive_requests: Array.isArray(state.pending_interactive_requests)
        ? state.pending_interactive_requests
        : [],
      running: s === 'running' || s === 'creating' || (active != null && typeof active.execution_id === 'string'),
    }
  } catch {
    return null
  }
}

// Re-export normalization helpers for other modules.
export { entriesFromParts }

// Shared normalizeSessionDiffPayload / SessionFileDiff types were removed:
// agena has no per-session diff endpoint. (See MISSING endpoints in report.)
