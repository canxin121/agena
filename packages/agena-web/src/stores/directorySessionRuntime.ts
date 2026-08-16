import type { SessionState } from '../types/chat'
import {
  normalizeSessionState,
  sessionStateIsBusy,
  sessionStateKind,
  sessionStateNeedsAttention,
  sessionStateNeedsRecovery,
} from '../types/chat'

/**
 * The directory store only keeps the canonical server state and the server
 * timestamp used to order snapshots. It deliberately has no second
 * status/phase/display-state state machine.
 */
export type SessionStateSnapshot = {
  state: SessionState
  updatedAt: number
}

type StateInput =
  | SessionState
  | SessionStateSnapshot
  | { state?: unknown; updatedAt?: unknown; updated_at?: unknown }
  | null
  | undefined

function updatedAtFrom(input: StateInput): number {
  if (!input || typeof input !== 'object') return 0
  const value = input as { updatedAt?: unknown; updated_at?: unknown }
  if (typeof value.updatedAt === 'number' && Number.isFinite(value.updatedAt)) {
    return Math.max(0, Math.floor(value.updatedAt))
  }
  if (typeof value.updated_at === 'number' && Number.isFinite(value.updated_at)) {
    return Math.max(0, Math.floor(value.updated_at))
  }
  if (typeof value.updated_at === 'string') {
    const parsed = Date.parse(value.updated_at)
    return Number.isFinite(parsed) ? parsed : 0
  }
  return 0
}

function stateFrom(input: StateInput): SessionState {
  if (!input || typeof input !== 'object') return normalizeSessionState(null)
  if (typeof (input as SessionState).kind === 'string') return normalizeSessionState(input)
  return normalizeSessionState((input as { state?: unknown }).state)
}

export function normalizeSessionStateSnapshot(input?: StateInput): SessionStateSnapshot {
  return { state: stateFrom(input), updatedAt: updatedAtFrom(input) }
}

export function stateSnapshotFromAgenaSession(session: Record<string, unknown>): SessionStateSnapshot {
  return normalizeSessionStateSnapshot({
    state: session.state,
    updated_at: session.updated_at,
  })
}

export function sessionStateKindOf(snapshot?: SessionStateSnapshot | null) {
  return sessionStateKind(snapshot?.state)
}

export function sessionStateIsActive(snapshot?: SessionStateSnapshot | null): boolean {
  const kind = sessionStateKindOf(snapshot)
  return kind === 'creating' || sessionStateIsBusy(snapshot?.state)
}

export function sessionStateHasAttention(snapshot?: SessionStateSnapshot | null): boolean {
  return sessionStateNeedsAttention(snapshot?.state)
}

export function sessionStateNeedsRecoveryOf(snapshot?: SessionStateSnapshot | null): boolean {
  return sessionStateNeedsRecovery(snapshot?.state)
}

export function mergeSessionStateSnapshot(
  current: SessionStateSnapshot | undefined,
  incoming: StateInput,
): SessionStateSnapshot {
  const next = normalizeSessionStateSnapshot(incoming)
  if (!current) return next
  if (next.updatedAt <= 0 || next.updatedAt >= current.updatedAt) return next
  return current
}

export function stateSnapshotEquivalent(left: SessionStateSnapshot, right: SessionStateSnapshot): boolean {
  return left.updatedAt === right.updatedAt && JSON.stringify(left.state) === JSON.stringify(right.state)
}
