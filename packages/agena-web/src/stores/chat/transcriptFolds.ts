import type { MessageEntry, MessageFold } from '../../types/chat'
import { compareChatIds } from './messageIndex'

export function transcriptFoldKey(sessionId: string, fold: MessageFold): string {
  return `${sessionId}:${Math.min(fold.runId, ...fold.runIds)}`
}

function sharesRuns(left: MessageFold, right: MessageFold): boolean {
  const ids = new Set([left.runId, ...left.runIds])
  return [right.runId, ...right.runIds].some((id) => ids.has(id))
}

function sameFoldDescriptor(left: MessageFold, right: MessageFold): boolean {
  const leftRuns = left.runIds || []
  const rightRuns = right.runIds || []
  return (
    left.runId === right.runId &&
    left.anchorPartId === right.anchorPartId &&
    left.hiddenCount === right.hiddenCount &&
    left.nextCursor === right.nextCursor &&
    leftRuns.length === rightRuns.length &&
    leftRuns.every((id, index) => id === rightRuns[index])
  )
}

function sameFoldList(current: readonly MessageFold[], next: readonly MessageFold[]): boolean {
  return current.length === next.length && next.every((fold, index) => sameFoldDescriptor(current[index]!, fold))
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
  const result = merged.map((entry) => {
    const current = entry.folds || []
    const folds = current.filter(
      (fold) =>
        ![fold.runId, ...fold.runIds].some((id) => recentRuns.has(id)) &&
        !incoming.some((next) => sharesRuns(fold, next)),
    )
    // A reply whose fold list survives the refresh unchanged keeps its entry
    // object, so reply-level projection caches stay valid instead of
    // re-rendering every message for an identical snapshot.
    return sameFoldList(current, folds) ? entry : { ...entry, folds }
  })

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
    if (!owner) continue
    if (!owner.folds) owner.folds = []
    owner.folds.push(fold)
  }
  return result
}
