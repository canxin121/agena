import type { MessageEntry, MessageInfo, MessagePart } from '@/types/chat'

export function isOlderPart(
  incoming: { revision?: number; updatedAt?: number },
  current: { revision?: number; updatedAt?: number },
): boolean {
  const revision = incoming.revision ?? 0
  const previous = current.revision ?? 0
  return revision < previous || (revision === previous && (incoming.updatedAt ?? 0) < (current.updatedAt ?? 0))
}

/**
 * True when `incoming` strictly advances the cached snapshot. A
 * revision/updatedAt tie keeps the object a transcript already displayed, so
 * identity-keyed projection caches stay valid while a refetched page merges.
 */
export function isNewerPart(
  incoming: { revision?: number; updatedAt?: number },
  current: { revision?: number; updatedAt?: number },
): boolean {
  const revision = incoming.revision ?? 0
  const previous = current.revision ?? 0
  if (revision !== previous) return revision > previous
  return (incoming.updatedAt ?? 0) > (current.updatedAt ?? 0)
}

export function compareChatIds(left: string, right: string): number {
  if (!/^\d+$/.test(left) || !/^\d+$/.test(right)) {
    throw new TypeError(`Agena chat ids must be decimal integers: ${left}, ${right}`)
  }
  const leftNumber = BigInt(left)
  const rightNumber = BigInt(right)
  if (leftNumber < rightNumber) return -1
  if (leftNumber > rightNumber) return 1
  return 0
}

/**
 * Count top-level transcript messages after the presentation fold is applied.
 *
 * Consecutive assistant runs are one visible assistant block in both Web and
 * TUI. A cursor page that only prepends another assistant run must therefore
 * not stop the older-history drain, even though it adds another raw message
 * entry to the wire projection.
 */
export function foldedMessageCount(list: readonly MessageEntry[]): number {
  let count = 0
  let previousRole = ''
  for (const message of list) {
    const role = String(message?.info?.role || '')
      .trim()
      .toLowerCase()
    if (!role) continue
    if (role === 'assistant' && previousRole === 'assistant') continue
    count += 1
    previousRole = role
  }
  return count
}

export function binarySearchById<T>(
  arr: T[],
  id: string,
  getId: (item: T) => string,
): { found: boolean; index: number } {
  let lo = 0
  let hi = arr.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    const midItem = arr[mid]
    if (midItem === undefined) break
    const cur = getId(midItem)
    if (compareChatIds(cur, id) < 0) lo = mid + 1
    else hi = mid
  }
  const index = lo
  const atIndex = index < arr.length ? arr[index] : undefined
  const found = atIndex !== undefined && getId(atIndex) === id
  return { found, index }
}

export function upsertMessageEntryIn(list: MessageEntry[], info: MessageInfo): MessageEntry {
  const { found, index } = binarySearchById(list, info.id, (m) => m.info.id)
  if (found) {
    const entry = list[index]
    if (!entry) {
      const fallback: MessageEntry = { info, parts: [] }
      list.splice(index, 0, fallback)
      return fallback
    }
    if (!isOlderPart(info, entry.info)) {
      // Run markers are complete snapshots; omitted lifecycle fields clear
      // previous values when a run is resumed or replaced.
      entry.info = info.revision === undefined ? { ...entry.info, ...info } : { ...info }
    }
    if (!Array.isArray(entry.parts)) entry.parts = []
    return entry
  }
  const entry: MessageEntry = { info, parts: [] }
  list.splice(index, 0, entry)
  return entry
}

function sameSnapshotValue(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true
  if (typeof left !== 'object' || typeof right !== 'object' || left === null || right === null) return false
  try {
    return JSON.stringify(left) === JSON.stringify(right)
  } catch {
    return false
  }
}

/** A live snapshot carries no state beyond the part already cached. */
function samePartSnapshot(cached: MessagePart, incoming: MessagePart): boolean {
  const keys = Object.keys(cached)
  if (keys.length !== Object.keys(incoming).length) return false
  for (const key of keys) {
    if (!Object.prototype.hasOwnProperty.call(incoming, key)) return false
    if (!sameSnapshotValue(cached[key], incoming[key])) return false
  }
  return true
}

/**
 * A sparse transcript snapshot must retain detail sections already received.
 * History reads keep cached facts on version ties; live events may correct a
 * same-version snapshot. Detail sections advance by their own revisions.
 */
export function mergePartSnapshot(
  cached: MessagePart,
  incoming: MessagePart,
  options: { preferCachedOnTie?: boolean } = {},
): MessagePart {
  const keepCached = options.preferCachedOnTie ? !isNewerPart(incoming, cached) : isOlderPart(incoming, cached)
  const base = keepCached ? cached : incoming
  const supplement = keepCached ? incoming : cached
  let merged = base
  if (base.agenaKind === 'tool_call' && supplement.agenaKind === 'tool_call') {
    const content = base.agenaContent
    const previous = supplement.agenaContent
    if (
      content &&
      typeof content === 'object' &&
      !Array.isArray(content) &&
      previous &&
      typeof previous === 'object' &&
      !Array.isArray(previous)
    ) {
      const nextContent = { ...content }
      const sections = new Map((base.agenaSections || []).map((loaded) => [loaded.section, loaded]))
      let presentation = base.agenaPresentation
      for (const loaded of supplement.agenaSections || []) {
        if ((loaded.section === 'output' || loaded.section === 'metadata') && supplement.partState !== base.partState)
          continue
        const current = sections.get(loaded.section)
        if (current && current.revision >= loaded.revision) continue
        if (
          !current &&
          loaded.section !== 'presentation' &&
          Object.prototype.hasOwnProperty.call(content, loaded.section)
        )
          continue
        if (loaded.section === 'presentation') presentation = supplement.agenaPresentation
        else if (Object.prototype.hasOwnProperty.call(previous, loaded.section))
          nextContent[loaded.section] = previous[loaded.section]!
        sections.set(loaded.section, loaded)
      }
      merged = {
        ...base,
        agenaContent: nextContent,
        agenaSections: [...sections.values()],
        agenaPresentation: presentation,
      }
    }
  }
  return samePartSnapshot(cached, merged) ? cached : merged
}

/**
 * Apply a live part snapshot to the cached object instead of replacing the
 * array element. Part objects key the transcript projection caches, so a
 * replaced object invalidates every derived block; writing the new fields onto
 * the cached object keeps those identities valid while reactive consumers
 * still observe the new revision/state/text. Fields the snapshot omits are
 * cleared, matching the snapshot-replace semantics this branch replaced.
 */
function applyPartSnapshot(cached: MessagePart, incoming: MessagePart) {
  if (samePartSnapshot(cached, incoming)) return
  for (const key of Object.keys(cached)) {
    if (!Object.prototype.hasOwnProperty.call(incoming, key)) delete cached[key]
  }
  Object.assign(cached, incoming)
}

export function upsertPart(entry: MessageEntry, part: MessagePart, delta: string) {
  if (!Array.isArray(entry.parts)) {
    entry.parts = []
  }
  const parts = entry.parts
  const { found, index } = binarySearchById(parts, part.id, (p) => p.id)

  const partType = typeof part.type === 'string' ? String(part.type) : ''
  const nextText = typeof part.text === 'string' ? String(part.text) : ''

  if (!found) {
    const next: MessagePart = { ...part }
    if ((partType === 'text' || partType === 'reasoning') && delta && (!nextText || partType === 'reasoning')) {
      // Chunk-only streams: seed text from delta.
      next.text = nextText || delta
    }
    parts.splice(index, 0, next)
    return
  }

  const prev = parts[index]
  if (!prev) {
    const fallback: MessagePart = { ...part }
    if ((partType === 'text' || partType === 'reasoning') && delta && (!nextText || partType === 'reasoning')) {
      fallback.text = nextText || delta
    }
    parts.splice(index, 0, fallback)
    return
  }
  const base = typeof prev.text === 'string' ? String(prev.text) : ''
  if (part.revision !== undefined && !delta) {
    applyPartSnapshot(prev, mergePartSnapshot(prev, part))
    return
  }
  if (isOlderPart(part, prev)) return

  // Prefer authoritative part snapshots when present, but fall back to delta when
  // the snapshot isn't advancing (some emitters stream delta-only).
  if (partType === 'text' || partType === 'reasoning') {
    if (delta) {
      if (nextText && nextText.length > base.length) {
        Object.assign(prev, part, { text: nextText })
      } else {
        Object.assign(prev, part, { text: base + delta })
      }
    } else if (nextText) {
      Object.assign(prev, part, { text: nextText })
    } else {
      Object.assign(prev, part)
    }
    return
  }

  Object.assign(prev, part)
}
