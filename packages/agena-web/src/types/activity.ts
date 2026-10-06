/** Canonical session-owned background members returned by GET /sessions/:id/state. */
export type SessionActivity = {
  id: string
  kind: string
  status: string
  title: string
  description: string
  command?: string
  workdir?: string
  source_part_id?: number
  finished_at_ms?: number
  session_id?: number
  parent_session_id?: number
  message?: string
  failure?: { user: { fallback: string } }
  next_event_at_ms?: number
  last_seq: number
  controls: string[]
}

export type ActivityLog = {
  activity_id: string
  status: string
  lines: { seq: number; stream: string; text: string; chunk?: boolean }[]
  last_seq: number
  has_more: boolean
  dropped_lines: number
  exit_code?: number | null
  completion_reason?: string | null
}

export function activityIsActive(status: string): boolean {
  return ['pending', 'running', 'waiting', 'paused'].includes(status)
}

/** Descriptors are small and flat apart from controls and a failure envelope. */
export function activityEquals(left: object, right: object): boolean {
  const before = left as Record<string, unknown>
  const after = right as Record<string, unknown>
  const keys = Object.keys(before)
  return keys.length === Object.keys(after).length && keys.every(key =>
    before[key] === after[key] || (before[key] !== null && typeof before[key] === 'object'
      && JSON.stringify(before[key]) === JSON.stringify(after[key])))
}

export function activityLogText(log: ActivityLog | null): string {
  return log?.lines.map(line => line.text + (line.chunk || line.text.endsWith('\n') ? '' : '\n')).join('') ?? ''
}

/** Bound retained output even for a monitor that runs for days. */
export function mergeActivityLog(previous: ActivityLog | null, next: ActivityLog): ActivityLog {
  const old = previous?.activity_id === next.activity_id ? previous.lines : []
  // Shell lines are immutable, while a delegated task's cursor line is a
  // snapshot of its still-streaming run. Replace a repeated sequence number.
  const bySequence = new Map(old.map(line => [line.seq, line]))
  for (const line of next.lines) bySequence.set(line.seq, line)
  const lines = [...bySequence.values()].sort((a, b) => a.seq - b.seq).slice(-200)
  let budget = 128 * 1024
  const encoder = new TextEncoder()
  const decoder = new TextDecoder()
  const retained: ActivityLog['lines'] = []
  for (const line of lines.reverse()) {
    if (budget <= 0) break
    const bytes = encoder.encode(line.text)
    if (budget < 4) break
    let text = line.text
    if (bytes.length > budget) {
      let start = bytes.length - budget + 3
      while ((bytes[start]! & 0xc0) === 0x80) start++
      text = '…' + decoder.decode(bytes.subarray(start))
    }
    retained.push({ ...line, text })
    budget -= encoder.encode(text).length
  }
  return { ...next, last_seq: Math.max(previous?.activity_id === next.activity_id ? previous.last_seq : 0, next.last_seq), lines: retained.reverse() }
}
