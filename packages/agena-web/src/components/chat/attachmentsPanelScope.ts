/**
 * Every workspace pane keeps its own attachments panel in the same document, so
 * Escape may only close the panel belonging to the pane the user is working in.
 */

const PANE_SELECTOR = '[data-workspace-pane-window]'

type ClosestLike = { closest: (selector: string) => unknown } | null | undefined

export function escapeBelongsToPanel(opts: { panel: ClosestLike; target: ClosestLike; paneFocused: boolean }): boolean {
  const panelPane = opts.panel?.closest(PANE_SELECTOR) || null
  // No pane wrapper (compact layout, embedded shell): keep the plain behaviour.
  if (!panelPane) return true

  const targetPane = opts.target?.closest(PANE_SELECTOR) || null
  if (targetPane) return targetPane === panelPane

  // Nothing inside a pane owns focus (for example a click on empty chrome): the
  // pane the user last worked in still owns the keystroke.
  return opts.paneFocused
}
