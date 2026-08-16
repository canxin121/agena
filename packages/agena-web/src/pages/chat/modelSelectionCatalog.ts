import { computed, ref, type Ref } from 'vue'

import { apiJson } from '../../lib/api'
import { extractConfigDefaults } from './modelSelectionDefaults'
import type { OpencodeConfigResponse } from '../../stores/opencodeConfig'
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

export type OpencodeConfigStoreLike = {
  data: OpencodeConfigResponse['config'] | null
  scope: string
  exists: boolean | null
  refresh: (opts: { scope: 'project' | 'user'; directory: string | null }) => Promise<void>
}

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

export function useModelSelectionCatalog(opts: {
  opencodeConfig: OpencodeConfigStoreLike
  sessionDirectory: Ref<string>
}) {
  const { opencodeConfig, sessionDirectory } = opts

  const projectConfigLayer = ref<ModelMetaRecord | null>(null)
  const userConfigLayer = ref<ModelMetaRecord | null>(null)
  const providerDefaultModels = ref<Record<string, string>>({})

  const providers = ref<Provider[]>([])
  const agents = ref<Agent[]>([])
  const catalogLoading = ref(false)

  const resolvedOpencodeConfig = computed(() => {
    return projectConfigLayer.value || userConfigLayer.value || {}
  })

  const shareDisabled = computed(() => {
    const cfg = resolvedOpencodeConfig.value
    return String(cfg?.share || '').trim() === 'disabled'
  })

  const projectConfigDefaults = computed(() => extractConfigDefaults(projectConfigLayer.value))
  const userConfigDefaults = computed(() => extractConfigDefaults(userConfigLayer.value))

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
    const def = (projectConfigDefaults.value.defaultProvider || userConfigDefaults.value.defaultProvider || '').trim()
    if (!def) return list
    if (list.some((p) => p.id === def)) return list
    return [...list, { id: def, name: def, models: {} }]
  }

  function mergeProviderLists(primary: Provider[], fallback: Provider[]) {
    const fallbackMap = new Map<string, Provider>()
    for (const p of fallback) fallbackMap.set(p.id, p)

    const out: Provider[] = []
    const seen = new Set<string>()
    for (const p of primary) {
      const existing = fallbackMap.get(p.id)
      out.push(existing ? { ...existing, ...p, models: p.models || existing.models } : p)
      seen.add(p.id)
    }
    for (const p of fallback) {
      if (!seen.has(p.id)) out.push(p)
    }
    return out
  }

  function providerListFromConfig(): Provider[] {
    const cfg = resolvedOpencodeConfig.value
    const out: Provider[] = []
    const providerMap = cfg?.provider
    if (!isRecord(providerMap)) return out

    for (const [id, value] of Object.entries(providerMap)) {
      const label = String(id).trim()
      if (!label) continue
      const valueRecord = asRecord(value)
      const name = typeof valueRecord.name === 'string' ? valueRecord.name : undefined
      const modelsRaw = valueRecord.models
      const models = Array.isArray(modelsRaw) || isRecord(modelsRaw) ? modelsRaw : undefined
      out.push({ id: label, name, models })
    }
    return out
  }

  function agentListFromConfig(): Agent[] {
    const cfg = resolvedOpencodeConfig.value
    const entries: Agent[] = []
    const agentMap = cfg?.agent
    const modeMap = cfg?.mode

    const readMap = (map: ModelMetaRecord | null | undefined) => {
      if (!isRecord(map)) return
      for (const [name, value] of Object.entries(map)) {
        const label = String(name).trim()
        if (!label) continue

        const rec = asRecord(value)
        const description = typeof rec.description === 'string' ? rec.description : undefined
        const mode = typeof rec.mode === 'string' ? rec.mode : undefined
        const hidden = typeof rec.hidden === 'boolean' ? rec.hidden : undefined
        const disable = typeof rec.disable === 'boolean' ? rec.disable : undefined

        if (disable === true) continue
        if (hidden === true) continue
        if (mode === 'subagent') continue
        if (!mode && (label === 'general' || label === 'explore')) continue

        entries.push({ name: label, description, mode, hidden, disable })
      }
    }

    readMap(isRecord(agentMap) ? agentMap : undefined)
    if (isRecord(modeMap)) {
      const decorated: ModelMetaRecord = {}
      for (const [k, v] of Object.entries(modeMap)) {
        decorated[k] = { ...asRecord(v), mode: 'primary' }
      }
      readMap(decorated)
    }

    const defaultAgent = (
      projectConfigDefaults.value.defaultAgent ||
      userConfigDefaults.value.defaultAgent ||
      ''
    ).trim()
    if (defaultAgent && !entries.some((a) => a.name === defaultAgent)) {
      entries.push({ name: defaultAgent })
    }

    return entries.filter(isSelectablePrimaryAgent)
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

    const fromCfg = providerListFromConfig().find((p) => p.id === pid)
    const cfgModels = fromCfg?.models
    if (Array.isArray(cfgModels)) {
      const match = cfgModels.find((m) => {
        const rec = asRecord(m)
        return readString(rec.id as JsonLike) === mid || readString(rec.model_id as JsonLike) === mid
      })
      return isRecord(match) ? match : null
    }
    if (isRecord(cfgModels) && !Array.isArray(cfgModels)) {
      const candidate = cfgModels[mid]
      return isRecord(candidate) ? candidate : null
    }

    return null
  }

  async function refreshOpencodeConfig() {
    const dir = sessionDirectory.value
    const scope = dir ? 'project' : 'user'
    projectConfigLayer.value = null
    userConfigLayer.value = null

    try {
      await opencodeConfig.refresh({ scope, directory: dir || null })
    } catch {
      // ignore
    }

    if (scope === 'project' && opencodeConfig.exists !== false) {
      projectConfigLayer.value = isRecord(opencodeConfig.data) ? opencodeConfig.data : {}
    }
    if (scope === 'user') {
      userConfigLayer.value = isRecord(opencodeConfig.data) ? opencodeConfig.data : {}
    }
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
      // Keep the config layers for defaults even though agena has no config
      // endpoint; the opencodeConfig store is a local stub.
      await refreshOpencodeConfig()

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
      providers.value = ensureDefaultProviderInList(mergeProviderLists(next, providerListFromConfig()))

      // No /api/agent endpoint in agena → derive agents from the (stubbed) config
      // layers; the picker shows the "auto" default entry when that is empty.
      agents.value = agentListFromConfig()
    } catch {
      providerDefaultModels.value = {}
      providers.value = ensureDefaultProviderInList(providerListFromConfig())
      agents.value = agentListFromConfig()
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
