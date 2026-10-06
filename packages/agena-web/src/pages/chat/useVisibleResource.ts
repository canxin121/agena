import { onBeforeUnmount, ref, shallowRef, watch, type Ref } from 'vue'
import { isDocumentVisible } from '../../lib/backgroundReads'
import {
  canReuseResource,
  captureResourceObservation,
  checkResourceVersions,
  subscribeResource,
} from '../../lib/resourceSync'

/** One cancellable read per visible resource; key changes discard late results. */
export function useVisibleResource<T>(options: {
  key: Ref<string>
  interval: () => number | null
  resource?: (key: string) => string | undefined
  minInterval?: number
  load: (
    key: string,
    signal: AbortSignal,
    previous: T | null,
    force: boolean,
    observe: (observation: ReturnType<typeof captureResourceObservation>) => void,
  ) => Promise<T>
}) {
  const data = shallowRef<T | null>(null)
  const error = ref('')
  const loading = ref(false)
  let controller: AbortController | null = null
  let timer: ReturnType<typeof setTimeout> | undefined
  let generation = 0
  let disposed = false
  let lastStart = 0
  let pendingRefresh = false
  let pendingForce = false
  let unsubscribe: (() => void) | undefined
  let failures = 0
  let allowedAt = 0
  let observed: ReturnType<typeof captureResourceObservation> | undefined

  function cancel() {
    generation++
    clearTimeout(timer)
    controller?.abort()
    controller = null
    loading.value = false
    pendingRefresh = false
    pendingForce = false
  }

  function schedule(delay: number) {
    clearTimeout(timer)
    if (!disposed && options.key.value && isDocumentVisible())
      timer = setTimeout(() => {
        void refresh()
      }, delay)
  }

  async function refresh(force = false) {
    if (disposed || !options.key.value || !isDocumentVisible()) return
    pendingForce ||= force
    if (controller) {
      pendingRefresh = true
      return
    }
    // Coalesce clicks, visibility changes and event bursts.
    // User refreshes retain a short coalescing gap and the failure backoff.
    // Force validates the body; it must not turn repeated clicks into a loop.
    const remaining = Math.max((options.minInterval ?? 250) - (Date.now() - lastStart), allowedAt - Date.now())
    if (remaining > 0) {
      schedule(remaining)
      return
    }
    clearTimeout(timer)
    const key = options.key.value
    const version = generation
    const request = new AbortController()
    controller = request
    pendingRefresh = false
    force = pendingForce
    pendingForce = false
    const timeout = setTimeout(() => request.abort(new Error('Request timed out')), 15_000)
    loading.value = true
    lastStart = Date.now()
    try {
      const resource = options.resource?.(key)
      // A first read needs a body regardless of its revision; its ETag seeds
      // the index without a redundant metadata request. Explicit refreshes
      // likewise validate the representation directly.
      const sameScope = resource && observed?.scope === captureResourceObservation(resource).scope
      if (resource && sameScope && observed?.token && data.value !== null && !force)
        await checkResourceVersions([resource], request.signal)
      if (
        !force &&
        resource &&
        sameScope &&
        observed?.token &&
        data.value !== null &&
        options.interval() === null &&
        !error.value &&
        canReuseResource(resource, observed.token)
      )
        return
      let responseObservation: ReturnType<typeof captureResourceObservation> | undefined
      const result = await options.load(key, request.signal, data.value, force, (value) => {
        responseObservation = value
      })
      if (version === generation && key === options.key.value && !disposed) {
        data.value = result
        error.value = ''
        failures = 0
        allowedAt = Date.now() + (options.minInterval ?? 250)
        // Only the HTTP response knows which version this body represents.
        // A concurrent revision check must not label an older body current.
        observed = responseObservation
      }
    } catch (cause) {
      if (version === generation && !disposed) {
        error.value = cause instanceof Error ? cause.message : String(cause)
        failures = Math.min(failures + 1, 6)
        allowedAt = Date.now() + Math.min(60_000, 5000 * 2 ** (failures - 1))
      }
    } finally {
      clearTimeout(timeout)
      if (version === generation) {
        controller = null
        loading.value = false
        // An event received during a read must get a trailing read. Otherwise
        // a terminal update can be lost until the slow idle heartbeat.
        const interval = options.interval()
        if (error.value) schedule(Math.max(allowedAt - Date.now(), interval ?? 0))
        else if (pendingRefresh) schedule(0)
        else if (interval !== null) schedule(interval)
      }
    }
  }

  watch(
    options.key,
    () => {
      cancel()
      unsubscribe?.()
      const resource = options.key.value ? options.resource?.(options.key.value) : undefined
      unsubscribe = resource
        ? subscribeResource(resource, () => {
            void refresh()
          })
        : undefined
      failures = 0
      allowedAt = 0
      data.value = null
      observed = undefined
      lastStart = 0
      error.value = ''
      void refresh()
    },
    { immediate: true, flush: 'sync' },
  )
  const visibility = () => {
    if (!isDocumentVisible()) cancel()
    else void refresh()
  }
  document.addEventListener('visibilitychange', visibility)
  onBeforeUnmount(() => {
    disposed = true
    cancel()
    unsubscribe?.()
    document.removeEventListener('visibilitychange', visibility)
  })
  return { data, error, loading, refresh: () => refresh(true) }
}
