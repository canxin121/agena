import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

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

  const sessions = computed(() => Object.entries(snapshot.value))

  /** GET /api/v1/activities → per-session busy snapshot (active activities only). */
  async function refresh() {
    loading.value = true
    error.value = null
    try {
      const list = await apiJson<ActivityItem[]>('/api/v1/activities')
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
      snapshot.value = next
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  function scheduleRefresh() {
    if (refreshTimer !== null) return
    refreshTimer = window.setTimeout(() => {
      refreshTimer = null
      void refresh()
    }, 100)
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
    const activityKind = activityKindFromEvent(evt)
    if (phase === 'idle') {
      if (!Object.prototype.hasOwnProperty.call(snapshot.value, sessionId)) return
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

  return { snapshot, sessions, loading, error, refresh, applyEvent }
})
