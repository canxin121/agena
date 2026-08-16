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

export type SessionState = 'creating' | 'ready' | 'running' | 'awaiting_user' | 'interrupted' | 'failed'

export type SessionRelationKind = 'root' | 'child' | 'fork' | 'rewind' | 'subagent'

export type Session = {
  id: string
  title?: string
  // Agena fields (kept on the open signature as well; listed for docs).
  state?: SessionState
  relation_kind?: SessionRelationKind
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
export type PartState = 'pending' | 'in_progress' | 'completed' | 'failed' | 'cancelled'

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

export type SessionStatus =
  | { type: 'idle' }
  | { type: 'busy' }
  | { type: 'retry'; attempt: number; message: string; next: number }

export type SessionStatusEvent = {
  at: number
  payload: SseEvent
  status: SessionStatus
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
// Agena pending-interactive-request projection (permission / user input).
// The server sends `pending_interactive_requests: [{session_id, request}]`
// inside SessionExecutionResource, plus a `runtime_signal` when one lands.
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
