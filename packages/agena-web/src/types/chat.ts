import type { SseEvent } from '../lib/sse'
import type { JsonValue as JsonLike } from './json'

// ---------------------------------------------------------------------------
// Agena wire types (mirror crates/agena-api/src/resource.rs + live.rs).
//
// The server talks /api/v1 with numeric session/part ids and RFC3339 UTC
// timestamps. Sessions are flat (no frontend directory concept): the server
// owns the workspace. `Session` keeps an open index signature so every field
// survives round-trips.
// ---------------------------------------------------------------------------

export type SessionWorkflowState = 'quiescent' | 'tool_pending' | 'awaiting_interaction'

export type SessionExecutionSnapshot = {
  execution_id: string
  phase: string
}

export type SessionPendingInteraction = JsonLike

/** Canonical server-owned session state. Clients branch on `kind`. */
export type SessionState =
  | { kind: 'creating' }
  | { kind: 'ready'; data: { last_failure?: JsonLike } }
  | {
      kind: 'running'
      data: {
        execution?: SessionExecutionSnapshot
        workflow: SessionWorkflowState
        requests?: SessionPendingInteraction[]
      }
    }
  | {
      kind: 'awaiting_interaction'
      data: {
        run_id?: number
        execution?: SessionExecutionSnapshot
        requests?: SessionPendingInteraction[]
      }
    }
  | {
      kind: 'interrupted'
      data: {
        run_id?: number
        reason?: string
        last_failure?: JsonLike
      }
    }
  | { kind: 'failed'; data: { failure?: JsonLike } }

export type SessionStateKind = SessionState['kind']

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function jsonObject(value: unknown): Record<string, JsonLike> {
  return isRecord(value) ? (value as Record<string, JsonLike>) : {}
}

function executionSnapshot(value: unknown): SessionExecutionSnapshot | undefined {
  if (!isRecord(value)) return undefined
  const executionId = typeof value.execution_id === 'string' ? value.execution_id.trim() : ''
  const phase = typeof value.phase === 'string' ? value.phase : ''
  if (!executionId || !phase) return undefined
  return { execution_id: executionId, phase }
}

/** Normalize the tagged wire value once at the API boundary. */
export function normalizeSessionState(value: unknown): SessionState {
  if (!isRecord(value) || typeof value.kind !== 'string') {
    return { kind: 'ready', data: {} }
  }

  const data = jsonObject(value.data)
  switch (value.kind) {
    case 'creating':
      return { kind: 'creating' }
    case 'ready':
      return { kind: 'ready', data }
    case 'running': {
      const workflow =
        data.workflow === 'tool_pending' || data.workflow === 'awaiting_interaction' ? data.workflow : 'quiescent'
      const requests = Array.isArray(data.requests) ? data.requests : undefined
      return {
        kind: 'running',
        data: {
          workflow,
          ...(executionSnapshot(data.execution) ? { execution: executionSnapshot(data.execution) } : {}),
          ...(requests ? { requests } : {}),
        },
      }
    }
    case 'awaiting_interaction': {
      const requests = Array.isArray(data.requests) ? data.requests : undefined
      return {
        kind: 'awaiting_interaction',
        data: {
          ...(typeof data.run_id === 'number' ? { run_id: data.run_id } : {}),
          ...(executionSnapshot(data.execution) ? { execution: executionSnapshot(data.execution) } : {}),
          ...(requests ? { requests } : {}),
        },
      }
    }
    case 'interrupted':
      return {
        kind: 'interrupted',
        data: {
          ...(typeof data.run_id === 'number' ? { run_id: data.run_id } : {}),
          ...(typeof data.reason === 'string' ? { reason: data.reason } : {}),
          ...(Object.prototype.hasOwnProperty.call(data, 'last_failure') ? { last_failure: data.last_failure } : {}),
        },
      }
    case 'failed':
      return {
        kind: 'failed',
        data: Object.prototype.hasOwnProperty.call(data, 'failure') ? { failure: data.failure } : {},
      }
    default:
      return { kind: 'ready', data: {} }
  }
}

export function sessionStateKind(state: SessionState | null | undefined): SessionStateKind {
  return state?.kind || 'ready'
}

export function sessionStateData(state: SessionState | null | undefined): Record<string, JsonLike> {
  return state && state.kind !== 'creating' ? (state.data as Record<string, JsonLike>) : {}
}

export function sessionStateExecution(state: SessionState | null | undefined): SessionExecutionSnapshot | undefined {
  const data = sessionStateData(state)
  return executionSnapshot(data.execution)
}

export function sessionStateRequests(state: SessionState | null | undefined): SessionPendingInteraction[] {
  const data = sessionStateData(state)
  return Array.isArray(data.requests) ? data.requests : []
}

export function sessionStateIsBusy(state: SessionState | null | undefined): boolean {
  return sessionStateKind(state) === 'running'
}

export function sessionStateNeedsAttention(state: SessionState | null | undefined): boolean {
  const kind = sessionStateKind(state)
  return (
    kind === 'awaiting_interaction' ||
    kind === 'interrupted' ||
    kind === 'failed' ||
    (kind === 'running' && sessionStateRequests(state).length > 0)
  )
}

export function sessionStateNeedsRecovery(state: SessionState | null | undefined): boolean {
  return sessionStateKind(state) === 'interrupted'
}

export type SessionRelationKind = 'root' | 'child' | 'fork' | 'rewind' | 'subagent'

export type Session = {
  id: string
  title?: string
  // Agena fields (kept on the open signature as well; listed for docs).
  state?: SessionState
  relation_kind?: SessionRelationKind
  favorite?: boolean
  pinned?: boolean
  version?: number
  message_count?: number
  child_session_count?: number
  created_at?: string
  updated_at?: string
  last_message_at?: string | null
  workspace_id?: number
  [k: string]: JsonLike
}

// Agena part execution states → tool status mapping in reducers.ts.
export type PartState =
  | 'pending'
  | 'in_progress'
  | 'completed'
  | 'policy_denied'
  | 'user_declined'
  | 'capability_unavailable'
  | 'tool_unavailable'
  | 'failed'
  | 'cancelled'

export type MessageInfo = {
  id: string
  sessionID: string
  role: 'user' | 'assistant' | 'system' | 'tool' | 'runtime' | string
  time?: { created?: number; completed?: number }
  finish?: string
  error?: MessageError
  modelID?: string
  providerID?: string
  adapterID?: string
  // Durable numeric ids that back this message (agena run marker).
  runId?: number
  // Preserve the run marker's durable state/content. Transcript projection
  // uses these fields for TUI-parity lifecycle chrome and assistant-run
  // folding instead of inferring state from the presence of text.
  runState?: string
  runContent?: JsonLike
  [k: string]: JsonLike
}

export type MessageError = {
  name?: string
  type?: string
  message?: string
  code?: string
  classification?: string
  [k: string]: JsonLike
}

export type MessagePart = {
  id: string
  sessionID: string
  messageID: string
  type: string
  text?: string
  // Agena part state (string wire value, e.g. "completed").
  partState?: string
  // Lossless Agena transcript identity. The frontend presentation layer must
  // be able to project the same open-set part kinds as the TUI; flattening a
  // tool call down to {tool,input,output} discards operation sections,
  // interaction records, attachments, lifecycle, and future part kinds.
  agenaKind?: string
  agenaRole?: string
  agenaSummary?: string | null
  agenaContent?: JsonLike
  runId?: number | null
  parentPartId?: number | null
  // For tool parts (ToolInvocation.vue contract).
  tool?: string
  state?: JsonLike
  metadata?: JsonLike
  time?: { start?: number; end?: number }
  // For file/attachment parts.
  url?: string
  filename?: string
  mime?: string
  [k: string]: JsonLike
}

export type MessageEntry = {
  info: MessageInfo
  parts: MessagePart[]
}

export type AttentionEvent = {
  kind: 'permission' | 'question'
  at: number
  payload: SseEvent
}

export type SessionErrorClassification = 'context_overflow' | 'provider_auth' | 'network' | 'provider_api' | 'unknown'

export type SessionError = {
  message: string
  rendered?: string
  code?: string
  name?: string
  classification?: SessionErrorClassification
  raw: JsonLike
}

export type SessionErrorEvent = {
  at: number
  payload: SseEvent
  error: SessionError
}

export type SessionRunConfig = {
  providerID?: string
  adapterID?: string
  modelID?: string
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
  parallelToolCalls?: boolean
  at: number
}

export type SessionUsage = {
  measured_prompt_tokens?: number | null
  current_tokens?: number
  projected_tokens?: number | null
  limit_tokens?: number | null
  limit_basis?: string | null
  reserved_tokens?: number | null
  model_context_window_tokens?: number | null
  model_max_input_tokens?: number | null
  model_max_output_tokens?: number | null
}

// ---------------------------------------------------------------------------
// Pending-interactive requests (permission / user input) are carried by
// `session.state.data.requests` inside SessionExecutionResource. A
// `runtime_signal` only tells the client to refresh that canonical state.
// ---------------------------------------------------------------------------

export type AgenaPermissionRequest = {
  request_id: string
  session_id?: number
  action?: { kind?: string; tool_name?: string; target_path?: string; target?: string; [k: string]: JsonLike }
  reason?: string
  explanation?: string
  scope?: string
  [k: string]: JsonLike
}

export type AgenaUserInputRequest = {
  request_id: string
  session_id?: number
  title?: string
  body_markdown?: string
  input_kind?: string
  questions?: Array<{ question_id?: string; title?: string; options?: JsonLike; [k: string]: JsonLike }>
  [k: string]: JsonLike
}

export type AgenaPendingInteractiveRequest = {
  session_id?: number
  kind?: 'permission' | 'user_input'
  request_id?: string
  // flatten: permission fields
  action?: AgenaPermissionRequest['action']
  reason?: string
  explanation?: string
  scope?: string
  // flatten: user-input fields
  title?: string
  body_markdown?: string
  input_kind?: string
  questions?: AgenaUserInputRequest['questions']
  [k: string]: JsonLike
}
