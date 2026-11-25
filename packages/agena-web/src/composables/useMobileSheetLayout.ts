import { ref, watch, type CSSProperties } from 'vue'
import { readViewportRect, useViewportObserver } from '@/composables/useViewportObserver'

export type MobileSheetPlacement = 'center' | 'top' | 'fill'

const margin = 8

export function useMobileSheetLayout(isOpen: () => boolean, placement: () => MobileSheetPlacement = () => 'center') {
  const maxHeight = `calc(100dvh - ${margin * 2}px)`
  const style = ref<CSSProperties>({
    top: placement() === 'center' ? '50%' : `${margin}px`,
    left: '50%',
    width: `calc(100% - ${margin * 2}px)`,
    maxHeight,
    height: placement() === 'fill' ? maxHeight : undefined,
  })

  function update() {
    if (!isOpen() || typeof document === 'undefined') return
    const viewport = readViewportRect()
    if (!viewport.height) return
    const safeArea = getComputedStyle(document.documentElement)
    const inset = (side: string) => Number.parseFloat(safeArea.getPropertyValue(`--oc-safe-area-${side}`)) || 0
    const usableWidth = Math.max(0, viewport.width - inset('left') - inset('right'))
    const height = Math.max(0, viewport.height - inset('top') - inset('bottom') - margin * 2)
    const top = viewport.top + inset('top') + margin
    const mode = placement()
    style.value = {
      top: `${Math.round(top + (mode === 'center' ? height / 2 : 0))}px`,
      left: `${Math.round(viewport.left + inset('left') + usableWidth / 2)}px`,
      width: `${Math.max(0, Math.round(usableWidth - margin * 2))}px`,
      maxHeight: `${Math.floor(height)}px`,
      height: mode === 'fill' ? `${Math.floor(height)}px` : undefined,
    }
  }

  watch([isOpen, placement], update, { immediate: true })
  useViewportObserver(isOpen, update)
  return style
}
