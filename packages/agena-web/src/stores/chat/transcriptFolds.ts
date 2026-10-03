import type { MessageEntry, MessageFold } from '../../types/chat'
import { compareChatIds } from './messageIndex'

export function transcriptFoldKey(sessionId: string, fold: MessageFold): string {
  return `${sessionId}:${Math.min(fold.runId, ...fold.runIds)}`
}

function sharesRuns(left: MessageFold, right: MessageFold): boolean {
  const ids = new Set([left.runId, ...left.runIds])
  return [right.runId, ...right.runIds].some((id) => ids.has(id))
}

/** Reconcile a recent server tail with the older parts already in the cache. */
export function reconcileTranscriptFolds(
  merged: MessageEntry[],
  snapshot: MessageEntry[],
  previous: MessageEntry[],
): MessageEntry[] {
  const recentRuns = new Set(snapshot.map((entry) => Number(entry.info.runId ?? entry.info.id)))
  const incoming = snapshot.flatMap((entry) => entry.folds || [])
  const oldFolds = previous.flatMap((entry) => entry.folds || [])
  const result = merged.map((entry) => ({
    ...entry,
    folds: (entry.folds || []).filter(
      (fold) =>
        ![fold.runId, ...fold.runIds].some((id) => recentRuns.has(id)) &&
        !incoming.some((next) => sharesRuns(fold, next)),
    ),
  }))

  for (const source of incoming) {
    const runIds = new Set([source.runId, ...source.runIds])
    const loaded = new Set(
      result
        .filter((entry) => runIds.has(Number(entry.info.runId ?? entry.info.id)))
        .flatMap((entry) => entry.parts)
        .filter((part) => part.agenaKind !== 'run' && compareChatIds(part.id, source.anchorPartId) < 0)
        .map((part) => part.id),
    )
    let fold = { ...source, hiddenCount: Math.max(0, source.hiddenCount - loaded.size) }
    const old = oldFolds.find((candidate) => sharesRuns(candidate, fold))
    // A new streaming gap takes precedence over the old prefix cursor. A
    // repeated snapshot otherwise keeps the exact position already reached.
    if (old && fold.hiddenCount <= old.hiddenCount && compareChatIds(old.anchorPartId, fold.anchorPartId) < 0) {
      fold = { ...fold, runId: old.runId, anchorPartId: old.anchorPartId, nextCursor: old.nextCursor }
    }
    if (fold.hiddenCount <= 0 || !fold.nextCursor) continue
    const owner = result.find((entry) => Number(entry.info.runId ?? entry.info.id) === fold.runId)
    owner?.folds.push(fold)
  }
  return result
}
