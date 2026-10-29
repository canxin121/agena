import { computed, onScopeDispose, ref, watch, type Ref } from 'vue'
import { limitBackgroundReads } from '@/lib/backgroundReads'
import { probePreviewProxyResponse } from '../api/previewProxyProbe'
import type { WorkspacePreviewSession } from '../api/workspacePreviewApi'
import { previewRunIdentity } from './previewAccess'
import { buildPreviewFrameSrc } from './previewUrl'

export type PreviewHealthState = 'unknown' | 'checking' | 'ok' | 'error'
type Entry = { state: PreviewHealthState; checkedAt: number; target: string; failures: number }
type Job = { target: string; controller: AbortController }

export function usePreviewHealth(options: {
  sessions: Readonly<Ref<WorkspacePreviewSession[]>>
  visible: Readonly<Ref<boolean>>
  loading: Readonly<Ref<boolean>>
  probe?: typeof probePreviewProxyResponse
  timeoutMs?: number
  retryMs?: number
  healthyTtlMs?: number
}) {
  const entries = ref<Record<string, Entry>>({})
  const documentVisible = ref(typeof document === 'undefined' || document.visibilityState !== 'hidden')
  const active = computed(() => options.visible.value && documentVisible.value)
  const jobs = new Map<string, Job>()
  let timer: ReturnType<typeof setTimeout> | undefined
  let disposed = false
  const targetFor = (session: WorkspacePreviewSession) => `${session.proxyBasePath}:${previewRunIdentity(session)}`
  const stopped = (session: WorkspacePreviewSession) => ['stopped', 'idle'].includes(session.state.toLowerCase())
  const delayFor = (entry: Entry) =>
    entry.state === 'ok'
      ? (options.healthyTtlMs ?? 60_000)
      : Math.min(30_000, (options.retryMs ?? 2_000) * 2 ** Math.max(0, entry.failures - 1))

  function clear(id?: string) {
    const ids = id ? [id] : [...new Set([...jobs.keys(), ...Object.keys(entries.value)])]
    for (const key of ids) {
      jobs.get(key)?.controller.abort()
      jobs.delete(key)
      const { [key]: _removed, ...rest } = entries.value
      entries.value = rest
    }
    schedule()
  }

  function schedule() {
    clearTimeout(timer)
    timer = undefined
    if (disposed || !active.value || options.loading.value) return
    let due = Infinity
    for (const session of options.sessions.value) {
      if (stopped(session) || jobs.has(session.id)) continue
      const entry = entries.value[session.id]
      due = Math.min(due, entry ? entry.checkedAt + delayFor(entry) : Date.now())
    }
    if (Number.isFinite(due)) timer = setTimeout(refresh, Math.max(1, due - Date.now()))
  }

  async function probe(session: WorkspacePreviewSession, target: string) {
    const id = session.id
    const previous = entries.value[id]
    const job = { target, controller: new AbortController() }
    jobs.set(id, job)
    entries.value = {
      ...entries.value,
      [id]: { state: 'checking', checkedAt: Date.now(), target, failures: previous?.failures ?? 0 },
    }
    const current = () =>
      !disposed &&
      active.value &&
      jobs.get(id) === job &&
      options.sessions.value.some((record) => record.id === id && targetFor(record) === target && !stopped(record))
    const finish = (ok: boolean) => {
      if (!current()) return
      entries.value = {
        ...entries.value,
        [id]: {
          state: ok ? 'ok' : 'error',
          checkedAt: Date.now(),
          target,
          failures: ok ? 0 : Math.min(5, (previous?.failures ?? 0) + 1),
        },
      }
    }
    let timedOut = false
    try {
      const src = buildPreviewFrameSrc(session.proxyBasePath, Date.now())
      if (!src) throw new Error('Preview proxy path is invalid')
      const response = await limitBackgroundReads(async () => {
        // Time spent queued behind another pane is not an upstream failure.
        const deadline = setTimeout(() => {
          timedOut = true
          job.controller.abort(new DOMException('Preview health probe timed out', 'TimeoutError'))
        }, options.timeoutMs ?? 3_500)
        try {
          return await (options.probe ?? probePreviewProxyResponse)(src, job.controller.signal)
        } finally {
          clearTimeout(deadline)
        }
      }, job.controller.signal)
      if (!job.controller.signal.aborted) finish(response.ok || response.upstreamResponded)
    } catch {
      if (timedOut || !job.controller.signal.aborted) finish(false)
    } finally {
      // A cancelled old run must not remove the replacement run's pending job.
      if (jobs.get(id) === job) jobs.delete(id)
      schedule()
    }
  }

  function refresh() {
    clearTimeout(timer)
    timer = undefined
    const records = new Map(options.sessions.value.map((session) => [session.id, session]))
    for (const [id, job] of jobs) {
      const session = records.get(id)
      if (!active.value || !session || stopped(session) || targetFor(session) !== job.target) clear(id)
    }
    for (const [id, entry] of Object.entries(entries.value)) {
      const session = records.get(id)
      if (!session || stopped(session) || targetFor(session) !== entry.target) clear(id)
    }
    if (disposed || !active.value || options.loading.value) return
    for (const session of records.values()) {
      if (stopped(session) || jobs.has(session.id)) continue
      const target = targetFor(session)
      const entry = entries.value[session.id]
      if (entry?.target === target && Date.now() - entry.checkedAt < delayFor(entry)) continue
      void probe(session, target)
    }
    schedule()
  }

  watch([active, options.loading, () => options.sessions.value.map(targetFor).join('\n')], refresh, {
    immediate: true,
    flush: 'sync',
  })
  const visibility = () => {
    documentVisible.value = document.visibilityState !== 'hidden'
  }
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', visibility)
  onScopeDispose(() => {
    disposed = true
    clear()
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', visibility)
  })
  return { entries, clear, refresh }
}
