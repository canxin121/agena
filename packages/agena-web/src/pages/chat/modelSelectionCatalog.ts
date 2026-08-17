import { computed, ref } from 'vue'

import { apiJson } from '../../lib/api'
import type { JsonValue } from '../../types/json'

export type ThinkingRequest = {
  type?: string
  effort?: string | null
  budget_tokens?: number
  display?: string
}

export type ProviderModelThinkingMode = {
  default?: boolean
  display_name?: string | null
  description?: string | null
  preset?: string | null
  thinking?: ThinkingRequest | null
}

export type ProviderModelSpeedMode = {
  default?: boolean
  display_name?: string | null
  description?: string | null
}

export type ProviderModel = {
  provider_id: string
  adapter_id?: string | null
  id: string
  catalog_model_id?: string | null
  display_name?: string | null
  native_compaction?: boolean
  capabilities?: Record<string, JsonValue>
  metadata?: Record<string, JsonValue>
  thinking_modes?: ProviderModelThinkingMode[]
  speed_modes?: Record<string, ProviderModelSpeedMode>
  [key: string]: JsonValue
}

export type Provider = {
  id: string
  name: string
  defaultAdapter: string
  defaultModel: string
  models: ProviderModel[]
}

export type RuntimeDefaultSelection = {
  provider: string
  adapter: string
  model: string
  thinkingMode: string
  speedMode: string
  verbosity: string
  parallelToolCalls?: boolean
}

export type ModelMetaRecord = ProviderModel

export type ModelModeOption = {
  value: string
  label: string
  description: string
  isDefault: boolean
}

type ProviderSummary = {
  provider_id?: string
  defaults?: { adapter?: string | null; model?: string | null }
  adapters?: Array<{ adapter_id?: string; enabled?: boolean; configured_model_count?: number }>
}

type ProviderAdapterModels = {
  adapter_id?: string
  enabled?: boolean
  models?: ProviderModel[]
  failure?: JsonValue
}

type RuntimeStatus = {
  default_selection?: {
    provider?: string | null
    adapter?: string | null
    model?: string | null
    thinking_mode?: string | null
    speed_mode?: string | null
    verbosity?: string | null
    parallel_tool_calls?: boolean | null
  } | null
}

function readString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function normalizeModel(model: ProviderModel, providerId: string, adapterId: string): ProviderModel | null {
  const id = readString(model?.id)
  if (!id) return null
  return {
    ...model,
    provider_id: readString(model.provider_id) || providerId,
    adapter_id: readString(model.adapter_id) || adapterId || undefined,
    id,
    thinking_modes: Array.isArray(model.thinking_modes) ? model.thinking_modes : [],
    speed_modes:
      model.speed_modes && typeof model.speed_modes === 'object' && !Array.isArray(model.speed_modes)
        ? model.speed_modes
        : {},
  }
}

export function modelIdsFromProviderModels(models: ProviderModel[] | null | undefined): string[] {
  return Array.isArray(models) ? models.map((model) => readString(model.id)).filter(Boolean) : []
}

export function thinkingModeSelector(mode: ProviderModelThinkingMode | null | undefined): string {
  if (!mode) return ''
  const preset = readString(mode.preset)
  if (preset) return preset
  const thinking = mode.thinking
  const type = readString(thinking?.type).toLowerCase()
  if (type === 'disabled') return 'off'
  if (type === 'effort' || type === 'adaptive') return readString(thinking?.effort)
  return ''
}

export function thinkingModeOptionsForModel(model: ProviderModel | null | undefined): ModelModeOption[] {
  const out: ModelModeOption[] = []
  const seen = new Set<string>()
  for (const mode of Array.isArray(model?.thinking_modes) ? model.thinking_modes : []) {
    const value = thinkingModeSelector(mode)
    if (!value || seen.has(value)) continue
    seen.add(value)
    out.push({
      value,
      label: readString(mode.display_name) || value,
      description: readString(mode.description),
      isDefault: mode.default === true,
    })
  }
  return out
}

export function speedModeOptionsForModel(model: ProviderModel | null | undefined): ModelModeOption[] {
  const modes = model?.speed_modes
  if (!modes || typeof modes !== 'object' || Array.isArray(modes)) return []
  return Object.entries(modes)
    .map(([key, mode]) => {
      const value = readString(key)
      if (!value) return null
      return {
        value,
        label: readString(mode?.display_name) || value,
        description: readString(mode?.description),
        isDefault: mode?.default === true,
      }
    })
    .filter((item): item is ModelModeOption => Boolean(item))
}

export function defaultModeValue(options: ModelModeOption[]): string {
  return options.find((option) => option.isDefault)?.value || ''
}

/**
 * Resolve the catalog-facing label for a selected mode without turning an
 * empty selection into an arbitrary mode. An empty result means that the
 * provider/model native default should be used, or that the catalog has no
 * display name for the supplied value.
 */
export function modeOptionDisplayLabel(options: ModelModeOption[], value: string): string {
  const normalized = readString(value)
  if (!normalized) return ''
  return options.find((option) => option.value === normalized)?.label || ''
}

export function useModelSelectionCatalog() {
  const providers = ref<Provider[]>([])
  const runtimeDefaultSelection = ref<RuntimeDefaultSelection>({
    provider: '',
    adapter: '',
    model: '',
    thinkingMode: '',
    speedMode: '',
    verbosity: '',
  })
  const catalogLoading = ref(false)
  const catalogError = ref('')
  let loadPromise: Promise<void> | null = null

  const fallbackProviderModel = computed(() => {
    const configured = runtimeDefaultSelection.value
    if (configured.provider && configured.model) {
      return { provider: configured.provider, adapter: configured.adapter, model: configured.model }
    }

    for (const provider of providers.value) {
      const defaultModel = provider.models.find((model) => {
        if (model.id !== provider.defaultModel) return false
        return !provider.defaultAdapter || readString(model.adapter_id) === provider.defaultAdapter
      })
      if (defaultModel) {
        return {
          provider: provider.id,
          adapter: readString(defaultModel.adapter_id),
          model: defaultModel.id,
        }
      }
    }

    const firstProvider = providers.value.find((provider) => provider.models.length > 0)
    const firstModel = firstProvider?.models[0]
    return {
      provider: firstProvider?.id || '',
      adapter: readString(firstModel?.adapter_id),
      model: firstModel?.id || '',
    }
  })

  function modelMetaFor(providerId: string, modelId: string, adapterId?: string): ProviderModel | null {
    const provider = providers.value.find((item) => item.id === readString(providerId))
    if (!provider) return null
    const targetModel = readString(modelId)
    const targetAdapter = readString(adapterId)
    if (targetAdapter) {
      return (
        provider.models.find((model) => model.id === targetModel && readString(model.adapter_id) === targetAdapter) ||
        null
      )
    }
    return provider.models.find((model) => model.id === targetModel) || null
  }

  async function loadProvidersAndModels() {
    if (loadPromise) return await loadPromise
    loadPromise = (async () => {
      catalogLoading.value = true
      catalogError.value = ''
      try {
        const [runtime, summaries] = await Promise.all([
          apiJson<RuntimeStatus>('/api/v1/runtime'),
          apiJson<ProviderSummary[]>('/api/v1/providers'),
        ])

        const selection = runtime?.default_selection || null
        runtimeDefaultSelection.value = {
          provider: readString(selection?.provider),
          adapter: readString(selection?.adapter),
          model: readString(selection?.model),
          thinkingMode: readString(selection?.thinking_mode),
          speedMode: readString(selection?.speed_mode),
          verbosity: readString(selection?.verbosity),
          ...(typeof selection?.parallel_tool_calls === 'boolean'
            ? { parallelToolCalls: selection.parallel_tool_calls }
            : {}),
        }

        const summaryList = Array.isArray(summaries) ? summaries : []
        const results = await Promise.allSettled(
          summaryList.map(async (summary) => {
            const providerId = readString(summary.provider_id)
            if (!providerId) return null
            const adapters = await apiJson<ProviderAdapterModels[]>(
              `/api/v1/providers/${encodeURIComponent(providerId)}/configured-models`,
            )
            const models: ProviderModel[] = []
            for (const adapter of Array.isArray(adapters) ? adapters : []) {
              if (adapter?.enabled === false) continue
              const adapterId = readString(adapter?.adapter_id)
              for (const rawModel of Array.isArray(adapter?.models) ? adapter.models : []) {
                const model = normalizeModel(rawModel, providerId, adapterId)
                if (model) models.push(model)
              }
            }
            return {
              id: providerId,
              name: providerId,
              defaultAdapter: readString(summary.defaults?.adapter),
              defaultModel: readString(summary.defaults?.model),
              models,
            } satisfies Provider
          }),
        )

        const next: Provider[] = []
        for (let index = 0; index < results.length; index += 1) {
          const result = results[index]
          if (result?.status === 'fulfilled' && result.value) {
            next.push(result.value)
            continue
          }
          const summary = summaryList[index]
          const providerId = readString(summary?.provider_id)
          if (providerId) {
            next.push({
              id: providerId,
              name: providerId,
              defaultAdapter: readString(summary?.defaults?.adapter),
              defaultModel: readString(summary?.defaults?.model),
              models: [],
            })
          }
        }

        const configuredDefault = runtimeDefaultSelection.value
        if (configuredDefault.provider && configuredDefault.model) {
          let provider = next.find((item) => item.id === configuredDefault.provider)
          if (!provider) {
            provider = {
              id: configuredDefault.provider,
              name: configuredDefault.provider,
              defaultAdapter: configuredDefault.adapter,
              defaultModel: configuredDefault.model,
              models: [],
            }
            next.push(provider)
          }
          const exists = provider.models.some(
            (model) =>
              model.id === configuredDefault.model &&
              (!configuredDefault.adapter || readString(model.adapter_id) === configuredDefault.adapter),
          )
          if (!exists) {
            provider.models.push({
              provider_id: configuredDefault.provider,
              adapter_id: configuredDefault.adapter || undefined,
              id: configuredDefault.model,
              thinking_modes: [],
              speed_modes: {},
            })
          }
        }

        providers.value = next.sort((a, b) => a.id.localeCompare(b.id))
      } catch (error) {
        catalogError.value = error instanceof Error ? error.message : String(error)
        providers.value = []
      } finally {
        catalogLoading.value = false
      }
    })()

    try {
      await loadPromise
    } finally {
      loadPromise = null
    }
  }

  return {
    providers,
    runtimeDefaultSelection,
    fallbackProviderModel,
    catalogLoading,
    catalogError,
    modelMetaFor,
    loadProvidersAndModels,
  }
}
