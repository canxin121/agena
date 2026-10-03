import type { SseEvent } from '../lib/sse'
import type { JsonValue as JsonLike } from './json'
import {
  EXECUTION_PHASES,
  SESSION_STATE_ATTENTION_KINDS,
  SESSION_STATE_BUSY_KINDS,
  SESSION_STATE_KINDS,
  WORKFLOW_STATES,
} from '../generated/agenaState'
import type {
  ActiveExecutionResource,
  ExecutionPhase,
  ExecutionStatus,
  SessionLifecycleState,
  SessionRelationKind,
  SessionState,
  SessionStateKind,
  SubtaskStatus,
  WorkflowState,
} from '../generated/agenaState'

// State types are generated from the backend definitions; the web client never
// keeps a hand-written copy of a state union. Regenerate the mirror with
// `cargo run -p agena-web-types > packages/agena-web/src/generated/agenaState.ts`.
// The mirror is imported relatively so the shipped parser stays loadable by the
// `node:test` contract suite without a bundler alias.
export type {
  ActiveExecutionResource,
  ExecutionPhase,
  ExecutionStatus,
  SessionLifecycleState,
  SessionRelationKind,
  SessionState,
  SessionStateKind,
  SubtaskStatus,
  WorkflowState,
}

// ---------------------------------------------------------------------------
// Agena wire types (mirror crates/agena-api/src/resource.rs + live.rs).
//
// The server talks /api/v1 with numeric session/part ids and RFC3339 UTC
// timestamps. Sessions are flat (no frontend directory concept): the server
// owns the workspace. `Session` keeps an open index signature so every field
// survives round-trips.
// ---------------------------------------------------------------------------

/** Live execution snapshot attached to a session state (generated type). */
export type SessionExecutionSnapshot = ActiveExecutionResource

export type SessionPendingInteraction = JsonLike

const SESSION_STATE_KIND_SET: readonly string[] = SESSION_STATE_KINDS
const SESSION_STATE_BUSY_SET: readonly string[] = SESSION_STATE_BUSY_KINDS
const SESSION_STATE_ATTENTION_SET: readonly string[] = SESSION_STATE_ATTENTION_KINDS
const EXECUTION_PHASE_SET: readonly string[] = EXECUTION_PHASES
const WORKFLOW_STATE_SET: readonly string[] = WORKFLOW_STATES

function hasOwn(record: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, key)
}

function isSessionStateKind(value: unknown): value is SessionStateKind {
  return typeof value === 'string' && SESSION_STATE_KIND_SET.includes(value)
}

function isExecutionPhase(value: unknown): value is ExecutionPhase {
  return typeof value === 'string' && EXECUTION_PHASE_SET.includes(value)
}

function isWorkflowState(value: unknown): value is WorkflowState {
  return typeof value === 'string' && WORKFLOW_STATE_SET.includes(value)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function jsonObject(value: unknown): Record<string, JsonLike> {
  return isRecord(value) ? (value as Record<string, JsonLike>) : {}
}

function executionSnapshot(value: unknown): SessionExecutionSnapshot | undefined {
  if (!isRecord(value)) return undefined
  const executionId = typeof value.execution_id === 'string' ? value.execution_id.trim() : ''
  const phase = value.phase
  if (!executionId || !isExecutionPhase(phase)) return undefined
  return { execution_id: executionId, phase }
}

/** Normalize the tagged wire value once at the API boundary. */
export function normalizeSessionState(value: unknown): SessionState {
  if (!isRecord(value) || !isSessionStateKind(value.kind)) {
    // Unknown kinds fall back to `ready`: the single documented fallback for a
    // payload this client version does not know yet.
    return { kind: 'ready', data: {} }
  }

  const data = jsonObject(value.data)
  const kind = value.kind
  switch (kind) {
    case 'creating':
      return { kind: 'creating' }
    case 'ready':
      return {
        kind: 'ready',
        data: hasOwn(data, 'last_failure') ? { last_failure: data.last_failure } : {},
      }
    case 'running': {
      const requests = Array.isArray(data.requests) ? (data.requests as JsonLike[]) : undefined
      const execution = executionSnapshot(data.execution)
      return {
        kind: 'running',
        data: {
          workflow: isWorkflowState(data.workflow) ? data.workflow : 'quiescent',
          ...(execution ? { execution } : {}),
          ...(requests ? { requests } : {}),
        },
      }
    }
    case 'awaiting_interaction': {
      const requests = Array.isArray(data.requests) ? (data.requests as JsonLike[]) : undefined
      const execution = executionSnapshot(data.execution)
      return {
        kind: 'awaiting_interaction',
        data: {
          ...(typeof data.run_id === 'number' ? { run_id: data.run_id } : {}),
          ...(execution ? { execution } : {}),
          ...(requests ? { requests } : {}),
        },
      }
    }
    case 'failed':
      return {
        kind: 'failed',
        data: hasOwn(data, 'failure') ? { failure: data.failure } : {},
      }
    default: {
      // Exhaustiveness guard: every generated kind is handled above, so a new
      // backend state kind fails to compile here instead of degrading silently.
      const unhandled: never = kind
      return unhandled
    }
  }
}

export function sessionStateKind(state: SessionState | null | undefined): SessionStateKind {
  return state?.kind || 'ready'
}

export function sessionStateData(state: SessionState | null | undefined): Record<string, JsonLike> {
  if (!state || state.kind === 'creating') return {}
  return state.data as Record<string, JsonLike>
}

export function sessionStateExecution(state: SessionState | null | undefined): SessionExecutionSnapshot | undefined {
  return executionSnapshot(sessionStateData(state).execution)
}

export function sessionStateRequests(state: SessionState | null | undefined): SessionPendingInteraction[] {
  const requests = sessionStateData(state).requests
  return Array.isArray(requests) ? (requests as SessionPendingInteraction[]) : []
}

export function sessionStateIsBusy(state: SessionState | null | undefined): boolean {
  return SESSION_STATE_BUSY_SET.includes(sessionStateKind(state))
}

export function sessionStateNeedsAttention(state: SessionState | null | undefined): boolean {
  const kind = sessionStateKind(state)
  return (
    SESSION_STATE_ATTENTION_SET.includes(kind) ||
    (SESSION_STATE_BUSY_SET.includes(kind) && sessionStateRequests(state).length > 0)
  )
}

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

/** Agena part execution states → tool status mapping in reducers.ts. */
export type PartState = ExecutionStatus

export type MessageInfo = {
  revision?: number
  updatedAt?: number
  id: string
  sessionID: string
  role: 'user' | 'assistant' | 'system' | 'tool' | 'runtime' | string
  time?: { created?: number; completed?: number }
  finish?: string
  error?: MessageError
  modelID?: string
  providerID?: string
  adapterID?: string
  // Conversation identities carried by the run marker content. The reply id
  // is the presentation identity of the reply lifecycle row, so a reply keeps
  // one id while it runs, fails, or is continued.
  turnId?: string
  replyId?: string
  // Durable numeric ids that back this message (agena run marker).
  runId?: number
  // 1-based ordinal of this message among the session's user-send messages,
  // ordered by the server's durable `(created_at_ms, part_id)` order. Present
  // only on user messages. Server-derived, so it is the message's true
  // absolute position in the conversation and never shifts with paging.
  userMessageOrdinal?: number
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
  revision?: number
  updatedAt?: number
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
  agenaPresentation?: JsonLike
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

export type MessageFold = {
  runId: number
  runIds: number[]
  anchorPartId: string
  hiddenCount: number
  nextCursor: string | null
}

export type MessageEntry = {
  info: MessageInfo
  parts: MessagePart[]
  folds?: MessageFold[]
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
