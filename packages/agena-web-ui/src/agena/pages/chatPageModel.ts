import type { SessionChange, SessionExecutionResource, SessionPart } from '../lib/agenaApi'

export type ChatPartsState = {
  parts: SessionPart[]
  sessionState: SessionExecutionResource | null
  selectedSessionId: number | null
}

function isTerminalState(state: SessionPart['state']): boolean {
  return !['pending', 'in_progress'].includes(state)
}

/** Canonical ordering basis: created_at_ms, then part_id (4.2). */
export function compareSessionParts(left: SessionPart, right: SessionPart): number {
  const byTime = left.created_at_ms - right.created_at_ms
  if (byTime !== 0) return byTime
  return left.part_id - right.part_id
}

export function upsertSessionPart(parts: SessionPart[], incoming: SessionPart): SessionPart[] {
  const index = parts.findIndex((part) => part.part_id === incoming.part_id)
  if (index < 0) {
    return [...parts, incoming].sort(compareSessionParts)
  }
  const current = parts[index]!
  // Streaming deltas must never regress a terminal part back to an open state.
  const state = isTerminalState(current.state) && !isTerminalState(incoming.state) ? current.state : incoming.state
  const merged: SessionPart = { ...current, ...incoming, state }
  const next = [...parts]
  next[index] = merged
  return next
}

/**
 * Reduce a single v2 `SessionChange` notification into the local parts array.
 * `PartAdded`/`PartUpdated` patch in place by `part_id`; `PartRemoved` deletes;
 * `SessionMetaUpdated` carries no part data, so it asks the caller to refresh
 * session metadata.
 */
export function applySessionChange(
  state: ChatPartsState,
  change: SessionChange,
): { state: ChatPartsState; shouldRefresh: boolean } {
  switch (change.type) {
    case 'PartAdded':
      return {
        state: { ...state, parts: upsertSessionPart(state.parts, change.part) },
        shouldRefresh: false,
      }
    case 'PartUpdated':
      return {
        state: { ...state, parts: upsertSessionPart(state.parts, change.part) },
        shouldRefresh: false,
      }
    case 'PartRemoved':
      return {
        state: { ...state, parts: state.parts.filter((part) => part.part_id !== change.part_id) },
        shouldRefresh: false,
      }
    case 'SessionMetaUpdated':
      return { state, shouldRefresh: true }
  }
}
