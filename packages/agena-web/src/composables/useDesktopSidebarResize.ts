import { onBeforeUnmount } from 'vue'
import { clampDesktopSidebarWidth, DESKTOP_SIDEBAR_MAX_WIDTH, DESKTOP_SIDEBAR_MIN_WIDTH, useUiStore } from '@/stores/ui'
import { isPlainNavigationKey } from '@/lib/keyboardNavigation'

export type DesktopSidebarResizeOptions = {
  deferCommit?: boolean
  onStart?: () => void
  onPreviewWidth?: (width: number) => void
  onEnd?: (finalWidth: number) => void
}

function inlineDirection(target: Element | null): number {
  return target && getComputedStyle(target).direction === 'rtl' ? -1 : 1
}

export function useDesktopSidebarResize() {
  const ui = useUiStore()
  let finishResize: (() => void) | undefined
  onBeforeUnmount(() => finishResize?.())

  function resizeDesktopSidebarFromKeyboard(event: KeyboardEvent) {
    if (ui.isCompactLayout || !isPlainNavigationKey(event)) return
    const step = (event.shiftKey ? 64 : 16) * inlineDirection(event.currentTarget as Element | null)
    let width: number | null = null
    if (event.key === 'ArrowLeft') width = ui.sidebarWidth - step
    if (event.key === 'ArrowRight') width = ui.sidebarWidth + step
    if (event.key === 'Home') width = DESKTOP_SIDEBAR_MIN_WIDTH
    if (event.key === 'End') width = DESKTOP_SIDEBAR_MAX_WIDTH
    if (width === null) return
    event.preventDefault()
    event.stopPropagation()
    ui.sidebarWidth = clampDesktopSidebarWidth(width)
  }

  function startDesktopSidebarResize(e: PointerEvent, options?: DesktopSidebarResizeOptions) {
    if (ui.isCompactLayout || e.button !== 0 || finishResize) return

    const target = e.currentTarget as HTMLElement | null
    if (!target) return

    const startX = e.clientX
    const startW = ui.sidebarWidth
    const direction = inlineDirection(target)
    const deferCommit = Boolean(options?.deferCommit)
    let previewWidth = startW

    target.setPointerCapture(e.pointerId)
    options?.onStart?.()
    options?.onPreviewWidth?.(startW)

    const style = document.body.style
    const previousCursor = style.cursor
    const previousUserSelect = style.userSelect
    style.cursor = 'col-resize'
    style.userSelect = 'none'

    const move = (ev: PointerEvent) => {
      if (ev.pointerId !== e.pointerId) return
      const next = clampDesktopSidebarWidth(startW + (ev.clientX - startX) * direction)
      previewWidth = next

      options?.onPreviewWidth?.(next)
      if (!deferCommit) {
        ui.sidebarWidth = next
      }
    }

    const up = (ev: PointerEvent) => {
      if (ev.pointerId === e.pointerId) finishResize?.()
    }

    finishResize = () => {
      finishResize = undefined
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
      target.removeEventListener('lostpointercapture', up)

      if (deferCommit) {
        ui.sidebarWidth = previewWidth
      }

      options?.onEnd?.(previewWidth)

      style.cursor = previousCursor
      style.userSelect = previousUserSelect

      try {
        if (target.hasPointerCapture(e.pointerId)) {
          target.releasePointerCapture(e.pointerId)
        }
      } catch {
        // ignore
      }
    }

    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)
    target.addEventListener('lostpointercapture', up)
  }

  return { startDesktopSidebarResize, resizeDesktopSidebarFromKeyboard }
}
