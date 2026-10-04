import { onBeforeUnmount, ref, shallowRef, watch, type Ref } from 'vue'

/** One cancellable read per visible resource; key changes discard late results. */
export function useVisibleResource<T>(options: {
  key: Ref<string>
  interval: () => number
  load: (key: string, signal: AbortSignal, previous: T | null) => Promise<T>
}) {
  const data = shallowRef<T | null>(null)
  const error = ref('')
  const loading = ref(false)
  let controller: AbortController | null = null
  let timer: ReturnType<typeof setTimeout> | undefined
  let generation = 0
  let disposed = false
  let lastStart = 0

  function cancel() {
    generation++
    clearTimeout(timer)
    controller?.abort()
    controller = null
    loading.value = false
  }

  function schedule(delay: number) {
    clearTimeout(timer)
    if (!disposed && options.key.value && !document.hidden) timer = setTimeout(refresh, delay)
  }

  async function refresh() {
    if (disposed || !options.key.value || document.hidden || controller) return
    // Coalesce clicks, visibility changes and event bursts.
    const remaining = 750 - (Date.now() - lastStart)
    if (remaining > 0) {
      schedule(remaining)
      return
    }
    clearTimeout(timer)
    const key = options.key.value
    const version = generation
    const request = new AbortController()
    controller = request
    const timeout = setTimeout(() => request.abort(new Error('Request timed out')), 15_000)
    loading.value = true
    lastStart = Date.now()
    try {
      const result = await options.load(key, request.signal, data.value)
      if (version === generation && key === options.key.value && !disposed) {
        data.value = result
        error.value = ''
      }
    } catch (cause) {
      if (version === generation && !disposed) error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      clearTimeout(timeout)
      if (version === generation) {
        controller = null
        loading.value = false
        schedule(error.value ? Math.max(30_000, options.interval()) : options.interval())
      }
    }
  }

  watch(
    options.key,
    () => {
      cancel()
      data.value = null
      error.value = ''
      void refresh()
    },
    { immediate: true, flush: 'sync' },
  )
  const visibility = () => {
    if (document.hidden) cancel()
    else void refresh()
  }
  document.addEventListener('visibilitychange', visibility)
  onBeforeUnmount(() => {
    disposed = true
    cancel()
    document.removeEventListener('visibilitychange', visibility)
  })
  return { data, error, loading, refresh }
}
