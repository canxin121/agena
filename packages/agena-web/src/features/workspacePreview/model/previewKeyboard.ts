import { isImeComposing } from '@/lib/imeKeyboard'

export type ViewportResizeAxis = 'x' | 'y' | 'both'

export function keyboardViewportSize(
  event: KeyboardEvent,
  axis: ViewportResizeAxis,
  width: number,
  height: number,
): { width: number; height: number } | null {
  if (event.defaultPrevented || isImeComposing(event) || event.altKey || event.ctrlKey || event.metaKey) return null
  const step = event.shiftKey ? 10 : 1
  if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return null
  const horizontal = axis !== 'y'
  const vertical = axis !== 'x'
  if (event.key === 'Home' || event.key === 'End') {
    const end = event.key === 'End'
    return { width: horizontal ? (end ? 4096 : 240) : width, height: vertical ? (end ? 4096 : 180) : height }
  }
  if (horizontal && event.key === 'ArrowLeft') width -= step
  else if (horizontal && event.key === 'ArrowRight') width += step
  else if (vertical && event.key === 'ArrowUp') height -= step
  else if (vertical && event.key === 'ArrowDown') height += step
  else return null
  return { width: Math.max(240, Math.min(4096, width)), height: Math.max(180, Math.min(4096, height)) }
}

/** A same-origin frame can cooperate directly, without a postMessage command channel. */
export function installPreviewKeyboardBridge(
  frame: HTMLIFrameElement,
  leave: (backwards: boolean) => void,
): (() => void) | null {
  try {
    const win = frame.contentWindow
    if (!win || !frame.contentDocument?.body) return null
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'F6' || event.altKey || event.ctrlKey || event.metaKey || isImeComposing(event)) return
      event.preventDefault()
      event.stopImmediatePropagation()
      leave(event.shiftKey)
    }
    win.addEventListener('keydown', onKey, true)
    return () => win.removeEventListener('keydown', onKey, true)
  } catch {
    // Browsers disallow inspecting cross-origin frames. The UI offers a separate tab.
    return null
  }
}
