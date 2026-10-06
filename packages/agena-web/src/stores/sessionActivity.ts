import { defineStore } from 'pinia'
import { computed, onScopeDispose, ref } from 'vue'

import { conditionalJsonObserved } from '../lib/conditionalJson'
import { apiUrl } from '../lib/api'
import { readUiAuthTokenVersion } from '../lib/uiAuthToken'
import { isDocumentVisible } from '../lib/backgroundReads'
import {
  canReuseResource,
  captureResourceObservation,
  checkResourceVersions,
  subscribeResource,
} from '../lib/resourceSync'
import { createRevalidator } from '../lib/revalidation'
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
  parent_session_id?: number
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
  let disposed = false
  let eventGeneration = 0
  const activities = new Map<string, ActivityItem>()
  const changedAt = new Map<string, number>()
  const liveAt = new Map<string, number>()
  const runs = new Map<string, Phase>()
  let controller: AbortController | null = null
  let resourceScope = ''
  let observedList: ReturnType<typeof captureResourceObservation> | undefined
  let recoveryGeneration = 0
  let observedRecoveryGeneration = -1
  let forceNextRead = false
  function ensureActivityScope() {
    const next = `${readUiAuthTokenVersion()}:${apiUrl('/')}`
    if (next !== resourceScope) {
      resourceScope = next
      controller?.abort()
      activities.clear()
      changedAt.clear()
      liveAt.clear()
      runs.clear()
      snapshot.value = {}
      observedList = undefined
      recoveryGeneration++
      observedRecoveryGeneration = -1
      eventGeneration++
    }
    return resourceScope
  }

  const sessions = computed(() => Object.entries(snapshot.value))

  function rebuild() {
    const next: Snapshot = {}
    for (const [sid, phase] of runs) if (phase !== 'idle') next[sid] = { type: phase, kinds: [] }
    for (const item of activities.values()) {
      const id = item.parent_session_id ?? item.session_id
      if (typeof id !== 'number' || !isActiveActivityStatus(String(item.status ?? ''))) continue
      const sid = String(id)
      const kinds = next[sid]?.kinds ?? []
      const kind = String(item.kind ?? '')
        .trim()
        .toLowerCase()
      next[sid] = { type: 'busy', kinds: kind ? [...kinds, kind] : kinds }
    }
    for (const entry of Object.values(next)) entry.kinds = [...new Set(entry.kinds)].sort()
    const previous = snapshot.value
    let changed = Object.keys(previous).length !== Object.keys(next).length
    for (const [sid, entry] of Object.entries(next)) {
      const old = previous[sid]
      if (
        old?.type === entry.type &&
        old.kinds.length === entry.kinds.length &&
        old.kinds.every((kind, index) => kind === entry.kinds[index])
      )
        next[sid] = old
      else changed = true
    }
    if (!changed) return
    snapshot.value = next
  }

  /** GET /api/v1/activities → per-session busy snapshot (active activities only). */
  async function refreshInternal() {
    const scope = ensureActivityScope()
    const generation = eventGeneration
    const requestRecoveryGeneration = recoveryGeneration
    const request = new AbortController()
    controller = request
    const timeout = window.setTimeout(() => request.abort(new Error('Activity request timed out')), 30_000)
    loading.value = true
    error.value = null
    try {
      const force = forceNextRead
      forceNextRead = false
      if (observedList?.scope === captureResourceObservation('activities').scope && observedList.token && !force) {
        await checkResourceVersions(['activities'], request.signal)
        if (canReuseResource('activities', observedList.token)) {
          observedRecoveryGeneration = recoveryGeneration
          return
        }
      }
      const { value: list, observation } = await conditionalJsonObserved<ActivityItem[]>(
        'activities',
        '/api/v1/activities',
        {
          signal: request.signal,
        },
        force,
      )
      const arr = Array.isArray(list) ? list : []
      const next = new Map<string, ActivityItem>()
      for (const item of arr) {
        if (typeof item.id === 'string') next.set(item.id, item)
      }
      if (!disposed && !request.signal.aborted && scope === ensureActivityScope()) {
        // Only the records touched during this read override the response.
        // Continuous output from one task must not discard unrelated rows.
        for (const [id, changed] of changedAt)
          if (changed > generation) {
            const current = activities.get(id)
            if (current) next.set(id, current)
            else next.delete(id)
          }
        activities.clear()
        for (const [id, item] of next) activities.set(id, item)
        observedList = observation
        observedRecoveryGeneration = requestRecoveryGeneration
        rebuild()
      }
    } catch (err) {
      if (!disposed && scope === ensureActivityScope() && !(err instanceof DOMException && err.name === 'AbortError')) {
        error.value = err instanceof Error ? err.message : String(err)
      }
      throw err
    } finally {
      window.clearTimeout(timeout)
      if (controller === request) {
        controller = null
        loading.value = false
      }
    }
  }

  const queue = createRevalidator(refreshInternal, {
    intervalMs: 250,
    retryMs: 5000,
    enabled: () => !disposed && isDocumentVisible(),
  })
  const releaseResource = subscribeResource('activities', (event) => {
    if (event?.type !== 'runtime_signal' || event.properties?.kind !== 'activity') {
      recoveryGeneration++
      queue.invalidate(150)
    }
  })
  const refresh = () => {
    forceNextRead = true
    recoveryGeneration++
    return queue.refresh().catch(() => {})
  }
  const scheduleRefresh = () => {
    recoveryGeneration++
    queue.invalidate(150)
  }
  const visibility = () => {
    if (isDocumentVisible()) queue.resume()
    else {
      queue.pause()
      controller?.abort()
    }
  }
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', visibility)

  function applyEvent(evt: SseEvent) {
    ensureActivityScope()
    const props = evt.properties ?? {}
    if (evt.type === 'runtime_signal' && props.kind === 'activity') {
      const payload =
        props.payload && typeof props.payload === 'object' && !Array.isArray(props.payload) ? props.payload : {}
      const item =
        payload.activity && typeof payload.activity === 'object' && !Array.isArray(payload.activity)
          ? payload.activity
          : payload
      if (typeof item.id !== 'string' || typeof item.status !== 'string') {
        scheduleRefresh()
        return
      }
      const timestamp = typeof payload.ts_ms === 'number' ? payload.ts_ms : 0
      // Separate progress/completion/dismissal events can share a millisecond.
      // The stream preserves their order; only strictly older events are stale.
      if (timestamp && timestamp < (liveAt.get(item.id) ?? 0)) return
      liveAt.set(item.id, timestamp)
      changedAt.set(item.id, ++eventGeneration)
      const previous = activities.get(item.id)
      const dismissed = payload.reason === 'dismissed'
      if (dismissed) activities.delete(item.id)
      else activities.set(item.id, item as ActivityItem)
      if (observedList?.token && observedRecoveryGeneration === recoveryGeneration) {
        const current = captureResourceObservation('activities')
        if (
          current.scope === observedList.scope &&
          current.token &&
          canReuseResource('activities', current.token) &&
          !controller
        )
          observedList = current
      }
      if (
        dismissed ||
        !previous ||
        previous.kind !== item.kind ||
        previous.status !== item.status ||
        previous.session_id !== item.session_id ||
        previous.parent_session_id !== item.parent_session_id
      )
        rebuild()
      // Tombstones protect in-flight reads but have a bounded lifetime.
      if (changedAt.size > 1024)
        for (const id of changedAt.keys()) {
          if (activities.has(id)) continue
          changedAt.delete(id)
          liveAt.delete(id)
          if (changedAt.size <= 512) break
        }
      return
    }
    const upd = extractSessionActivityUpdate(evt)
    if (!upd) return
    const sessionId = upd.sessionID
    const phase = upd.phase as Phase
    if (!sessionId) return
    if ((runs.get(sessionId) ?? 'idle') === phase) return
    if (phase === 'idle') runs.delete(sessionId)
    else runs.set(sessionId, phase)
    if (runs.size > 512) runs.delete(runs.keys().next().value!)
    rebuild()
  }

  onScopeDispose(() => {
    disposed = true
    controller?.abort()
    queue.dispose()
    releaseResource()
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', visibility)
  })

  return { snapshot, sessions, loading, error, refresh, invalidate: scheduleRefresh, applyEvent }
})
