/**
 * Input-method (IME) keyboard events.
 *
 * A browser reports key events produced while an IME (Chinese, Japanese,
 * Korean, ...) is composing with `isComposing: true` and the historical
 * `keyCode 229`. The `Enter` that confirms a candidate therefore reaches key
 * handlers looking like an ordinary press even though the user only asked the
 * IME to commit text.
 *
 * Composition keys must never activate a control, submit an answer, advance a
 * wizard page, close a surface, move the transcript cursor, or trigger a
 * global shortcut: the committed text still arrives through `input` /
 * `compositionend`, and the next real `Enter` performs the action.
 *
 * WebKit usually reports `isComposing` on the confirming keydown as well, but
 * older builds only expose `keyCode 229`, so both signals are checked.
 */
export type CompositionKeyEvent = {
  isComposing?: boolean
  keyCode?: number
}

export function isImeComposing(event: CompositionKeyEvent | null | undefined): boolean {
  if (!event) return false
  return event.isComposing === true || event.keyCode === 229
}

/**
 * Engines disagree about when a composition ends relative to the key that
 * confirmed a candidate. Chrome and Safari report their confirmation on the
 * `keydown` itself, but some builds deliver the bare `Enter` right after
 * `compositionend`, when the composition state is already gone. That key still
 * belongs to the input method, so it is recognized by timing instead: a bare
 * `Enter` that follows a composition end within this window.
 */
export const IME_COMMIT_ENTER_WINDOW_MS = 50

/** The fields needed to place a key press on the composition-end timeline. */
export type CommitKeyEvent = {
  key?: string
  timeStamp?: number
  altKey?: boolean
  ctrlKey?: boolean
  metaKey?: boolean
}

/**
 * Whether a key press is the `Enter` that confirmed an input-method candidate
 * and was delivered just after the composition ended rather than while it was
 * still reported as composing.
 */
export function isImeCommitEnter(
  event: CommitKeyEvent | null | undefined,
  compositionEndedAt: number,
): boolean {
  if (!event || event.key !== 'Enter') return false
  if (event.altKey || event.ctrlKey || event.metaKey) return false
  const timeStamp = event.timeStamp ?? 0
  if (!Number.isFinite(timeStamp) || timeStamp <= 0) return false
  if (!Number.isFinite(compositionEndedAt) || compositionEndedAt <= 0) return false
  return timeStamp - compositionEndedAt <= IME_COMMIT_ENTER_WINDOW_MS
}

/**
 * Whether an event target owns a text editing context an input method can
 * compose into.
 */
function isTextEntryTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  if (target.isContentEditable) return true
  return target.tagName === 'INPUT' || target.tagName === 'TEXTAREA'
}

/**
 * Keep composition-owned key presses away from application key handlers.
 *
 * Composition keys are not application input. The `Enter` that confirms a
 * candidate must only commit the composed text, yet many surfaces handle
 * `Enter` themselves: form fields that save on enter, dialogs that submit,
 * search boxes that jump to a match, list controls that activate the focused
 * row, and global shortcuts. Guarding each handler by hand is easy to miss,
 * so the listeners are stopped once, at the window, in the capture phase,
 * before any handler runs.
 *
 * Only the propagation is stopped, never the default action: the input method
 * keeps the key, and the committed text still arrives through
 * `input`/`compositionend`. The guard is limited to text-entry targets, the
 * only places an IME composes, so ordinary keys keep their normal routing.
 *
 * Returns a disposer that removes the listener again.
 */
export function installImeCompositionGuard(root: Window = window): () => void {
  // Track when a composition ended so a confirmation key delivered after it is
  // still attributed to the input method.
  let compositionEndedAt = Number.NEGATIVE_INFINITY
  const onCompositionEnd = (event: Event) => {
    compositionEndedAt = event.timeStamp
  }
  const listener = (event: Event) => {
    if (!(event instanceof KeyboardEvent)) return
    if (!isTextEntryTarget(event.target)) return
    if (!isImeComposing(event) && !isImeCommitEnter(event, compositionEndedAt)) return
    event.stopPropagation()
  }
  root.addEventListener('compositionend', onCompositionEnd, true)
  root.addEventListener('keydown', listener, true)
  return () => {
    root.removeEventListener('compositionend', onCompositionEnd, true)
    root.removeEventListener('keydown', listener, true)
  }
}
