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

/**
 * Older expansion pages are prepended, so the newest id is the stable run
 * anchor. A server fold's oldest-visible anchor changes after every request
 * and must never be used as local visibility state.
 */
export function transcriptActivityRunKey(
  messageId: string,
  parts: readonly { id?: unknown }[],
  fallback: string | number,
): string {
  return `activity-summary:${messageId}:${String(parts.at(-1)?.id || fallback)}`
}
