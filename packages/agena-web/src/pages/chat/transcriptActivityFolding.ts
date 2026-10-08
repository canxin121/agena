import type { MessageFold } from '../../types/chat'

export type ActivityVisibility = { count?: number; ids: string[]; keepOpen?: boolean; pendingRequestIds?: string[] }

export type TranscriptActivityFold<T> = {
  hiddenCount: number
  visibleParts: T[]
}

/**
 * Fold an activity run strictly by chronological position and part count.
 * Individual activity expansion controls only that activity's body; it must
 * never pin an old activity outside the run's collapsed prefix.
 */
export function foldTranscriptActivityRun<T>(parts: readonly T[], visibleCount = 5): TranscriptActivityFold<T> {
  const budget = Number.isFinite(visibleCount) ? Math.max(0, Math.floor(visibleCount)) : 5
  const hiddenCount = Math.max(0, parts.length - budget)
  return {
    hiddenCount,
    visibleParts: parts.slice(hiddenCount),
  }
}

export function foldTranscriptReply<T extends { kind: string }>(
  parts: readonly T[],
  folds: readonly MessageFold[],
  visibleCount = 5,
  keepVisible?: (part: T) => boolean,
): TranscriptActivityFold<T> & { fold: MessageFold | null } {
  const fold = folds.find((item) => item.hiddenCount > 0 && item.nextCursor) || null
  const contentParts = parts.filter((part) => part.kind !== 'lifecycle')
  const requiredIndex = keepVisible ? contentParts.findIndex(keepVisible) : -1
  const budget = requiredIndex < 0 ? visibleCount : Math.max(visibleCount, contentParts.length - requiredIndex)
  const content = foldTranscriptActivityRun(contentParts, budget)
  return {
    fold,
    hiddenCount: (fold?.hiddenCount || 0) + content.hiddenCount,
    visibleParts: [...content.visibleParts, ...parts.filter((part) => part.kind === 'lifecycle')],
  }
}

export function preserveActivityVisibility(next: string[], previous: string[], count: number): number {
  if (!previous.length) return count
  const tail = next.indexOf(previous[previous.length - 1]!)
  return tail < 0 ? count : count + next.length - tail - 1
}

/**
 * The reply identity survives both prepending history and streaming new
 * parts. Neither end of its changing part window owns expansion state.
 */
export function transcriptActivityRunKey(
  messageId: string,
  _parts: readonly { id?: unknown }[],
  _fallback: string | number,
): string {
  return `activity-summary:${messageId}`
}
