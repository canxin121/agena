import { isImeComposing } from '@/lib/imeKeyboard'

/** Modified keys and input-method candidates keep their existing owner. */
export function isPlainNavigationKey(event: KeyboardEvent): boolean {
  return !event.defaultPrevented && !isImeComposing(event) && !event.altKey && !event.ctrlKey && !event.metaKey
}

export function preserveImeEscape(event: KeyboardEvent) {
  if (event.key === 'Escape' && isImeComposing(event)) event.stopPropagation()
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
      '[role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"], [data-keyboard-overlay="true"]:not([data-state="closed"]), [data-command-palette="true"]',
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

const optionSelector = '[data-keyboard-option]'

export function enabledKeyboardItems(root: HTMLElement | null, selector = optionSelector): HTMLElement[] {
  return Array.from(root?.querySelectorAll<HTMLElement>(selector) || []).filter(
    (item) => !item.matches(':disabled, [aria-disabled="true"]') && isVisibleKeyboardTarget(item),
  )
}

export function focusKeyboardItem(item: HTMLElement | undefined | null): boolean {
  if (!item) return false
  item.focus({ preventScroll: true })
  if (item.ownerDocument.activeElement !== item) return false
  item.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  return true
}

export function focusDialogOnOpen(event: Event, selector: string | undefined) {
  if (!selector || !(event.target instanceof HTMLElement)) return
  const target = event.target.querySelector<HTMLElement>(selector)
  target?.focus({ preventScroll: true })
  if (target && target.ownerDocument.activeElement === target) event.preventDefault()
}

/** Move between options, or enter the list from its search field or container. */
export function handleOptionKeydown(
  event: KeyboardEvent,
  root: HTMLElement | null,
  entry: HTMLElement | null = null,
): boolean {
  if (!root || !isPlainNavigationKey(event) || event.shiftKey) return false
  const vertical = event.key === 'ArrowDown' || event.key === 'ArrowUp'
  if (!vertical && event.key !== 'Home' && event.key !== 'End') return false
  const target = event.target
  if (!(target instanceof HTMLElement)) return false
  if (target !== root && !target.matches(optionSelector) && !(vertical && target === entry)) return false

  const items = enabledKeyboardItems(root)
  const index = navigationIndex(event.key, items.indexOf(target), items.length)
  if (index === null || !focusKeyboardItem(items[index])) return false
  event.preventDefault()
  event.stopPropagation()
  return true
}

export function optionOpenEdge(event: KeyboardEvent): 'first' | 'last' | undefined {
  if (!isPlainNavigationKey(event) || event.shiftKey) return
  if (event.key === 'ArrowDown') return 'first'
  if (event.key === 'ArrowUp') return 'last'
}

export function handleTabKeydown(event: KeyboardEvent, activate = true) {
  if (!isPlainNavigationKey(event) || event.shiftKey) return
  const root = event.currentTarget
  if (!(root instanceof HTMLElement)) return
  let key = event.key
  if (key === 'ArrowLeft' || key === 'ArrowRight') {
    if (getComputedStyle(root).direction === 'rtl') key = key === 'ArrowLeft' ? 'ArrowRight' : 'ArrowLeft'
  } else if (key !== 'Home' && key !== 'End') return

  const items = enabledKeyboardItems(root, activate ? '[role="tab"]' : optionSelector)
  const current = items.findIndex((item) => item === event.target)
  if (current < 0) return
  const index = navigationIndex(key, current, items.length)
  if (index === null || !focusKeyboardItem(items[index])) return
  event.preventDefault()
  event.stopPropagation()
  if (activate && index !== current) items[index]?.click()
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
