/**
 * Absolute timeline numbering for the Web chat transcript.
 *
 * The transcript loads newest-first, so `chat.messages` only ever holds the
 * newest suffix of the conversation. A position inside that window is not a
 * conversation index; the server-assigned `userMessageOrdinal` is. These
 * helpers turn a window position plus the authoritative session total into the
 * `current / total` pair the transcript navigator displays.
 */
export type NavigableMessage = { id: string; ordinal: number | null }

function finiteOrdinal(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return null
  return Math.floor(value)
}

/**
 * The session's user-message total. The server count is authoritative; a
 * live-delivered ordinal (the newest user message's ordinal IS the total)
 * can only confirm it. The result never falls below the number of messages
 * already loaded, so the counter can never read "5 / 3".
 */
export function timelineTotal(entries: readonly NavigableMessage[], serverTotal: number | null | undefined): number {
  const base =
    typeof serverTotal === 'number' && Number.isFinite(serverTotal)
      ? Math.max(0, Math.floor(serverTotal))
      : entries.length
  let highest = 0
  for (const entry of entries) {
    const ordinal = finiteOrdinal(entry.ordinal)
    if (ordinal !== null && ordinal > highest) highest = ordinal
  }
  return Math.max(base, highest, entries.length)
}

/**
 * Absolute 1-based position of the message at `index` inside the loaded
 * window. Prefers the server ordinal; otherwise derives the same value from
 * the authoritative total, which is exact because the loaded window is the
 * newest suffix of the conversation.
 */
export function timelineCurrentOrdinal(entries: readonly NavigableMessage[], index: number, total: number): number {
  if (!entries.length) return 0
  const safeTotal =
    typeof total === 'number' && Number.isFinite(total) ? Math.max(0, Math.floor(total)) : entries.length
  const clamped = Math.min(Math.max(0, Math.floor(index)), entries.length - 1)
  const entry = entries[clamped]
  const ordinal = finiteOrdinal(entry?.ordinal)
  if (ordinal !== null) return ordinal
  const derived = safeTotal - (entries.length - 1 - clamped)
  return Math.min(Math.max(1, derived), Math.max(1, safeTotal))
}
