import { defineStore } from 'pinia'
import { ref } from 'vue'

import { postAppBroadcast } from '@/lib/appBroadcast'
import { getLocalJson, setLocalJson } from '@/lib/persist'

/**
 * Client-side UI preferences. Agena's server-side configuration (providers,
 * permissions, model catalog, …) is managed through the settings page panels
 * that talk to /api/v1/* directly; appearance + chat-activity UX preferences
 * are per-browser and persist in localStorage.
 */
export type Settings = {
  // Appearance
  useSystemTheme?: boolean
  themeVariant?: 'light' | 'dark'
  themeId?: string
  lightThemeId?: string
  darkThemeId?: string
  uiFont?: string
  monoFont?: string
  fontSize?: number
  padding?: number
  cornerRadius?: number
  inputBarOffset?: number
  typographySizes?: {
    markdown?: string
    code?: string
    uiHeader?: string
    uiLabel?: string
    meta?: string
    micro?: string
  }

  // Chat message UX
  showChatTimestamps?: boolean
  showReasoningTraces?: boolean
  showTextJustificationActivity?: boolean
  chatActivityAutoCollapseOnIdle?: boolean
  chatActivitySummaryFilters?: string[]
  chatToolActivitySummaryFilters?: string[]
  chatActivityKindDefaultExpanded?: string[]
  // Legacy OpenCode activity keys; read only for preference migration.
  chatActivityDefaultExpanded?: string[]
  chatActivityDefaultExpandedToolFilters?: string[]
  chatToolActivityDefaultExpandedOverrides?: Record<string, boolean>
  diffLayoutPreference?: 'dynamic' | 'inline' | 'side-by-side'
  diffViewMode?: 'single' | 'stacked'
}

const STORAGE_KEY = 'agena.settings.ui-prefs.v1'

function cloneRecord(raw: unknown): Settings | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  return { ...(raw as Record<string, unknown>) } as Settings
}

export const useSettingsStore = defineStore('settings', () => {
  const data = ref<Settings | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  function hydrate() {
    const raw = getLocalJson<unknown>(STORAGE_KEY, null)
    data.value = cloneRecord(raw)
  }

  async function refresh() {
    loading.value = true
    error.value = null
    try {
      hydrate()
      if (!data.value) {
        data.value = {}
      }
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      data.value = null
    } finally {
      loading.value = false
    }
  }

  async function save(partial: Partial<Settings>) {
    error.value = null
    try {
      const next: Settings = {
        ...(data.value || {}),
        ...partial,
      }
      data.value = next
      setLocalJson(STORAGE_KEY, next)
      postAppBroadcast('settings.updated', { updatedAt: Date.now() })
    } catch (err) {
      if (err instanceof Error) {
        error.value = err.message
      } else {
        error.value = String(err)
      }
    }
  }

  return {
    data,
    loading,
    error,
    hydrate,
    refresh,
    save,
  }
})
