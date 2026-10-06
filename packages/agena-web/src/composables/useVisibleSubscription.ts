import { onScopeDispose, watch, type Ref } from 'vue'
import { usePaneVisibility } from './usePaneVisibility'

/** Keep a shared source alive only while this pane is actually visible. */
export function useVisibleSubscription(
  retain: () => () => void,
  visible: Readonly<Ref<boolean>> = usePaneVisibility(),
) {
  let release: (() => void) | undefined
  watch(
    visible,
    (shown) => {
      if (shown) release ??= retain()
      else {
        release?.()
        release = undefined
      }
    },
    { immediate: true, flush: 'sync' },
  )
  onScopeDispose(() => {
    release?.()
    release = undefined
  })
}
