import { onBeforeUnmount, onMounted } from 'vue'

import { applyDeviceClasses, getDeviceInfo } from '@/lib/device'
import { useUiStore } from '@/stores/ui'

const DEVICE_MEDIA_QUERIES = ['(max-width: 900px)', '(max-width: 1024px)', '(pointer: coarse)', '(hover: hover)']

export function useDeviceRuntime() {
  const ui = useUiStore()
  const mediaQueries: MediaQueryList[] = []

  function applyDevice() {
    const info = getDeviceInfo()
    applyDeviceClasses(info)
    ui.setIsCompactLayout(info.isCompactLayout)
    ui.setIsMobileDevice(info.isMobileDevice)
    ui.setIsTouchPointer(info.isTouchPointer)
    ui.setIsMobilePointer(info.isMobilePointer)
  }

  // Prime state synchronously during App setup so loading/login UI renders
  // with the same device semantics as the authenticated workspace.
  applyDevice()

  onMounted(() => {
    window.addEventListener('resize', applyDevice)
    window.addEventListener('orientationchange', applyDevice)

    if (typeof window.matchMedia === 'function') {
      for (const query of DEVICE_MEDIA_QUERIES) {
        const media = window.matchMedia(query)
        media.addEventListener?.('change', applyDevice)
        mediaQueries.push(media)
      }
    }
  })

  onBeforeUnmount(() => {
    window.removeEventListener('resize', applyDevice)
    window.removeEventListener('orientationchange', applyDevice)
    for (const media of mediaQueries) {
      media.removeEventListener?.('change', applyDevice)
    }
    mediaQueries.length = 0
  })
}
