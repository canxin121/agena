/** Canonical session-owned background members returned by GET /sessions/:id/state. */
export type SessionActivity = {
  id: string
  kind: string
  status: string
  title: string
  description: string
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
  lines: { seq: number; stream: string; text: string }[]
  last_seq: number
  has_more: boolean
  dropped_lines: number
  exit_code?: number | null
  completion_reason?: string | null
}

export function activityIsActive(status: string): boolean {
  return ['pending', 'running', 'waiting', 'paused'].includes(status)
}

/** Bound retained output even for a monitor that runs for days. */
export function mergeActivityLog(previous: ActivityLog | null, next: ActivityLog): ActivityLog {
  const old = previous?.activity_id === next.activity_id ? previous.lines : []
  const cursor = old.at(-1)?.seq ?? 0
  const lines = [...old, ...next.lines.filter((line) => line.seq > cursor)].slice(-200)
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
  return { ...next, lines: retained.reverse() }
}
