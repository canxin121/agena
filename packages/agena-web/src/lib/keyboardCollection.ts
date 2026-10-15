import { nextTick, type ObjectDirective } from 'vue'
import { isImeComposing } from '@/lib/imeKeyboard'
import { isTextEditingTarget, isVisibleKeyboardTarget, navigationIndex } from '@/lib/keyboardNavigation'

export type KeyboardCollectionOptions = { tree?: boolean }

/** Delegated navigation keeps secondary row actions reachable with Tab. */
export function installKeyboardCollection(root: HTMLElement, options: KeyboardCollectionOptions = {}): () => void {
  let currentKey = ''
  let currentIndex = 0
  let lastPrimary: HTMLElement | null = null
  let query = ''
  let queryTime = 0
  const secondaryTabStops = new WeakMap<HTMLElement, number>()
  const primaryFor = (row: HTMLElement): HTMLElement | null =>
    row.matches('button, a[href], [role="treeitem"]')
      ? row
      : row.querySelector<HTMLElement>('[data-oc-list-item-primary], [data-keyboard-primary]') ||
        row.querySelector<HTMLElement>('button:not(:disabled)')
  const rows = () =>
    Array.from(root.querySelectorAll<HTMLElement>('[data-keyboard-item]')).filter((row) => {
      const primary = primaryFor(row)
      return primary && !primary.matches(':disabled') && isVisibleKeyboardTarget(primary)
    })
  const level = (row: HTMLElement) => Number(row.dataset.keyboardLevel || 1)
  const rowFor = (target: EventTarget | null) =>
    target instanceof Element ? target.closest<HTMLElement>('[data-keyboard-item]') : null
  function focus(row: HTMLElement | undefined) {
    if (!row) return
    const primary = primaryFor(row)
    primary?.focus({ preventScroll: true })
    primary?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }
  function sync() {
    if (!isVisibleKeyboardTarget(root)) return
    const items = rows()
    const baseLevel = Math.min(...items.map(level))
    const activeRow = rowFor(document.activeElement)
    let selected =
      activeRow && items.includes(activeRow) ? activeRow : items.find((row) => row.dataset.keyboardItem === currentKey)
    if (!currentKey) selected ||= items.find((row) => row.dataset.keyboardSelected === 'true')
    selected ||= items[Math.min(currentIndex, items.length - 1)]
    root.tabIndex = items.length ? -1 : 0
    for (const row of items) {
      const primary = primaryFor(row)!
      primary.tabIndex = row === selected ? 0 : -1
      for (const control of row.querySelectorAll<HTMLElement>('button, a[href], input, select, textarea, [tabindex]')) {
        if (control === primary) continue
        if (!secondaryTabStops.has(control)) secondaryTabStops.set(control, control.tabIndex)
        control.tabIndex = row === selected ? secondaryTabStops.get(control)! : -1
      }
      if (options.tree) {
        primary.setAttribute('role', 'treeitem')
        primary.setAttribute('aria-level', String(level(row) - baseLevel + 1))
        if (row.dataset.keyboardSelected !== undefined)
          primary.setAttribute('aria-selected', row.dataset.keyboardSelected)
        const expanded = row.dataset.keyboardExpanded
        if (expanded !== undefined) primary.setAttribute('aria-expanded', expanded)
        else primary.removeAttribute('aria-expanded')
      }
    }
    // Removing the focused row must not strand focus on body. A new surface or
    // a user-selected control always takes precedence over this fallback.
    if (lastPrimary && !lastPrimary.isConnected && document.activeElement === document.body) {
      if (selected) focus(selected)
      else root.focus({ preventScroll: true })
    }
    if (selected) {
      currentKey = selected.dataset.keyboardItem || ''
      currentIndex = items.indexOf(selected)
      lastPrimary = primaryFor(selected)
    }
  }
  function onFocus(event: FocusEvent) {
    const row = rowFor(event.target)
    if (!row || !root.contains(row)) return
    currentKey = row.dataset.keyboardItem || ''
    currentIndex = rows().indexOf(row)
    lastPrimary = primaryFor(row)
    sync()
  }
  function onKey(event: KeyboardEvent) {
    if (event.defaultPrevented || isImeComposing(event) || isTextEditingTarget(event.target)) return
    const row = rowFor(event.target)
    if (!row || !root.contains(row)) return
    const primary = primaryFor(row)
    if (event.target !== primary) return
    const items = rows()
    const index = items.indexOf(row)
    if (index < 0) return
    let handled = false
    let target: HTMLElement | undefined
    const plain = !event.altKey && !event.ctrlKey && !event.metaKey
    if (plain && ((event.key === 'F10' && event.shiftKey) || event.key === 'ContextMenu')) {
      const menu = row.querySelector<HTMLElement>('[data-keyboard-menu]')
      if (menu) menu.click()
      else {
        const context = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
        row.dispatchEvent(context)
        handled = context.defaultPrevented
      }
      if (menu) handled = true
    } else if (plain && !event.shiftKey && event.key === 'F2') {
      const rename = row.querySelector<HTMLElement>('[data-keyboard-rename]')
      if (rename) {
        rename.click()
        handled = true
      }
    } else if (plain && ['ArrowDown', 'ArrowUp', 'Home', 'End', 'PageDown', 'PageUp'].includes(event.key)) {
      const next =
        event.key === 'PageDown'
          ? Math.min(items.length - 1, index + 10)
          : event.key === 'PageUp'
            ? Math.max(0, index - 10)
            : navigationIndex(event.key, index, items.length)
      if (next !== null) target = items[next]
      if (target && event.shiftKey && row.dataset.keyboardMultiselect === 'true') {
        primaryFor(target)?.dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))
      }
      handled = true
    } else if (plain && !event.shiftKey && options.tree && ['ArrowLeft', 'ArrowRight'].includes(event.key)) {
      const expanded = row.dataset.keyboardExpanded
      const toggle = row.querySelector<HTMLElement>('[data-keyboard-toggle]') || primary
      if (event.key === 'ArrowRight') {
        if (expanded === 'false') toggle?.click()
        else if (level(items[index + 1] || row) > level(row)) target = items[index + 1]
      } else if (expanded === 'true') toggle?.click()
      else
        target = items
          .slice(0, index)
          .reverse()
          .find((candidate) => level(candidate) < level(row))
      handled = true
    } else if (plain && event.key.length === 1 && event.key !== ' ') {
      const now = Date.now()
      query = now - queryTime > 700 ? event.key : query + event.key
      queryTime = now
      const term = query.normalize('NFKC').toLocaleLowerCase()
      const ordered = [...items.slice(index + 1), ...items.slice(0, index + 1)]
      const matches = (candidate: HTMLElement, value: string) =>
        (candidate.dataset.keyboardLabel || primaryFor(candidate)?.textContent || '')
          .trim()
          .normalize('NFKC')
          .toLocaleLowerCase()
          .startsWith(value)
      target = ordered.find((candidate) => matches(candidate, term))
      if (!target && [...term].every((letter) => letter === term[0]))
        target = ordered.find((candidate) => matches(candidate, term[0]!))
      handled = Boolean(target)
    }
    if (handled) {
      event.preventDefault()
      event.stopPropagation()
      focus(target)
      void nextTick(sync)
    }
  }
  if (options.tree) root.setAttribute('role', 'tree')
  root.setAttribute('data-keyboard-collection', '')
  root.addEventListener('keydown', onKey)
  root.addEventListener('focusin', onFocus)
  const observer = new MutationObserver(sync)
  observer.observe(root, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['data-keyboard-expanded', 'data-keyboard-selected', 'data-keyboard-item', 'hidden', 'disabled'],
  })
  sync()
  return () => {
    observer.disconnect()
    root.removeEventListener('keydown', onKey)
    root.removeEventListener('focusin', onFocus)
  }
}

const disposers = new WeakMap<HTMLElement, () => void>()
export const vKeyboardCollection: ObjectDirective<HTMLElement, KeyboardCollectionOptions | undefined> = {
  mounted(root, binding) {
    disposers.set(root, installKeyboardCollection(root, binding.value))
  },
  beforeUnmount(root) {
    disposers.get(root)?.()
    disposers.delete(root)
  },
}
