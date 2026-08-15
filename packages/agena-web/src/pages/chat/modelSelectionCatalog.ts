import { computed, ref, type Ref } from 'vue'

import { apiJson } from '../../lib/api'
import type { JsonValue as JsonLike } from '../../types/json'

// ---------------------------------------------------------------------------
// Agena provider catalog (replaces the old opencode /api/config/* + /api/agent
// sources). The server exposes:
//
//   GET /api/v1/providers                    → ProviderSummaryResource[]
//   GET /api/v1/providers/{provider_id}/models → { provider_id, models: ProviderModelResource[] }
//
// ProviderModelResource: { provider_id, adapter_id?, id, catalog_model_id?,
//   display_name?, native_compaction, capabilities, metadata, thinking_modes[],
//   speed_modes{} }. The catalog keeps the same public surface as the old
// opencode catalog so the model-selection machinery above it is unchanged.
// ---------------------------------------------------------------------------

export type Provider = { id: string; name?: string; models?: JsonLike }
export type Agent = { name: string; description?: string; mode?: string; hidden?: boolean; disable?: boolean }
export type ModelMetaRecord = Record<string, JsonLike>

type AgenaProviderSummary = {
  provider_id?: string
  defaults?: { adapter?: string; model?: string }
  adapters?: Array<{ adapter_id?: string; enabled?: boolean; configured_model_count?: number }>
}

type AgenaProviderModelsResponse = {
  provider_id?: string
  models?: JsonLike[]
}

function isRecord(value: JsonLike | null | undefined): value is ModelMetaRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asRecord(value: JsonLike): ModelMetaRecord {
  return isRecord(value) ? value : {}
}

function readString(value: JsonLike | null | undefined): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function modelIdsFromProviderModels(models: JsonLike): string[] {
  if (!models) return []
  if (Array.isArray(models)) {
    return models
      .map((m) => {
        const rec = asRecord(m)
        return readString(rec.id as JsonLike) || readString(rec.model_id as JsonLike)
      })
      .filter(Boolean)
  }
  if (isRecord(models)) {
    return Object.keys(models)
  }
  return []
}

export function isSelectablePrimaryAgent(agent: Agent | null | undefined): boolean {
  if (!agent) return false
  const name = String(agent.name || '').trim()
  if (!name) return false
  if (agent.disable === true) return false
  if (agent.hidden === true) return false
  if (agent.mode === 'subagent') return false
  return true
}

export function useModelSelectionCatalog(opts: { sessionDirectory: Ref<string> }) {
  // The agena server owns the workspace; keep the sessionDirectory ref accepted
  // for signature compatibility (no ?directory= query is sent anymore).
  void opts.sessionDirectory

  const providerDefaultModels = ref<Record<string, string>>({})

  const providers = ref<Provider[]>([])
  const agents = ref<Agent[]>([])
  const catalogLoading = ref(false)

  // Agena has no per-session share toggle; sharing is available.
  const shareDisabled = computed(() => false)

  // Agena derives defaults server-side from the provider list; no client config
  // layers exist anymore.
  const projectConfigDefaults = computed(() => ({ defaultAgent: '', defaultProvider: '', defaultModel: '' }))
  const userConfigDefaults = computed(() => ({ defaultAgent: '', defaultProvider: '', defaultModel: '' }))

  const fallbackAgent = computed(() => (agents.value[0]?.name || '').trim())

  const fallbackProviderModel = computed(() => {
    const first = providers.value[0]
    const provider = typeof first?.id === 'string' ? first.id.trim() : ''
    if (!provider) return { provider: '', model: '' }

    const ids = modelIdsFromProviderModels(first?.models)
    const idSet = new Set(ids)
    const candidate = (providerDefaultModels.value[provider] || '').trim()
    const model = candidate && idSet.has(candidate) ? candidate : ids[0] || ''
    return { provider, model }
  })

  function ensureDefaultProviderInList(list: Provider[]) {
    const def = (providerDefaultModels.value['__default__'] || '').trim()
    if (!def) return list
    if (list.some((p) => p.id === def)) return list
    return [...list, { id: def, name: def, models: {} }]
  }

  function modelMetaFor(providerId: string, modelId: string): ModelMetaRecord | null {
    const pid = (providerId || '').trim()
    const mid = (modelId || '').trim()
    if (!pid || !mid) return null

    const fromRemote = providers.value.find((p) => p.id === pid)
    const remoteModels = fromRemote?.models
    if (Array.isArray(remoteModels)) {
      const match = remoteModels.find((m) => {
        const rec = asRecord(m)
        return readString(rec.id as JsonLike) === mid || readString(rec.model_id as JsonLike) === mid
      })
      return isRecord(match) ? match : null
    }
    if (isRecord(remoteModels) && !Array.isArray(remoteModels)) {
      const candidate = remoteModels[mid]
      return isRecord(candidate) ? candidate : null
    }
    return null
  }

  function providerFromSummary(summary: AgenaProviderSummary, models: JsonLike[]): Provider {
    const id = readString(summary.provider_id as JsonLike)
    return {
      id,
      name: id,
      models,
    }
  }

  function hasConfiguredModels(summary: AgenaProviderSummary): boolean {
    if (!Array.isArray(summary.adapters)) return false
    return summary.adapters.some((adapter) => {
      const enabled = adapter?.enabled !== false
      const count = Number(adapter?.configured_model_count ?? 0)
      return enabled && count > 0
    })
  }

  async function loadProvidersAndAgents() {
    if (catalogLoading.value) return
    catalogLoading.value = true
    try {
      // GET /api/v1/providers → summaries; agena has no separate agent catalog.
      const summaries: AgenaProviderSummary[] = await apiJson<AgenaProviderSummary[]>('/api/v1/providers')
      const list = Array.isArray(summaries) ? summaries : []

      const nextDefaultModels: Record<string, string> = {}
      for (const summary of list) {
        const pid = readString(summary.provider_id as JsonLike)
        const model = readString(summary.defaults?.model as JsonLike)
        if (pid && model) nextDefaultModels[pid] = model
      }
      providerDefaultModels.value = nextDefaultModels

      // Fetch models for each provider that has configured adapters, in parallel.
      const providersWithModels = list.filter(hasConfiguredModels)
      const settled = await Promise.allSettled(
        providersWithModels.map(async (summary) => {
          const pid = readString(summary.provider_id as JsonLike)
          if (!pid) return null
          const resp = await apiJson<AgenaProviderModelsResponse>(`/api/v1/providers/${encodeURIComponent(pid)}/models`)
          return providerFromSummary(summary, Array.isArray(resp?.models) ? resp.models : [])
        }),
      )
      const next: Provider[] = []
      for (const result of settled) {
        if (result.status === 'fulfilled' && result.value) next.push(result.value)
      }
      // Providers without fetched models still appear so defaults resolve.
      for (const summary of list) {
        const pid = readString(summary.provider_id as JsonLike)
        if (!pid) continue
        if (next.some((p) => p.id === pid)) continue
        next.push(providerFromSummary(summary, []))
      }
      providers.value = ensureDefaultProviderInList(next)

      // No /api/agent endpoint in agena → empty agent list; the picker shows
      // only the "auto" default entry.
      agents.value = []
    } catch {
      providerDefaultModels.value = {}
      providers.value = []
      agents.value = []
    } finally {
      catalogLoading.value = false
    }
  }

  return {
    providers,
    agents,
    catalogLoading,
    shareDisabled,
    projectConfigDefaults,
    userConfigDefaults,
    fallbackAgent,
    fallbackProviderModel,
    modelMetaFor,
    loadProvidersAndAgents,
  }
}
