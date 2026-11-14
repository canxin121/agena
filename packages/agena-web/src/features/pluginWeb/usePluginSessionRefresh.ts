import { ref, watch } from 'vue'
import type { PluginPageCatalogItem } from './sdk'

/** Session pages can mount from an already-populated shared catalog. Refresh
 * immediately and keep a shared session request's late error with its own page.
 */
export function usePluginSessionRefresh(
  scope: () => PluginPageCatalogItem['scope'] | undefined,
  refresh: () => Promise<void>,
) {
  const error = ref('')
  watch(
    scope,
    (value, _previous, onCleanup) => {
      error.value = ''
      if (value !== 'session') return
      let current = true
      onCleanup(() => {
        current = false
      })
      void refresh().catch((cause) => {
        if (current) error.value = cause instanceof Error ? cause.message : String(cause)
      })
    },
    { immediate: true },
  )
  return error
}
