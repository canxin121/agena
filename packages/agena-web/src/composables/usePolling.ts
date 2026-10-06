import { onMounted, onScopeDispose, watch } from 'vue'
import { usePaneVisibility } from './usePaneVisibility'
import { createRevalidator } from '@/lib/revalidation'

/** Recurring display work is serialized, backs off on errors, and pauses hidden. */
export function usePolling(read: () => Promise<void>, intervalMs: number, enabled?: () => boolean) {
  const visible = usePaneVisibility()
  const queue = createRevalidator(read, {
    intervalMs,
    retryMs: intervalMs,
    pollIntervalMs: intervalMs,
    enabled: () => visible.value && enabled?.() !== false,
  })
  const visibility = () => {
    if (visible.value) queue.resume()
    else queue.pause()
  }
  onMounted(() => {
    queue.invalidate(0)
  })
  watch(visible, visibility, { flush: 'sync' })
  onScopeDispose(() => {
    queue.dispose()
  })
  return queue
}
