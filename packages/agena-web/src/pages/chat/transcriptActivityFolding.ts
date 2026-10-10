import type { MessageFold } from '../../types/chat'

/**
 * One message's part window.
 *
 * `count` is how many parts the window shows. `undefined` is the default
 * collapsed state: the message follows its newest parts, so every part a live
 * reply appends pushes the oldest visible one back into the collapsed prefix.
 * A number means the user asked for more through the message's own load
 * controls; `keepOpen` then keeps the rows that were revealed visible while
 * newer parts append, until the message is collapsed again.
 *
 * `ids` mirrors the last projection of the message so a revealed window can be
 * shifted when parts are prepended or appended. `pendingRequestIds` tracks
 * outstanding user requests, whose parts stay visible in any state.
 */
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
 * The part window a message keeps while its parts change (streaming, reconnect,
 * prepended history).
 *
 * A message that follows its newest parts keeps following them: an undefined
 * count stays undefined, so the window is a pure suffix and every new part
 * pushes the oldest visible one back into the collapsed prefix. Only a count the
 * user set through the message's own load controls pins rows, and then only the
 * rows that were visible stay visible while newer parts append.
 *
 * Nothing else may change the window: selecting a row, navigating the
 * transcript, or expanding a single part body never turns a collapsed reply
 * into a wall of parts.
 */
export function nextActivityWindowCount(
  state: { count?: number; ids: readonly string[] },
  nextIds: readonly string[],
): number | undefined {
  if (state.count === undefined) return undefined
  return preserveActivityVisibility([...nextIds], [...state.ids], state.count)
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
