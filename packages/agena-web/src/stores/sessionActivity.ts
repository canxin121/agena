import { defineStore } from 'pinia'
import { computed, onScopeDispose, ref } from 'vue'

import { apiJson } from '../lib/api'
import { extractSessionActivityUpdate } from '../lib/sessionActivityEvent.js'
import type { SseEvent } from '../lib/sse'
import type { JsonValue as JsonLike } from '@/types/json'

type Phase = 'idle' | 'busy' | 'cooldown'

export type SessionActivitySnapshotEntry = { type: Phase; kinds: string[] }
type Snapshot = Record<string, SessionActivitySnapshotEntry>

type ActivityItem = {
  id?: string
  kind?: string
  status?: string
  session_id?: number
  [k: string]: JsonLike
}

function isActiveActivityStatus(status: string): boolean {
  const s = String(status || '').toLowerCase()
  return s === 'pending' || s === 'running' || s === 'waiting' || s === 'paused'
}

export const useSessionActivityStore = defineStore('sessionActivity', () => {
  const snapshot = ref<Snapshot>({})
  const loading = ref(false)
  const error = ref<string | null>(null)
  let refreshTimer: number | null = null
  let inFlight: Promise<void> | null = null
  let dirty = false
  let disposed = false
  let eventGeneration = 0
  let lastStartedAt = 0
  let controller: AbortController | null = null

  const sessions = computed(() => Object.entries(snapshot.value))

  /** GET /api/v1/activities → per-session busy snapshot (active activities only). */
  async function refreshInternal() {
    const generation = eventGeneration
    lastStartedAt = Date.now()
    controller = new AbortController()
    const timeout = window.setTimeout(() => controller?.abort(), 30_000)
    loading.value = true
    error.value = null
    try {
      const list = await apiJson<ActivityItem[]>('/api/v1/activities', { signal: controller.signal })
      const arr = Array.isArray(list) ? list : []
      const next: Snapshot = {}
      for (const item of arr) {
        const sidRaw = typeof item?.session_id === 'number' ? item.session_id : null
        if (sidRaw == null) continue
        if (isActiveActivityStatus(String(item?.status || ''))) {
          const sid = String(sidRaw)
          const kinds = next[sid]?.kinds || []
          const kind = String(item?.kind || '')
            .trim()
            .toLowerCase()
          // Keep one entry per active activity. TUI renders counts (for
          // example, "shell 2"), so collapsing equal kinds loses information.
          next[sid] = { type: 'busy', kinds: kind ? [...kinds, kind] : kinds }
        }
      }
      if (!disposed && generation === eventGeneration) snapshot.value = next
    } catch (err) {
      if (!disposed) error.value = err instanceof Error ? err.message : String(err)
    } finally {
      window.clearTimeout(timeout)
      loading.value = false
    }
  }

  function refresh(): Promise<void> {
    if (disposed) return Promise.resolve()
    if (inFlight) return inFlight
    dirty = false
    if (refreshTimer !== null) window.clearTimeout(refreshTimer)
    refreshTimer = null
    inFlight = refreshInternal().finally(() => {
      inFlight = null
      if (dirty) scheduleRefresh()
    })
    return inFlight
  }

  function scheduleRefresh() {
    dirty = true
    if (disposed || refreshTimer !== null || inFlight) return
    refreshTimer = window.setTimeout(
      () => {
        refreshTimer = null
        void refresh()
      },
      Math.max(100, 500 - (Date.now() - lastStartedAt)),
    )
  }

  function activityKindFromEvent(evt: SseEvent): string {
    if (evt.type !== 'runtime_signal') return ''
    const props = evt.properties && typeof evt.properties === 'object' ? evt.properties : {}
    if (String(props.kind || '').trim() !== 'activity') return ''
    const payload = props.payload && typeof props.payload === 'object' ? props.payload : {}
    return String(payload.kind || '')
      .trim()
      .toLowerCase()
  }

  function applyEvent(evt: SseEvent) {
    const upd = extractSessionActivityUpdate(evt)
    if (!upd) return
    const sessionId = upd.sessionID
    const phase = upd.phase as Phase
    if (!sessionId) return
    eventGeneration++
    const activityKind = activityKindFromEvent(evt)
    if (phase === 'idle') {
      if (!Object.prototype.hasOwnProperty.call(snapshot.value, sessionId)) {
        scheduleRefresh()
        return
      }
      const next = { ...snapshot.value }
      delete next[sessionId]
      snapshot.value = next
      scheduleRefresh()
      return
    }
    snapshot.value = {
      ...snapshot.value,
      [sessionId]: {
        type: phase,
        kinds:
          activityKind && !snapshot.value[sessionId]?.kinds.includes(activityKind)
            ? [...(snapshot.value[sessionId]?.kinds || []), activityKind]
            : snapshot.value[sessionId]?.kinds || [],
      },
    }
    // The event is an optimistic signal; the list endpoint is authoritative
    // for concurrent same-kind activities and terminal transitions.
    if (activityKind) scheduleRefresh()
  }

  onScopeDispose(() => {
    disposed = true
    controller?.abort()
    if (refreshTimer !== null) window.clearTimeout(refreshTimer)
  })

  return { snapshot, sessions, loading, error, refresh, applyEvent }
})
