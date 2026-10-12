/**
 * The transcript owns a copy handler on the document, and a mouse selection
 * stays in the pane that made it. A copy somewhere else - the other pane's
 * transcript, the composer - must not be answered with this pane's stale
 * selection, so the current selection has to be inside this pane's scroll area.
 */

export function copySelectionBelongsToRoot(
  root: { contains: (other: Node | null) => boolean } | null | undefined,
  selection: { rangeCount: number; anchorNode: Node | null } | null | undefined,
): boolean {
  if (!root) return false
  if (!selection || selection.rangeCount <= 0) return false
  const anchor = selection.anchorNode
  if (!anchor) return false
  return root.contains(anchor)
}
