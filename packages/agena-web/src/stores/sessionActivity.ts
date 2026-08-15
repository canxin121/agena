import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { apiJson } from '../lib/api'
import { extractSessionActivityUpdate } from '../lib/sessionActivityEvent.js'
import type { SseEvent } from '../lib/sse'
import type { JsonValue as JsonLike } from '@/types/json'

type Phase = 'idle' | 'busy' | 'cooldown'

type Snapshot = Record<string, { type: Phase }>

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
          next[String(sidRaw)] = { type: 'busy' }
        }
      }
      snapshot.value = next
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  function applyEvent(evt: SseEvent) {
    const upd = extractSessionActivityUpdate(evt)
    if (!upd) return
    const sessionId = upd.sessionID
    const phase = upd.phase as Phase
    if (!sessionId) return
    if (phase === 'idle') {
      if (!Object.prototype.hasOwnProperty.call(snapshot.value, sessionId)) return
      const next = { ...snapshot.value }
      delete next[sessionId]
      snapshot.value = next
      return
    }
    snapshot.value = {
      ...snapshot.value,
      [sessionId]: { type: phase },
    }
  }

  return { snapshot, sessions, loading, error, refresh, applyEvent }
})
