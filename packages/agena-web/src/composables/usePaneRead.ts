import { onMounted, onScopeDispose, ref, watch } from 'vue'
import { usePaneVisibility } from './usePaneVisibility'
import { createRevalidator } from '../lib/revalidation'
import { limitBackgroundReads } from '../lib/backgroundReads'
import { captureResourceObservation } from '../lib/resourceSync'

/** Serialize manual display refreshes, keep existing data, and defer hidden reads. */
export function usePaneRead<T>(read: (signal: AbortSignal) => Promise<T>, apply: (value: T) => void) {
  const visible = usePaneVisibility()
  const loading = ref(false)
  const error = ref('')
  let active: AbortController | undefined
  const queue = createRevalidator(
    async () => {
      const controller = new AbortController()
      active = controller
      const scope = captureResourceObservation('sessions').scope
      loading.value = true
      error.value = ''
      try {
        const result = await limitBackgroundReads(
          () => read(AbortSignal.any([controller.signal, AbortSignal.timeout(30_000)])),
          controller.signal,
        )
        if (!controller.signal.aborted && scope === captureResourceObservation('sessions').scope) apply(result)
      } catch (reason) {
        if (controller.signal.aborted || scope !== captureResourceObservation('sessions').scope) return
        error.value = reason instanceof Error ? reason.message : String(reason)
        throw reason
      } finally {
        if (active === controller) {
          active = undefined
          loading.value = false
        }
      }
    },
    { enabled: () => visible.value, intervalMs: 0 },
  )
  const refresh = () => {
    queue.invalidate(0)
    return queue.refresh().catch(() => {})
  }
  onMounted(refresh)
  watch(
    visible,
    (shown) => {
      if (shown) queue.resume()
      else {
        queue.pause()
        if (active) {
          active.abort()
          active = undefined
          loading.value = false
          queue.invalidate(0)
        }
      }
    },
    { flush: 'sync' },
  )
  onScopeDispose(() => {
    queue.dispose()
    active?.abort()
  })
  return { loading, error, refresh }
}
