// Agena REST API client for the chat subsystem.
//
// All endpoints live under /api/v1. Ids are numeric; timestamps are RFC3339
// UTC. Error responses are {problem:{id, code, category, ...}} envelopes;
// apiJson throws ApiError with .status / .code on non-2xx.
//
// RunOptions are flattened into message/continue/compact request bodies. The
// server rejects unknown fields, including the old Agent/profile selection.

import { apiJson } from '../../lib/api'
import { isRunInFlight, isRunTerminal } from '../../lib/chatRunState'
import { normalizeSessionState } from '../../types/chat'
import type { JsonObject, JsonValue } from '@/types/json'
import type {
  MessageEntry,
  MessageError,
  MessageFold,
  MessagePart,
  MessageInfo,
  Session,
  SessionState,
} from '../../types/chat'
import { compareChatIds } from './messageIndex'

// --- agena wire projections ------------------------------------------------

/** SessionResource — GET /api/v1/sessions, overview items, etc. */
export type AgenaSession = {
  id: number
  parent_id?: number | null
  depth?: number
  root_id?: number
  workspace_id?: number
  title?: string
  favorite?: boolean
  pinned?: boolean
  version?: number
  relation_kind?: string
  lifecycle_state?: string
  state?: SessionState
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
  presentation?: JsonValue | null
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
  folds?: Array<{
    run_id: number
    run_ids?: number[]
    anchor_part_id: number
    hidden_count: number
    next_cursor?: string | null
  }>
  page?: {
    returned?: number
    has_more?: boolean
    next_cursor?: string | null
  }
}

/** SessionExecutionResource — GET /api/v1/sessions/{id}/state. */
export type AgenaExecutionState = {
  session: AgenaSession
  parts: AgenaPart[]
  latest_event_seq?: number | null
  execution?: {
    agent_id?: string
    model_provider_id?: string | null
    model_adapter_id?: string | null
    model_id?: string | null
    model_thinking_mode?: string | null
    model_speed_mode?: string | null
    model_verbosity?: string | null
    model_parallel_tool_calls?: boolean | null
    effective_workspace_root?: string | null
    [k: string]: JsonValue
  }
  usage?: {
    measured_prompt_tokens?: number | null
    current_tokens?: number
    projected_tokens?: number | null
    limit_tokens?: number | null
    limit_basis?: string | null
    reserved_tokens?: number | null
    model_context_window_tokens?: number | null
    model_max_input_tokens?: number | null
    model_max_output_tokens?: number | null
    [k: string]: JsonValue
  }
  [k: string]: JsonValue
}

export type AgenaModelRef = {
  provider_id: string
  adapter_id?: string
  model_id: string
}

export type RunOptionsPayload = {
  model?: AgenaModelRef
  thinking_mode?: string
  speed_mode?: string
  verbosity?: string
  parallel_tool_calls?: boolean
  system?: string
  temperature?: number
  max_output_tokens?: number
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

function messageFoldsFromWire(folds: AgenaSessionParts['folds']): MessageFold[] {
  if (!Array.isArray(folds)) return []
  return folds
    .filter(
      (fold) =>
        Number.isFinite(fold?.run_id) && Number.isFinite(fold?.anchor_part_id) && Number.isFinite(fold?.hidden_count),
    )
    .map((fold) => ({
      runId: Number(fold.run_id),
      runIds:
        Array.isArray(fold.run_ids) && fold.run_ids.length
          ? fold.run_ids.map((runId) => Number(runId)).filter(Number.isFinite)
          : [Number(fold.run_id)],
      anchorPartId: String(fold.anchor_part_id),
      hiddenCount: Math.max(0, Math.floor(Number(fold.hidden_count))),
      nextCursor: typeof fold.next_cursor === 'string' ? fold.next_cursor : null,
    }))
}

export type SendMessageResponse = {
  queued?: boolean
}

export type SessionExecutionStatus = {
  state: SessionState
  execution?: AgenaExecutionState['execution']
  usage?: AgenaExecutionState['usage']
}

export type CancellationResult = 'cancellation_requested' | 'already_terminal' | 'not_found' | 'execution_mismatch'

export type CancellationOutcome = {
  result: CancellationResult
  restored_user_message?: JsonValue | null
  restored_user_run_id?: number | null
}

// --- helpers ---------------------------------------------------------------

function isRecord(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asRecord(value: JsonValue): JsonObject {
  return isRecord(value) ? value : {}
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
    state: normalizeSessionState(rec.state),
    ...(title ? { title } : {}),
    ...(lastAt ? { last_message_at: lastAt } : {}),
    ...(created ? { created_at: created } : {}),
    ...(updated ? { updated_at: updated } : {}),
  }
}

function entriesFromParts(
  sessionId: string,
  parts: JsonValue[],
  folds: MessageFold[] = [],
  contextRunIds: readonly number[] = [],
): MessageEntry[] {
  const list: MessageEntry[] = []
  const map = new Map<string, MessageEntry>()
  const order: string[] = []
  const now = Date.now()

  const runMarkers = new Set<string>()
  for (const raw of parts) {
    const part = asRecord(raw)
    if (str(part.kind) !== 'run' || typeof part.part_id !== 'number') continue
    runMarkers.add(String(part.part_id))
  }
  const contextualRuns = new Set(contextRunIds.filter((runId) => Number.isFinite(runId)).map(String))

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
      const runState = str(part.state)
      const info: MessageInfo = {
        id: partIdStr,
        sessionID: sessionId,
        role,
        runId: partId,
        ...(runState ? { runState } : {}),
        runContent: content,
        ...(isRunTerminal(runState) ? { finish: runState } : {}),
        time: { created, ...(typeof finished === 'number' ? { completed: finished } : {}) },
      }
      // Run marker metadata can carry provider/model identity.
      const providerID = str(content.provider_id)
      const adapterID = str(content.adapter_id)
      const modelID = str(content.model_id)
      const turnId = str(content.turn_id)
      if (providerID) info.providerID = providerID
      if (adapterID) info.adapterID = adapterID
      if (modelID) info.modelID = modelID
      if (turnId) info.turnId = turnId
      ensure(partIdStr, info)
      continue
    }

    // Every content part must point at a materialized run marker. A missing
    // or unknown owner is malformed current data, not a message that can be
    // reconstructed by the client.
    if (runIdRaw == null) continue
    const key = String(runIdRaw)
    if (!runMarkers.has(key) && !contextualRuns.has(key)) continue
    let entry = map.get(key)
    if (!entry) {
      entry = ensure(key, {
        id: key,
        sessionID: sessionId,
        role,
        runId: runIdRaw,
        time: { created: createdMs },
      })
    }
    if (!entry.parts) entry.parts = []
    const messageError = messageErrorFromAgenaPart(raw)
    if (messageError) entry.info.error = messageError
    const normalized = normalizeAgenaPart(partIdStr, sessionId, key, raw as JsonValue)
    if (normalized) entry.parts.push(normalized)
  }

  for (const key of order) {
    const entry = map.get(key)
    if (entry && entry.parts) {
      entry.parts.sort((a, b) => compareChatIds(a.id, b.id))
    }
    if (entry) {
      const messageFolds = folds.filter((fold) => String(entry?.info.runId || entry?.info.id) === String(fold.runId))
      if (messageFolds.length) entry.folds = messageFolds
      list.push(entry)
    }
  }
  return list
}

// --- part normalization ----------------------------------------------------

function asObject(value: JsonValue): JsonObject {
  return isRecord(value) ? value : {}
}

function toolCallView(content: JsonObject, presentationValue: JsonValue): JsonObject {
  const presentation = asObject(presentationValue)
  const title = stringField(presentation, ['title'])
  const summary = stringField(presentation, ['summary'])
  const invocation = {
    name: stringField(content, ['name']) || 'unknown',
    plugin_name: content.plugin,
    input: asObject(content.input),
    tool_api_call: asObject(content.tool_api_call),
  }
  const blocks = Array.isArray(presentation.blocks) ? presentation.blocks : []
  return {
    call_id: content.call_id ?? 0,
    invocation,
    title,
    summary,
    blocks,
    user_input: asObject(content.user_input),
    authorization: asObject(content.authorization),
    metadata: asObject(content.metadata),
    error: content.error ?? null,
    lifecycle: asObject(content.lifecycle),
    output: content.output ?? null,
  }
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
      .join('')
      .trim()
  }
  return ''
}

function problemMessage(value: JsonValue): string {
  const problem = asObject(value)
  const user = asObject(problem.user)
  return stringField(user, ['fallback']) || stringField(problem, ['message'])
}

function operationFailureMessage(value: JsonValue): string {
  const error = asObject(value)
  return problemMessage(error.failure) || problemMessage(error.problem) || stringField(error, ['message', 'detail'])
}

export function messageErrorFromAgenaPart(raw: JsonValue): MessageError | null {
  const part = asRecord(raw)
  if (str(part.kind) !== 'error') return null
  const content = asObject(part.content)
  const problem = asObject(content.problem)
  const message = problemMessage(problem) || stringField(content, ['message']) || str(part.summary) || 'The run failed.'
  const code = stringField(problem, ['code'])
  const category = stringField(problem, ['category']) || stringField(content, ['category'])
  return {
    name: 'AgenaError',
    type: category || 'error',
    message,
    ...(code ? { code } : {}),
    ...(category ? { classification: category } : {}),
    problem,
  }
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
  const state = str(part.state)
  const content = part.content
  const createdMs = typeof part.created_at_ms === 'number' ? part.created_at_ms : Date.now()
  const startedMs = typeof part.started_at_ms === 'number' ? part.started_at_ms : createdMs
  const finishedMs = typeof part.finished_at_ms === 'number' ? part.finished_at_ms : undefined
  const runId = typeof part.run_id === 'number' ? part.run_id : null
  const parentPartId = typeof part.parent_part_id === 'number' ? part.parent_part_id : null
  const agenaSummary = typeof part.summary === 'string' ? part.summary : null
  const agenaPresentation = part.presentation ?? null

  const base: MessagePart = {
    id: partIdStr,
    sessionID: sessionId,
    messageID: messageId,
    type: 'text',
    ...(state ? { partState: state } : {}),
    agenaKind: kind,
    agenaRole: str(part.role) || 'assistant',
    agenaSummary,
    agenaContent: content,
    agenaPresentation,
    runId,
    parentPartId,
    time: {
      start: startedMs,
      ...(typeof finishedMs === 'number' ? { end: finishedMs } : {}),
    },
  }

  const toStatus = (s: string): 'pending' | 'running' | 'completed' | 'error' | '' => {
    if (s === 'pending') return 'pending'
    if (isRunInFlight(s)) return 'running'
    if (s === 'completed') return 'completed'
    if (isRunTerminal(s)) return 'error'
    return ''
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
      const operation = toolCallView(asObject(content), agenaPresentation)
      const invocation = asObject(operation.invocation)
      const toolName = stringField(content, ['name']) || stringField(invocation, ['name']) || 'unknown'
      const canonicalInput = asObject(asObject(content).input)
      const operationInput = asObject(invocation.input)
      const input = Object.keys(canonicalInput).length > 0 ? canonicalInput : operationInput
      const outputRecord = asObject(operation.output)
      const payloadRecord = asObject(outputRecord.payload)
      const output =
        stringField(agenaPresentation, ['summary']) ||
        stringField(outputRecord, ['text']) ||
        stringField(payloadRecord, ['text']) ||
        stringField(operation, ['summary'])
      const error = operationFailureMessage(operation.error)
      const metaCandidate = {
        ...asObject(operation.metadata),
        ...asObject(outputRecord.metadata),
        ...asObject(asObject(content).metadata),
      }
      const title = stringField(operation, ['title'])
      const status = toStatus(stringField(content, ['state']) || state)
      const toolState: JsonObject = {
        ...(status ? { status } : {}),
        input,
        ...(output ? { output } : {}),
        ...(error ? { error } : {}),
        ...(title ? { title } : {}),
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
    case 'file_ref': {
      const contentRecord = asObject(content)
      const attachments = Array.isArray(contentRecord.attachments) ? contentRecord.attachments : []
      const firstAttachment = attachments.length > 0 ? asObject(attachments[0]) : {}
      const source = {
        ...asObject(firstAttachment.source),
        ...asObject(contentRecord.source),
      }
      const name = stringField(content, ['name', 'title']) || stringField(firstAttachment, ['filename', 'title'])
      const mime = stringField(content, ['mime']) || stringField(firstAttachment, ['mime'])
      const path = stringField(content, ['path']) || stringField(source, ['path'])
      const directUrl =
        stringField(content, ['data_url', 'url']) ||
        stringField(source, ['data_url', 'url']) ||
        stringField(firstAttachment, ['data_url', 'url'])
      const base64 =
        stringField(content, ['base64']) ||
        stringField(source, ['base64', 'data']) ||
        stringField(firstAttachment, ['base64'])
      const url = directUrl || (base64 && mime ? `data:${mime};base64,${base64}` : '') || path
      return {
        ...base,
        type: 'file',
        ...(name ? { filename: name } : {}),
        ...(url ? { url } : {}),
        ...(path ? { serverPath: path } : {}),
        ...(mime ? { mime } : {}),
      }
    }
    case 'paste_ref': {
      const text = stringField(content, ['text'])
      if (!text) return null
      return { ...base, type: 'text', text, synthetic: true }
    }
    case 'skill_ref': {
      const contentRecord = asObject(content)
      const skills = Array.isArray(contentRecord.skills) ? contentRecord.skills : []
      const firstSkill = skills.length > 0 ? asObject(skills[0]) : {}
      const name = stringField(content, ['skill', 'name']) || stringField(firstSkill, ['name']) || 'skill'
      const description = stringField(content, ['description']) || stringField(firstSkill, ['description'])
      return {
        ...base,
        type: 'tool',
        tool: 'skill',
        state: {
          ...(toStatus(state) ? { status: toStatus(state) } : {}),
          input: { name },
          ...(description ? { output: description } : {}),
        },
      }
    }
    case 'notice':
    case 'hook': {
      const hook = stringField(content, ['hook', 'kind']) || kind
      const title = stringField(content, ['summary', 'title'])
      const output = stringField(content, ['detail', 'message', 'summary'])
      return {
        ...base,
        type: 'tool',
        tool: hook,
        state: {
          ...(toStatus(state) ? { status: toStatus(state) } : {}),
          input: {},
          ...(title ? { title } : {}),
          ...(output ? { output } : {}),
        },
      }
    }
    case 'compaction': {
      const summary = stringField(content, ['summary', 'detail']) || str(part.summary)
      return { ...base, type: 'compaction', ...(summary ? { text: summary } : {}) }
    }
    case 'error': {
      const message = messageErrorFromAgenaPart(raw)?.message || 'The run failed.'
      return { ...base, type: 'text', text: message, synthetic: true }
    }
    case 'system_notification': {
      const opKind = stringField(content, ['operation_kind']) || 'background'
      const title = stringField(content, ['summary'])
      const output = stringField(content, ['body', 'detail', 'summary'])
      return {
        ...base,
        type: 'tool',
        tool: opKind,
        state: {
          ...(toStatus(state) ? { status: toStatus(state) } : {}),
          input: {},
          ...(title ? { title } : {}),
          ...(output ? { output } : {}),
        },
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
  workspaceId?: number | string
  parentId?: number | string
  roots?: boolean
  excludeSubagents?: boolean
  signal?: AbortSignal
}): Promise<SessionListResponse> {
  const params: string[] = []
  const limit = typeof opts?.limit === 'number' && Number.isFinite(opts.limit) ? Math.floor(opts.limit) : 30
  params.push(`limit=${encodeURIComponent(String(Math.max(1, Math.min(1000, limit))))}`)
  const cursor = typeof opts?.cursor === 'string' ? opts.cursor.trim() : ''
  if (cursor) params.push(`cursor=${encodeURIComponent(cursor)}`)
  const search = typeof opts?.search === 'string' ? opts.search.trim() : ''
  if (search) params.push(`search=${encodeURIComponent(search)}`)
  const workspaceId = Number(opts?.workspaceId)
  if (Number.isSafeInteger(workspaceId) && workspaceId > 0) {
    params.push(`workspace_id=${encodeURIComponent(String(workspaceId))}`)
  }
  const parentId = Number(opts?.parentId)
  if (Number.isSafeInteger(parentId) && parentId > 0) {
    params.push(`parent_id=${encodeURIComponent(String(parentId))}`)
  }
  if (typeof opts?.roots === 'boolean') params.push(`roots=${opts.roots ? 'true' : 'false'}`)
  if (typeof opts?.excludeSubagents === 'boolean') {
    params.push(`exclude_subagents=${opts.excludeSubagents ? 'true' : 'false'}`)
  }
  const suffix = params.length ? `?${params.join('&')}` : ''

  const payload = await apiJson<JsonValue>(
    `/api/v1/sessions${suffix}`,
    opts?.signal ? { signal: opts.signal } : undefined,
  )
  const body = asRecord(payload)
  const rawItems = body.items
  const items = Array.isArray(rawItems) ? rawItems : []
  const sessions = items.map((s) => toSession(s)).filter((s): s is Session => Boolean(s))
  const page = asRecord(body.page)
  const nextCursor = typeof page.next_cursor === 'string' ? (page.next_cursor as string) : null
  return {
    sessions,
    total: typeof body.total === 'number' ? Number(body.total) : undefined,
    hasMore: page.has_more === true,
    nextCursor,
  }
}

export type CreateSessionInput = {
  workspaceId: number
  title: string
  parentId?: number
}

export function buildCreateSessionRequest(input: CreateSessionInput): JsonObject {
  const workspaceId = Number(input.workspaceId)
  const title = String(input.title || '').trim()
  if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) throw new Error('A valid workspace id is required')
  if (!title) throw new Error('A session title is required')

  const payload: JsonObject = { workspace_id: workspaceId, title }
  if (typeof input.parentId === 'number' && Number.isSafeInteger(input.parentId) && input.parentId > 0) {
    payload.parent_id = input.parentId
  }
  return payload
}

/** POST /api/v1/sessions — create a session in a concrete server workspace. */
export async function createSession(input: CreateSessionInput): Promise<Session> {
  const payload = buildCreateSessionRequest(input)
  const created = await apiJson<JsonValue>('/api/v1/sessions', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  })
  const session = toSession(created)
  if (!session) throw new Error('Server did not return a session')
  return session
}

/** POST /api/v1/workspaces/resolve — resolve a path and create its workspace when needed. */
export async function resolveWorkspace(path: string): Promise<{ id: number; path: string }> {
  const workspacePath = String(path || '').trim()
  if (!workspacePath) throw new Error('A workspace path is required')
  const payload = await apiJson<JsonValue>('/api/v1/workspaces/resolve', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path: workspacePath, create_if_missing: true }),
  })
  const record = asRecord(payload)
  const id = record.id
  const resolvedPath = str(record.path)
  if (typeof id !== 'number' || !Number.isSafeInteger(id) || id <= 0 || !resolvedPath) {
    throw new Error('Server did not return a workspace')
  }
  return { id, path: resolvedPath }
}

/** GET /api/v1/workspaces/{id} — read the authoritative workspace path. */
export async function getWorkspace(workspaceId: number): Promise<{ id: number; path: string }> {
  if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) throw new Error('A valid workspace id is required')
  const payload = await apiJson<JsonValue>(`/api/v1/workspaces/${encodeURIComponent(String(workspaceId))}`)
  const record = asRecord(payload)
  const id = record.id
  const path = str(record.path)
  if (typeof id !== 'number' || !Number.isSafeInteger(id) || id <= 0 || !path) {
    throw new Error('Server did not return a workspace')
  }
  return { id, path }
}

/** GET /api/v1/runtime — resolve the server's active workspace for first-run clients. */
export async function getRuntimeWorkspaceRoot(): Promise<string> {
  const payload = await apiJson<JsonValue>('/api/v1/runtime')
  const root = str(asRecord(payload).workspace_root)
  if (!root) throw new Error('Server did not report a workspace root')
  return root
}

export type WorkspaceFileUpload = {
  workspace_id: number
  path: string
  name: string
  mime?: string | null
  size_bytes: number
}

/** POST /api/v1/workspaces/{id}/files — upload a composer attachment. */
export async function uploadWorkspaceFile(
  workspaceId: number,
  input: { filename: string; dataBase64: string; mime?: string },
): Promise<WorkspaceFileUpload> {
  if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) throw new Error('A valid workspace id is required')
  const filename = String(input.filename || '').trim()
  const dataBase64 = String(input.dataBase64 || '').trim()
  if (!filename || !dataBase64) throw new Error('Attachment filename and data are required')
  return await apiJson<WorkspaceFileUpload>(`/api/v1/workspaces/${encodeURIComponent(String(workspaceId))}/files`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      filename,
      data_base64: dataBase64,
      ...(String(input.mime || '').trim() ? { mime: String(input.mime).trim() } : {}),
    }),
  })
}

/** POST /api/v1/workspaces — create a workspace (project) by path. Returns the new workspace id. */
export async function createWorkspace(path: string): Promise<number> {
  const created = await apiJson<JsonValue>('/api/v1/workspaces', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path }),
  })
  const record = asRecord(created)
  const id = record?.id
  if (typeof id !== 'number' || !Number.isFinite(id)) {
    throw new Error('Server did not return a workspace id')
  }
  return id
}

/** DELETE /api/v1/workspaces/{id} — remove a workspace (project). */
export async function deleteWorkspace(workspaceId: string): Promise<void> {
  const wid = String(workspaceId || '').trim()
  if (!wid) throw new Error('Missing workspace id')
  await apiJson(`/api/v1/workspaces/${encodeURIComponent(wid)}`, { method: 'DELETE' })
}

/** DELETE /api/v1/sessions/{id} */
export async function deleteSession(sessionId: string): Promise<void> {
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}`, { method: 'DELETE' })
}

/** PUT /api/v1/sessions/{id} — atomically update user-editable metadata. */
export async function patchSessionMetadata(
  sessionId: string,
  patch: { title?: string; favorite?: boolean; pinned?: boolean },
): Promise<Session> {
  const updated = await apiJson<JsonValue>(`/api/v1/sessions/${encodeURIComponent(sessionId)}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(patch),
  })
  const session = toSession(updated)
  if (!session) throw new Error('Server did not return a session')
  return session
}

export async function patchSessionTitle(sessionId: string, title: string): Promise<Session> {
  return patchSessionMetadata(sessionId, { title })
}

/** GET /api/v1/sessions/{id} — single session read-back (replaces opencode locateSession). */
export async function getSession(sessionId: string): Promise<Session> {
  const session = toSession(await apiJson<JsonValue>(`/api/v1/sessions/${encodeURIComponent(sessionId)}`))
  if (!session) throw new Error('Server did not return a session')
  return session
}

/** GET /api/v1/sessions/{id}/state — execution + parts + tagged session state. */
export async function getSessionExecution(sessionId: string): Promise<AgenaExecutionState> {
  const raw = await apiJson<AgenaExecutionState>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/state`)
  return {
    ...raw,
    session: {
      ...raw.session,
      state: normalizeSessionState(raw.session?.state),
    },
  }
}

/** GET /api/v1/sessions/{id}/parts — ordered part snapshot (reconnect catch-up). */
export async function getSessionParts(
  sessionId: string,
  limit?: number,
  cursor?: string | null,
): Promise<AgenaSessionParts> {
  const params = new URLSearchParams()
  if (typeof limit === 'number' && Number.isFinite(limit)) params.set('limit', String(Math.floor(limit)))
  if (typeof cursor === 'string' && cursor.trim()) params.set('cursor', cursor.trim())
  const suffix = params.toString() ? `?${params.toString()}` : ''
  return await apiJson<AgenaSessionParts>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/parts${suffix}`)
}

/**
 * Load a session's collapsed transcript as MessageEntry[].
 * The server walks raw `/parts` pages internally and returns only visible
 * tails plus fold cursors; hidden activity never crosses the HTTP boundary.
 */
export async function listMessages(
  sessionId: string,
  limit: number,
  cursor?: string | null,
): Promise<MessageListResponse> {
  const sid = String(sessionId || '').trim()
  if (!sid) return { entries: [], hasMore: false, nextCursor: null }
  const params = new URLSearchParams()
  params.set('limit', String(Math.max(1, Math.min(12, Math.floor(limit || 8)))))
  if (typeof cursor === 'string' && cursor.trim()) params.set('cursor', cursor.trim())
  const parts = await apiJson<AgenaSessionParts>(
    `/api/v1/sessions/${encodeURIComponent(sid)}/transcript?${params.toString()}`,
  )
  const folds = messageFoldsFromWire(parts.folds)
  return {
    entries: entriesFromParts(sid, parts.parts as unknown as JsonValue[], folds),
    hasMore: Boolean(parts.page?.has_more),
    nextCursor: parts.page?.next_cursor ?? null,
  }
}

/** GET /api/v1/sessions/{id}/transcript/runs/{run_id} — one expansion chunk. */
export async function listTranscriptRunParts(
  sessionId: string,
  runId: number,
  limit: number,
  cursor?: string | null,
): Promise<MessageListResponse> {
  const sid = String(sessionId || '').trim()
  if (!sid || !Number.isFinite(runId)) return { entries: [], hasMore: false, nextCursor: null }
  const params = new URLSearchParams()
  params.set('limit', String(Math.max(1, Math.min(50, Math.floor(limit || 5)))))
  if (typeof cursor === 'string' && cursor.trim()) params.set('cursor', cursor.trim())
  const parts = await apiJson<AgenaSessionParts>(
    `/api/v1/sessions/${encodeURIComponent(sid)}/transcript/runs/${encodeURIComponent(String(runId))}?${params.toString()}`,
  )
  return {
    entries: entriesFromParts(sid, parts.parts as unknown as JsonValue[], [], [runId]),
    hasMore: Boolean(parts.page?.has_more),
    nextCursor: parts.page?.next_cursor ?? null,
  }
}

/** GET /api/v1/sessions/{id}/transcript/folds — one expansion chunk. */
export async function listTranscriptFoldParts(
  sessionId: string,
  runIds: number[],
  limit: number,
  cursor?: string | null,
): Promise<MessageListResponse> {
  const sid = String(sessionId || '').trim()
  const ids = runIds.filter((runId) => Number.isFinite(runId))
  if (!sid || !ids.length) return { entries: [], hasMore: false, nextCursor: null }
  const params = new URLSearchParams()
  params.set('limit', String(Math.max(1, Math.min(50, Math.floor(limit || 5)))))
  if (typeof cursor === 'string' && cursor.trim()) params.set('cursor', cursor.trim())
  const parts = await apiJson<AgenaSessionParts>(
    `/api/v1/sessions/${encodeURIComponent(sid)}/transcript/folds?${params.toString()}`,
  )
  return {
    entries: entriesFromParts(sid, parts.parts as unknown as JsonValue[], [], ids),
    hasMore: Boolean(parts.page?.has_more),
    nextCursor: parts.page?.next_cursor ?? null,
  }
}

export type SendMessagePayload = RunOptionsPayload & {
  document: JsonValue[]
}

export function buildRunRequestBody(options?: RunOptionsPayload): JsonObject {
  const source = options || {}
  const body: JsonObject = {}
  const model = source.model
  if (model) {
    const providerId = String(model.provider_id || '').trim()
    const adapterId = String(model.adapter_id || '').trim()
    const modelId = String(model.model_id || '').trim()
    if (providerId && modelId) {
      body.model = {
        provider_id: providerId,
        ...(adapterId ? { adapter_id: adapterId } : {}),
        model_id: modelId,
      }
    }
  }

  for (const key of ['thinking_mode', 'speed_mode', 'verbosity', 'system'] as const) {
    const value = source[key]
    if (typeof value === 'string' && value.trim()) body[key] = key === 'system' ? value : value.trim()
  }
  if (typeof source.parallel_tool_calls === 'boolean') body.parallel_tool_calls = source.parallel_tool_calls
  if (typeof source.temperature === 'number' && Number.isFinite(source.temperature))
    body.temperature = source.temperature
  if (
    typeof source.max_output_tokens === 'number' &&
    Number.isSafeInteger(source.max_output_tokens) &&
    source.max_output_tokens > 0
  ) {
    body.max_output_tokens = source.max_output_tokens
  }
  return body
}

export function buildMessageRequestBody(payload: SendMessagePayload): JsonObject {
  return {
    ...buildRunRequestBody(payload),
    document: Array.isArray(payload.document) ? payload.document : [],
  }
}

/** POST /api/v1/sessions/{id}/messages — submit a composer document + run options. */
export async function sendMessage(sessionId: string, payload: SendMessagePayload): Promise<SendMessageResponse> {
  const resp = await apiJson<JsonValue>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/messages`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(buildMessageRequestBody(payload)),
  })
  const body = asRecord(resp)
  return { queued: body.queued === true }
}

/** POST /api/v1/sessions/{id}/continue — resume a paused/interrupted run. */
export async function continueSession(sessionId: string, options?: RunOptionsPayload): Promise<void> {
  const body = buildRunRequestBody(options)
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/continue`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

/** POST /api/v1/sessions/{id}/compact — context compaction. */
export async function compactSession(sessionId: string, options?: RunOptionsPayload): Promise<void> {
  const body = buildRunRequestBody(options)
  await apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/compact`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

/** POST /api/v1/sessions/{id}/cancel — abort an active execution. */
export async function cancelSession(sessionId: string, executionId?: string | null): Promise<CancellationOutcome> {
  const response = await apiJson<JsonValue>(`/api/v1/sessions/${encodeURIComponent(sessionId)}/cancel`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ execution_id: executionId || null }),
  })
  const body = asRecord(response)
  const rawResult = str(body.result)
  const result: CancellationResult =
    rawResult === 'already_terminal' ||
    rawResult === 'not_found' ||
    rawResult === 'execution_mismatch' ||
    rawResult === 'cancellation_requested'
      ? rawResult
      : 'not_found'
  return {
    result,
    ...(Object.prototype.hasOwnProperty.call(body, 'restored_user_message')
      ? { restored_user_message: body.restored_user_message }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(body, 'restored_user_run_id')
      ? {
          restored_user_run_id:
            typeof body.restored_user_run_id === 'number' && Number.isFinite(body.restored_user_run_id)
              ? body.restored_user_run_id
              : null,
        }
      : {}),
  }
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
export async function forkSession(
  sessionId: string,
  opts?: { at_message_id?: number; title?: string },
): Promise<Session> {
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
  reply: 'once' | 'always' | 'reject' | 'reject_always',
  message?: string,
): Promise<boolean> {
  const kind =
    reply === 'once'
      ? 'allow_once'
      : reply === 'always'
        ? 'allow_always'
        : reply === 'reject_always'
          ? 'deny_always'
          : 'deny_once'
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

/** POST /api/v1/sessions/{id}/interactive/{request_id}/present — presentation ack. */
export function interactivePresentationPath(sessionId: string, requestId: string): string {
  return `/api/v1/sessions/${encodeURIComponent(sessionId)}/interactive/${encodeURIComponent(requestId)}/present`
}

export async function presentInteractiveRequest(sessionId: string, requestId: string): Promise<void> {
  try {
    await apiJson(interactivePresentationPath(sessionId, requestId), { method: 'POST' })
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
    const raw = await apiJson<AgenaExecutionState>(
      `/api/v1/sessions/${encodeURIComponent(sid)}/state?include_parts=false`,
    )
    const state = {
      ...raw,
      session: {
        ...raw.session,
        state: normalizeSessionState(raw.session?.state),
      },
    }
    const s = normalizeSessionState(state.session?.state)
    return {
      state: s,
      execution: state.execution,
      usage: state.usage,
    }
  } catch {
    return null
  }
}

// Re-export normalization helpers for other modules.
export { entriesFromParts }

// Shared normalizeSessionDiffPayload / SessionFileDiff types were removed:
// agena has no per-session diff endpoint. (See MISSING endpoints in report.)
