import { computed, onMounted, onScopeDispose, reactive, watch } from 'vue'
import { usePlanViewer } from './usePlanViewer'
import { subscribeResource } from '../../lib/resourceSync'
import { isDocumentVisible } from '../../lib/backgroundReads'

export function useSessionPlan(
  sessionId: () => string | null,
  _busy: () => boolean,
  _changeSignal: () => unknown,
  invoke: Parameters<typeof usePlanViewer>[1],
) {
  const expandedSessions = reactive(new Set<string>())
  let lastStarted = 0
  let failures = 0
  let nextAllowedAt = 0
  const viewer = usePlanViewer(
    () => [Boolean(sessionId()), sessionId()],
    async (...args) => {
      lastStarted = Date.now()
      const owner = sessionId()
      try {
        const result = await Promise.resolve().then(() => invoke(...args))
        if (owner === sessionId()) {
          failures = 0
          nextAllowedAt = Date.now() + 1000
        }
        return result
      } catch (error) {
        if (owner === sessionId() && !args[3].aborted) {
          failures = Math.min(failures + 1, 5)
          nextAllowedAt = Date.now() + Math.min(60_000, 5000 * 2 ** (failures - 1))
          scheduleRefresh()
        }
        throw error
      }
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
  let unsubscribe: (() => void) | undefined
  let disposed = false
  let dirty = false
  const isVisible = isDocumentVisible

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
        void viewer.refresh(false)
      },
      Math.max(180, 1000 - (Date.now() - lastStarted), nextAllowedAt - Date.now()),
    )
  }

  watch([viewer.loading, viewer.toggling], ([loading, toggling]) => {
    if (!loading && !toggling && dirty) scheduleRefresh()
  })
  watch(
    sessionId,
    (id) => {
      clearTimeout(timer)
      timer = undefined
      dirty = false
      failures = 0
      nextAllowedAt = 0
      unsubscribe?.()
      unsubscribe = id ? subscribeResource(`session:${id}:plan`, scheduleRefresh) : undefined
    },
    { flush: 'sync', immediate: true },
  )
  watch(expanded, (value) => {
    if (value && Date.now() - lastStarted >= 5_000) scheduleRefresh()
  })
  onMounted(() => {
    document.addEventListener('visibilitychange', onVisibility)
  })
  function onVisibility() {
    if (isVisible()) scheduleRefresh()
    else {
      clearTimeout(timer)
      timer = undefined
      viewer.pauseRead()
    }
  }
  onScopeDispose(() => {
    disposed = true
    clearTimeout(timer)
    unsubscribe?.()
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisibility)
  })
  return { viewer, expanded, visible }
}
