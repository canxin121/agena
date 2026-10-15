/**
 * Where Cmd+I should put the caret.
 *
 * Every workspace pane keeps its own composer in the same document, and a
 * background tab keeps its composer mounted but hidden. A plain document query
 * focuses the first composer - or a hidden one, which cannot take focus at all -
 * so the focused pane's composer wins whenever it exists.
 */

export function composerFocusSelectors(windowId?: string | null): string[] {
  const id = String(windowId || '').trim()
  const escaped = id && typeof CSS !== 'undefined' && typeof CSS.escape === 'function' ? CSS.escape(id) : id
  return [
    ...(escaped ? [`[data-workspace-pane-window="${escaped}"] [data-chat-input="true"]`] : []),
    '[data-chat-input="true"]',
  ]
}

export function pickComposerFocusTarget(opts: {
  windowId?: string | null
  queryAll: (selector: string) => Iterable<HTMLElement>
  isVisible?: (element: HTMLElement) => boolean
}): HTMLElement | null {
  const isVisible = opts.isVisible || (() => true)
  for (const selector of composerFocusSelectors(opts.windowId)) {
    for (const element of opts.queryAll(selector)) {
      if (isVisible(element)) return element
    }
  }

  return null
}
