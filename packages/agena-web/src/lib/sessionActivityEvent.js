// Normalize agena live events into a { sessionID, phase } activity update.
//
// Sources:
//   - runtime_signal (kind "activity") with payload = BackgroundActivityResource
//     { id, kind, status, session_id, ... }
//   - session_changed part_added/part_updated where part.kind === "run"
//     (run marker: state in_progress → busy; terminal → idle)

function trimString(v) {
  return typeof v === 'string' ? v.trim() : ''
}

function readRecord(v) {
  return v && typeof v === 'object' && !Array.isArray(v) ? v : null
}

function readSessionId(props) {
  const p = readRecord(props) || {}
  const raw = typeof p.session_id === 'number' ? p.session_id : p.sessionId ?? p.sessionID
  const n = typeof raw === 'number' && Number.isFinite(raw) ? raw : null
  if (n != null) return String(n)
  const s = trimString(raw)
  return s || ''
}

function activityPhaseFromStatus(status) {
  const s = trimString(status).toLowerCase()
  if (s === 'pending' || s === 'running' || s === 'waiting' || s === 'paused') return 'busy'
  return 'idle'
}

/**
 * @param {unknown} evt
 * @returns {{ sessionID: string, phase: 'idle'|'busy'|'cooldown' } | null}
 */
export function extractSessionActivityUpdate(evt) {
  const type = trimString(evt && evt.type)
  if (!type) return null
  const props = readRecord(evt && evt.properties) || {}

  if (type === 'runtime_signal') {
    const kind = trimString(props.kind)
    if (kind !== 'activity') return null
    const payload = readRecord(props.payload)
    const sessionID = readSessionId(payload || {})
    if (!sessionID) return null
    const status = trimString(payload && payload.status)
    return { sessionID, phase: activityPhaseFromStatus(status) }
  }

  if (type === 'session_changed') {
    const changeKind = trimString(props.kind)
    if (changeKind !== 'part_added' && changeKind !== 'part_updated') return null
    const part = readRecord(props.part)
    if (!part || trimString(part.kind) !== 'run') return null
    const sessionID = readSessionId(props)
    if (!sessionID) return null
    const state = trimString(part.state).toLowerCase()
    if (state === 'in_progress' || state === 'pending' || state === 'running') {
      return { sessionID, phase: 'busy' }
    }
    return { sessionID, phase: 'idle' }
  }

  return null
}
