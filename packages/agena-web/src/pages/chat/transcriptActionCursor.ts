import { isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'

export type ActionCursorDirection = 'left' | 'right' | 'up' | 'down'
export type ActionCursorRect = { left: number; right: number; top: number; bottom: number }

/** Choose a spatial neighbour. There is no wrapping at the edge of a part. */
export function actionCursorNeighbour<T>(
  origin: ActionCursorRect,
  candidates: readonly { value: T; rect: ActionCursorRect }[],
  direction: ActionCursorDirection,
  preferredX = origin.left,
): T | null {
  const vertical = direction === 'up' || direction === 'down'
  const forward = direction === 'down' || direction === 'right'
  const x = (origin.left + origin.right) / 2
  const y = (origin.top + origin.bottom) / 2
  let best: T | null = null
  let bestScore = Infinity
  for (const { value, rect } of candidates) {
    const cx = (rect.left + rect.right) / 2
    const cy = (rect.top + rect.bottom) / 2
    const sameRow = Math.min(origin.bottom, rect.bottom) - Math.max(origin.top, rect.top) > 2
    if (vertical ? sameRow || (forward ? cy <= y + 2 : cy >= y - 2) : !sameRow || (forward ? cx <= x + 2 : cx >= x - 2))
      continue
    const primary = vertical ? Math.abs(cy - y) : Math.abs(cx - x)
    const secondary = vertical ? Math.max(rect.left - preferredX, preferredX - rect.right, 0) : Math.abs(cy - y)
    // Prefer the next visual row, then the control nearest the desired column.
    const score = primary * 1000 + secondary
    if (score < bestScore) {
      bestScore = score
      best = value
    }
  }
  return best
}

const selector =
  'button, a[href], input:not([type="hidden"]), textarea, select, summary, [role="button"], [role="option"]'

export function transcriptActionControls(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(selector)).filter(
    (control) =>
      !control.matches(':disabled, [aria-disabled="true"], [tabindex="-1"]') &&
      !control.closest('[inert], .xterm, [data-transcript-native-keys="true"]') &&
      isVisibleKeyboardTarget(control),
  )
}

export function transcriptActionControl(element: Element | null): HTMLElement | null {
  return (
    element?.closest<HTMLElement>(selector) ||
    element?.closest('label')?.querySelector<HTMLElement>('input[type="radio"], input[type="checkbox"]') ||
    null
  )
}

export function transcriptActionRect(control: HTMLElement): DOMRect {
  // A choice is one target, including its label and description.
  return (
    control.matches('input[type="radio"], input[type="checkbox"]') ? control.closest('label') || control : control
  ).getBoundingClientRect()
}

export function focusTranscriptAction(control: HTMLElement) {
  control.focus({ preventScroll: true })
  control.scrollIntoView({ block: 'nearest', inline: 'nearest' })
}
