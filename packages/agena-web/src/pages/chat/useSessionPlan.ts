import { computed, onMounted, onScopeDispose, reactive, watch } from 'vue'
import { usePlanViewer } from './usePlanViewer'

export function useSessionPlan(
  sessionId: () => string | null,
  busy: () => boolean,
  changeSignal: () => unknown,
  invoke: Parameters<typeof usePlanViewer>[1],
) {
  const expandedSessions = reactive(new Set<string>())
  let lastStarted = 0
  const viewer = usePlanViewer(
    () => [Boolean(sessionId()), sessionId()],
    (...args) => {
      lastStarted = Date.now()
      return invoke(...args)
    },
  )
  const expanded = computed({
    get: () => expandedSessions.has(sessionId() || ''),
    set: (value: boolean) => {
      const id = sessionId()
      if (!id) return
      if (value) {
        expandedSessions.delete(id)
        expandedSessions.add(id)
        if (expandedSessions.size > 32) expandedSessions.delete(expandedSessions.values().next().value!)
      } else expandedSessions.delete(id)
    },
  })
  const visible = computed(() => Boolean(viewer.snapshot.value) || expanded.value)
  let timer: ReturnType<typeof setTimeout> | undefined
  let poll: ReturnType<typeof setInterval> | undefined
  let disposed = false
  let dirty = false
  const isVisible = () => typeof document === 'undefined' || document.visibilityState !== 'hidden'

  function scheduleRefresh() {
    dirty = true
    if (disposed || timer || !sessionId() || !isVisible() || viewer.loading.value || viewer.toggling.value) return
    timer = setTimeout(
      () => {
        timer = undefined
        if (disposed || !isVisible()) return
        if (viewer.loading.value || viewer.toggling.value) {
          return
        }
        dirty = false
        void viewer.refresh()
      },
      Math.max(180, 750 - (Date.now() - lastStarted)),
    )
  }

  watch(changeSignal, scheduleRefresh)
  watch([viewer.loading, viewer.toggling], ([loading, toggling]) => {
    if (!loading && !toggling && dirty) scheduleRefresh()
  })
  watch(
    sessionId,
    () => {
      clearTimeout(timer)
      timer = undefined
      dirty = false
    },
    { flush: 'sync' },
  )
  watch(expanded, (value) => {
    if (value && Date.now() - lastStarted >= 5_000) scheduleRefresh()
  })
  onMounted(() => {
    poll = setInterval(() => {
      // Nothing rendered means nothing to refresh: a plan appears when the
      // session signals a change, not from a timer that queries an empty viewer.
      if (!visible.value) return
      const interval = busy() || expanded.value ? 5_000 : viewer.snapshot.value ? 30_000 : 60_000
      if (Date.now() - lastStarted >= interval) scheduleRefresh()
    }, 1_000)
    document.addEventListener('visibilitychange', scheduleRefresh)
  })
  onScopeDispose(() => {
    disposed = true
    clearTimeout(timer)
    clearInterval(poll)
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', scheduleRefresh)
  })
  return { viewer, expanded, visible }
}
