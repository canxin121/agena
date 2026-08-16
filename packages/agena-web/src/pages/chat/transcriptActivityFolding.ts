export type TranscriptActivityFold<T> = {
  hiddenCount: number
  visibleParts: T[]
}

/**
 * Fold an activity run strictly by chronological position and part count.
 * Individual activity expansion controls only that activity's body; it must
 * never pin an old activity outside the run's collapsed prefix.
 */
export function foldTranscriptActivityRun<T>(
  parts: readonly T[],
  summaryExpanded: boolean,
  visibleCount = 5,
): TranscriptActivityFold<T> {
  const budget = Number.isFinite(visibleCount) ? Math.max(0, Math.floor(visibleCount)) : 5
  const hiddenCount = Math.max(0, parts.length - budget)
  return {
    hiddenCount,
    visibleParts: summaryExpanded ? [...parts] : parts.slice(hiddenCount),
  }
}
