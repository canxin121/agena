import { onMounted, onScopeDispose } from 'vue'
import { isDocumentVisible } from '@/lib/backgroundReads'
import { createRevalidator } from '@/lib/revalidation'

/** Recurring display work is serialized, backs off on errors, and pauses hidden. */
export function usePolling(read: () => Promise<void>, intervalMs: number, enabled?: () => boolean) {
  const queue = createRevalidator(read, {
    intervalMs,
    retryMs: intervalMs,
    pollIntervalMs: intervalMs,
    enabled: () => isDocumentVisible() && enabled?.() !== false,
  })
  const visibility = () => {
    if (isDocumentVisible()) queue.resume()
    else queue.pause()
  }
  onMounted(() => {
    queue.invalidate(0)
    document.addEventListener('visibilitychange', visibility)
  })
  onScopeDispose(() => {
    queue.dispose()
    document.removeEventListener('visibilitychange', visibility)
  })
  return queue
}
