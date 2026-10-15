import { hasOpenKeyboardOverlay, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'

/** Wait for a lazy page to mount, but abandon the request as soon as the user moves on. */
export function deferKeyboardFocus(
  findTarget: () => HTMLElement | null,
  origin: Element | null = document.activeElement,
): () => void {
  let done = false
  let timer: ReturnType<typeof setTimeout> | undefined
  const observer = new MutationObserver(attempt)
  function cancel() {
    if (done) return
    done = true
    observer.disconnect()
    if (timer) clearTimeout(timer)
    document.removeEventListener('focusin', onFocus, true)
    window.removeEventListener('pointerdown', cancel, true)
    window.removeEventListener('keydown', cancel, true)
  }
  function onFocus(event: FocusEvent) {
    if (event.target !== origin && event.target !== document.body) cancel()
  }
  function attempt() {
    if (done || hasOpenKeyboardOverlay()) return
    const active = document.activeElement
    if (active && active !== document.body && active !== origin) {
      cancel()
      return
    }
    const target = findTarget()
    if (!target || !isVisibleKeyboardTarget(target)) return
    cancel()
    target.focus({ preventScroll: true })
  }
  observer.observe(document.body, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['class', 'style', 'hidden', 'inert', 'aria-busy', 'data-keyboard-route'],
  })
  document.addEventListener('focusin', onFocus, true)
  window.addEventListener('pointerdown', cancel, true)
  window.addEventListener('keydown', cancel, true)
  timer = setTimeout(cancel, 10000)
  attempt()
  return cancel
}
