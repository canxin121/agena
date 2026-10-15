import { nextTick, watch } from 'vue'
import { focusKeyboardRegion, hasOpenKeyboardOverlay, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'

const overlaySources = new WeakMap<HTMLElement, HTMLElement>()
export function rememberKeyboardOverlaySource(overlay: HTMLElement, source: HTMLElement) {
  overlaySources.set(overlay, source)
}

function sourceOutsideClosingMenus(element: HTMLElement | null): HTMLElement | null {
  const visited = new Set<HTMLElement>()
  while (element && !visited.has(element)) {
    visited.add(element)
    const overlay = element.closest<HTMLElement>('[data-keyboard-overlay="true"]')
    const source = overlay && overlaySources.get(overlay)
    if (!source) break
    element = source
  }
  return element
}

/** Radix restores the original control; retain a nearby destination if that control is deleted. */
export function useDialogReturnFocus(isOpen: () => boolean): (event: Event) => void {
  let source: HTMLElement | null = null
  let sourceWasRedirected = false
  let collection: HTMLElement | null = null
  let region: HTMLElement | null = null
  let index = 0
  watch(
    isOpen,
    (open) => {
      if (!open || typeof document === 'undefined') return
      const active = document.activeElement instanceof HTMLElement ? document.activeElement : null
      source = sourceOutsideClosingMenus(active)
      sourceWasRedirected = source !== active
      collection = source?.closest<HTMLElement>('[data-keyboard-collection]') || null
      region = source?.closest<HTMLElement>('[data-keyboard-region]') || null
      index = collection
        ? Array.from(collection.querySelectorAll('[data-keyboard-item]')).findIndex((row) => row.contains(source))
        : 0
    },
    { immediate: true, flush: 'sync' },
  )
  return (event) => {
    // Radix temporarily aria-hides the page until its focus scope unmounts.
    // That marker must not make a surviving source look deleted or collapsed.
    if (
      !source ||
      (!sourceWasRedirected &&
        source.isConnected &&
        source.getClientRects().length > 0 &&
        getComputedStyle(source).visibility !== 'hidden' &&
        !source.closest('[inert]'))
    )
      return
    event.preventDefault()
    void nextTick(() => {
      if (hasOpenKeyboardOverlay()) return
      const active = document.activeElement
      if (active instanceof HTMLElement && active !== document.body && isVisibleKeyboardTarget(active)) return
      if (source && isVisibleKeyboardTarget(source)) {
        source.focus({ preventScroll: true })
        return
      }
      if (collection && isVisibleKeyboardTarget(collection)) {
        const rows = Array.from(collection.querySelectorAll<HTMLElement>('[data-keyboard-item]'))
        const row = rows[Math.min(Math.max(0, index), rows.length - 1)]
        const primary = row?.matches('button, a[href]')
          ? row
          : row?.querySelector<HTMLElement>('[data-oc-list-item-primary], [data-keyboard-primary]')
        if (primary && isVisibleKeyboardTarget(primary)) {
          primary.focus({ preventScroll: true })
          return
        }
        collection.focus({ preventScroll: true })
        return
      }
      const fallback =
        region && isVisibleKeyboardTarget(region)
          ? region
          : Array.from(document.querySelectorAll<HTMLElement>('[data-keyboard-region]')).find(isVisibleKeyboardTarget)
      if (fallback) focusKeyboardRegion(fallback)
    })
  }
}
