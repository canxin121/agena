import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { getRuntimeSetting, setRuntimeSetting } from '@/lib/runtimeSettings'
import type { JsonObject } from '@/types/json'

type BoolMap = Record<string, boolean>

type TranscriptPreferences = {
  activity: {
    default_expanded: boolean
    kinds: BoolMap
  }
  tools: {
    categories: BoolMap
    overrides: BoolMap
  }
}

const DEFAULT_PREFERENCES: TranscriptPreferences = {
  activity: {
    default_expanded: false,
    kinds: { text: true },
  },
  tools: {
    categories: {},
    overrides: {},
  },
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function boolMap(value: unknown, normalize: (key: string) => string = (key) => key.trim()): BoolMap {
  const result: BoolMap = {}
  for (const [rawKey, rawValue] of Object.entries(record(value))) {
    const key = normalize(rawKey)
    if (key && typeof rawValue === 'boolean') result[key] = rawValue
  }
  return result
}

function normalizeToolId(value: string): string {
  const key = value.trim().toLowerCase()
  return key.startsWith('agena.') ? key.slice('agena.'.length) : key
}

function parsePreferences(value: unknown): TranscriptPreferences {
  const root = record(value)
  const activity = record(root.activity)
  const tools = record(root.tools)
  return {
    activity: {
      default_expanded:
        typeof activity.default_expanded === 'boolean'
          ? activity.default_expanded
          : DEFAULT_PREFERENCES.activity.default_expanded,
      kinds: {
        ...DEFAULT_PREFERENCES.activity.kinds,
        ...boolMap(activity.kinds),
      },
    },
    tools: {
      categories: boolMap(tools.categories, (key) => key.trim().toLowerCase()),
      overrides: boolMap(tools.overrides, normalizeToolId),
    },
  }
}

function serializePreferences(value: TranscriptPreferences): JsonObject {
  return {
    activity: {
      default_expanded: value.activity.default_expanded,
      kinds: { ...value.activity.kinds },
    },
    tools: {
      categories: { ...value.tools.categories },
      overrides: { ...value.tools.overrides },
    },
  }
}

export const useTranscriptPreferencesStore = defineStore('transcriptPreferences', () => {
  const preferences = ref<TranscriptPreferences>(parsePreferences(DEFAULT_PREFERENCES))
  const loading = ref(false)
  const loaded = ref(false)
  const error = ref('')
  const revision = ref(0)
  let writes = Promise.resolve()

  const activityDefaultExpanded = computed(() => preferences.value.activity.default_expanded)
  const activityKindOverrides = computed(() => preferences.value.activity.kinds)
  const toolCategoryOverrides = computed(() => preferences.value.tools.categories)
  const toolOverrides = computed(() => preferences.value.tools.overrides)

  async function persist(next: TranscriptPreferences) {
    preferences.value = next
    revision.value += 1
    const currentRevision = revision.value
    writes = writes.catch(() => {}).then(async () => {
      if (currentRevision !== revision.value) return
      await setRuntimeSetting('ui.transcript', serializePreferences(next), { reload: false })
    })
    await writes
  }

  async function refresh() {
    if (loading.value) return
    loading.value = true
    revision.value += 1
    error.value = ''
    try {
      const canonical = await getRuntimeSetting('ui.transcript')
      const canonicalValue = canonical.value
      if (canonicalValue && typeof canonicalValue === 'object' && !Array.isArray(canonicalValue)) {
        preferences.value = parsePreferences(canonicalValue)
      } else {
        preferences.value = parsePreferences(DEFAULT_PREFERENCES)
      }
      loaded.value = true
    } catch (reason) {
      error.value = reason instanceof Error ? reason.message : String(reason)
      preferences.value = parsePreferences(DEFAULT_PREFERENCES)
    } finally {
      loading.value = false
    }
  }

  function update(mutator: (next: TranscriptPreferences) => void) {
    const next = parsePreferences(preferences.value)
    mutator(next)
    void persist(next).catch((reason: unknown) => {
      error.value = reason instanceof Error ? reason.message : String(reason)
    })
  }

  function setActivityDefaultExpanded(value: boolean) {
    update((next) => {
      next.activity.default_expanded = value
    })
  }

  function setActivityKindExpanded(id: string, value: boolean) {
    const key = id.trim()
    if (!key) return
    update((next) => {
      next.activity.kinds[key] = value
    })
  }

  function setToolCategoryExpanded(id: string, value: boolean) {
    const key = id.trim().toLowerCase()
    if (!key) return
    update((next) => {
      next.tools.categories[key] = value
    })
  }

  function setToolExpanded(name: string, value: boolean) {
    const key = normalizeToolId(name)
    if (!key) return
    update((next) => {
      next.tools.overrides[key] = value
    })
  }

  function clearToolOverride(name: string) {
    const key = normalizeToolId(name)
    if (!key) return
    update((next) => {
      delete next.tools.overrides[key]
    })
  }

  function kindExpanded(id: string): boolean {
    return preferences.value.activity.kinds[id] ?? preferences.value.activity.default_expanded
  }

  function toolCategoryExpanded(id: string): boolean {
    return preferences.value.tools.categories[id] === true
  }

  function toolExpanded(name: string): boolean | undefined {
    return preferences.value.tools.overrides[normalizeToolId(name)]
  }

  return {
    preferences,
    activityDefaultExpanded,
    activityKindOverrides,
    toolCategoryOverrides,
    toolOverrides,
    loading,
    loaded,
    error,
    refresh,
    setActivityDefaultExpanded,
    setActivityKindExpanded,
    setToolCategoryExpanded,
    setToolExpanded,
    clearToolOverride,
    kindExpanded,
    toolCategoryExpanded,
    toolExpanded,
  }
})
