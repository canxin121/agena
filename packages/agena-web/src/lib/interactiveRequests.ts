import { isRunInFlight } from './chatRunState'
import type { JsonObject } from '../types/json'

export type InteractiveRequestKind = 'permission' | 'user_input'
export type InteractiveRequest = {
  kind: InteractiveRequestKind
  requestId: string
  sessionId: string
  request: JsonObject
}

function record(value: unknown): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as JsonObject) : {}
}

/** The request id is opaque; its owner and reply family come from typed facts. */
export function interactiveRequests(requests: unknown[], fallbackSessionId: string): InteractiveRequest[] {
  return requests.flatMap((raw) => {
    const request = record(raw)
    const kind = request.kind
    const requestId = typeof request.request_id === 'string' ? request.request_id.trim() : ''
    if (!requestId || (kind !== 'permission' && kind !== 'user_input')) return []
    const sessionId = typeof request.session_id === 'number' ? String(request.session_id) : fallbackSessionId
    return [{ kind, requestId, sessionId, request }]
  })
}

type InteractionPart = {
  agenaKind?: unknown
  agenaContent?: unknown
  partState?: unknown
  originSessionId?: unknown
  sessionID?: unknown
}

export function partOwnsInteractions(part: InteractionPart): boolean {
  return part.originSessionId == null || part.sessionID == null || String(part.originSessionId) === part.sessionID
}

/** Same eligibility as the runtime: an owned, in-flight tool with an unanswered record. */
export function pendingPartInteractions(part: InteractionPart): InteractiveRequest[] {
  if (part.agenaKind !== 'tool_call' || !isRunInFlight(part.partState) || !partOwnsInteractions(part)) return []
  const content = record(part.agenaContent)
  return (['permission', 'user_input'] as const).flatMap((kind) => {
    const bucket = record(kind === 'permission' ? content.authorization : content.user_input)
    const entries = kind === 'permission' ? bucket.permissions : bucket.requests
    return (Array.isArray(entries) ? entries : []).flatMap((raw) => {
      const entry = record(raw)
      if (entry.reply != null) return []
      return interactiveRequests([{ ...record(entry.request), kind }], String(part.sessionID || ''))
    })
  })
}
