import { isImeComposing } from '@/lib/imeKeyboard'

/** Modified keys and input-method candidates keep their existing owner. */
export function isPlainNavigationKey(event: KeyboardEvent): boolean {
  return !event.defaultPrevented && !isImeComposing(event) && !event.altKey && !event.ctrlKey && !event.metaKey
}

export function isTextEditingTarget(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    Boolean(target.closest('input, textarea, select, [contenteditable="true"], [role="textbox"]'))
  )
}

export function isVisibleKeyboardTarget(element: HTMLElement): boolean {
  return (
    element.isConnected &&
    element.getClientRects().length > 0 &&
    getComputedStyle(element).visibility !== 'hidden' &&
    !element.closest('[inert], [aria-hidden="true"]')
  )
}

/** Popovers are portalled, so checking only the event target misses their owner. */
export function hasOpenKeyboardOverlay(): boolean {
  return Array.from(
    document.querySelectorAll<HTMLElement>(
      '[role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"], [data-keyboard-overlay="true"], [data-command-palette="true"]',
    ),
  ).some(isVisibleKeyboardTarget)
}

export function navigationIndex(key: string, current: number, count: number): number | null {
  if (count <= 0) return null
  if (key === 'Home') return 0
  if (key === 'End') return count - 1
  if (key === 'ArrowDown' || key === 'ArrowRight') return current < 0 ? 0 : (current + 1) % count
  if (key === 'ArrowUp' || key === 'ArrowLeft') return current < 0 ? count - 1 : (current - 1 + count) % count
  return null
}

export function focusKeyboardRegion(region: HTMLElement): void {
  const activeTab = region.querySelector<HTMLElement>('[data-workspace-tab-primary][aria-selected="true"]')
  const target =
    (activeTab && isVisibleKeyboardTarget(activeTab) ? activeTab : null) ||
    Array.from(
      region.querySelectorAll<HTMLElement>(
        'button:not(:disabled), a[href], input:not(:disabled), textarea:not(:disabled), [tabindex="0"], [contenteditable="true"]',
      ),
    ).find((element) => element.tabIndex >= 0 && isVisibleKeyboardTarget(element))
  const focusTarget = target || region
  focusTarget.focus({ preventScroll: true })
}

export function cycleKeyboardRegion(backwards = false): void {
  const regions = Array.from(document.querySelectorAll<HTMLElement>('[data-keyboard-region]')).filter(
    isVisibleKeyboardTarget,
  )
  const active = document.activeElement
  const current = regions.findIndex((region) => region === active || region.contains(active))
  const index = navigationIndex(backwards ? 'ArrowUp' : 'ArrowDown', current, regions.length)
  if (index !== null) focusKeyboardRegion(regions[index]!)
}
