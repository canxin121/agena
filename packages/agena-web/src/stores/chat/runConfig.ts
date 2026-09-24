import type { SessionRunConfig } from '@/types/chat'

export function loadSessionRunConfigMap(storageKey: string): Record<string, SessionRunConfig> {
  try {
    const raw = localStorage.getItem(storageKey)
    const parsed = raw ? JSON.parse(raw) : null
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return parsed as Record<string, SessionRunConfig>
    }
  } catch {
    // ignore
  }
  return {}
}

export function createSessionRunConfigPersister(
  storageKey: string,
  getValue: () => Record<string, SessionRunConfig>,
): { persistSoon: () => void } {
  let timer: number | null = null
  function persistSoon() {
    if (timer) return
    timer = window.setTimeout(() => {
      timer = null
      try {
        localStorage.setItem(storageKey, JSON.stringify(getValue()))
      } catch {
        // ignore
      }
    }, 250)
  }
  return { persistSoon }
}
