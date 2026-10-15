import { useUiStore } from '@/stores/ui'
import { isPlainNavigationKey } from '@/lib/keyboardNavigation'

const SIDEBAR_MIN_WIDTH = 280
const SIDEBAR_MAX_WIDTH = 520

type DesktopSidebarResizeOptions = {
  deferCommit?: boolean
  onStart?: () => void
  onPreviewWidth?: (width: number) => void
  onEnd?: (finalWidth: number) => void
}

export function useDesktopSidebarResize() {
  const ui = useUiStore()

  function resizeDesktopSidebarFromKeyboard(event: KeyboardEvent) {
    if (ui.isCompactLayout || !ui.isSidebarOpen || !isPlainNavigationKey(event)) return
    const step = event.shiftKey ? 64 : 16
    let width: number | null = null
    if (event.key === 'ArrowLeft') width = ui.sidebarWidth - step
    if (event.key === 'ArrowRight') width = ui.sidebarWidth + step
    if (event.key === 'Home') width = SIDEBAR_MIN_WIDTH
    if (event.key === 'End') width = SIDEBAR_MAX_WIDTH
    if (width === null) return
    event.preventDefault()
    event.stopPropagation()
    ui.sidebarWidth = Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, width))
  }

  function startDesktopSidebarResize(e: PointerEvent, options?: DesktopSidebarResizeOptions) {
    if (ui.isCompactLayout || !ui.isSidebarOpen) return

    const target = e.currentTarget as HTMLElement | null
    if (!target) return

    const startX = e.clientX
    const startW = ui.sidebarWidth
    const deferCommit = Boolean(options?.deferCommit)
    let previewWidth = startW

    target.setPointerCapture(e.pointerId)
    options?.onStart?.()
    options?.onPreviewWidth?.(startW)

    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'

    const move = (ev: PointerEvent) => {
      const next = Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, startW + (ev.clientX - startX)))
      previewWidth = next

      options?.onPreviewWidth?.(next)
      if (!deferCommit) {
        ui.sidebarWidth = next
      }
    }

    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)

      if (deferCommit) {
        ui.sidebarWidth = previewWidth
      }

      options?.onEnd?.(previewWidth)

      document.body.style.cursor = ''
      document.body.style.userSelect = ''

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
  }

  return { startDesktopSidebarResize, resizeDesktopSidebarFromKeyboard }
}
