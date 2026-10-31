import { watch } from 'vue'

export type ViewportRect = { width: number; height: number; left: number; top: number }

function positiveFinite(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : fallback
}

// Read on demand: closed controls must not force viewport layout during mount.
export function readViewportRect(): ViewportRect {
  if (typeof window === 'undefined') return { width: 0, height: 0, left: 0, top: 0 }
  const viewport = window.visualViewport
  return {
    width: positiveFinite(viewport?.width, window.innerWidth),
    height: positiveFinite(viewport?.height, window.innerHeight),
    left: positiveFinite(viewport?.offsetLeft, 0),
    top: positiveFinite(viewport?.offsetTop, 0),
  }
}

/** Observe while active; callers retain initial layout, positioning and focus policy. */
export function useViewportObserver(enabled: () => boolean, onChange: () => void): void {
  watch(
    enabled,
    (active, _previous, onCleanup) => {
      if (!active || typeof window === 'undefined') return
      const owner = window
      const viewport = owner.visualViewport
      owner.addEventListener('resize', onChange)
      owner.addEventListener('orientationchange', onChange)
      viewport?.addEventListener('resize', onChange)
      viewport?.addEventListener('scroll', onChange)
      // Capture the registered objects so replacement of visualViewport cannot
      // leak the original listeners. Vue also runs this cleanup on unmount.
      onCleanup(() => {
        owner.removeEventListener('resize', onChange)
        owner.removeEventListener('orientationchange', onChange)
        viewport?.removeEventListener('resize', onChange)
        viewport?.removeEventListener('scroll', onChange)
      })
    },
    { immediate: true },
  )
}
