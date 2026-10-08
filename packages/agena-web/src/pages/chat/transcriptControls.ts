const NATIVE_CONTROL_SELECTOR =
  'button, a[href], input, textarea, select, summary, [role="button"], [contenteditable="true"], .xterm'

/** Native controls and terminal selection own their pointer gestures. */
export function transcriptNativeControl(target: Element): Element | null {
  return target.closest(NATIVE_CONTROL_SELECTOR)
}

/** Move through a reply's loading controls in their visible DOM order. */
export function focusTranscriptPartControl(root: HTMLElement, direction: -1 | 1, from?: Element | null): boolean {
  const controls = Array.from(root.querySelectorAll<HTMLElement>('[data-part-control="true"]')).filter(
    (control) => !control.matches(':disabled, [aria-disabled="true"]'),
  )
  if (!controls.length) return false
  const current = controls.findIndex((control) => control === from || Boolean(from && control.contains(from)))
  const index =
    current < 0 ? (direction > 0 ? 0 : controls.length - 1) : (current + direction + controls.length) % controls.length
  controls[index]!.focus({ preventScroll: true })
  return true
}
