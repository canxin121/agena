import { onScopeDispose, ref, watch, type Ref } from 'vue'

const listeners = new Map<Element, (visible: boolean) => void>()
let observer: IntersectionObserver | undefined

/** One observer per renderer, shared by all rich transcript bodies. Keep a
 * generous margin so rendering is ready before a message enters the viewport.
 */
export function useNearViewport(element: Ref<HTMLElement | null>) {
  const available = typeof IntersectionObserver !== 'undefined'
  const visible = ref(!available)
  let observedElement: HTMLElement | null = null
  function stopObserving() {
    if (!observedElement) return
    observer?.unobserve(observedElement)
    listeners.delete(observedElement)
    observedElement = null
  }
  const stop = watch(
    element,
    (next) => {
      stopObserving()
      visible.value = !available
      if (!next || !available) return
      if (!observer) {
        observer = new IntersectionObserver(
          (entries) => {
            for (const entry of entries) listeners.get(entry.target)?.(entry.isIntersecting)
          },
          { rootMargin: '800px 0px' },
        )
      }
      listeners.set(next, (value) => {
        visible.value = value
      })
      observedElement = next
      observer.observe(next)
    },
    { flush: 'post', immediate: true },
  )
  onScopeDispose(() => {
    stop()
    // The ref may already have changed while its post-flush watch is still
    // queued. Release the element actually registered with the observer.
    stopObserving()
    if (!listeners.size) {
      observer?.disconnect()
      observer = undefined
    }
  })
  return visible
}
