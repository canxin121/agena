import type { ModelCatalogEntry, ProviderAdapterDiscovery, ProviderModel } from '../lib/agenaApi'
import { buildConfiguredProviderModelFromDraft, createModelCatalogDraftFromEntry } from './useRuntimeModelCatalogActions'

export type ProviderAdapterPatch = {
  enabled: boolean
  models?: Record<string, Record<string, unknown>>
}

export function catalogEntriesForModelId(entries: ModelCatalogEntry[], modelId: string): ModelCatalogEntry[] {
  return entries
    .filter((entry) => entry.model_id === modelId)
    .sort((left, right) => {
      if (left.kind !== right.kind) return left.kind === 'custom' ? -1 : 1
      return left.model_id.localeCompare(right.model_id)
    })
}

export function preferredCatalogEntryForModelId(entries: ModelCatalogEntry[], modelId: string): ModelCatalogEntry | null {
  return catalogEntriesForModelId(entries, modelId)[0] || null
}

export function matchedCatalogModelDefinitions(
  entries: ModelCatalogEntry[],
  models: ProviderModel[],
): Record<string, Record<string, unknown>> {
  const definitions: Record<string, Record<string, unknown>> = {}
  for (const model of models) {
    const entry = preferredCatalogEntryForModelId(entries, model.id)
    if (!entry) continue
    definitions[model.id] = buildConfiguredProviderModelFromDraft(createModelCatalogDraftFromEntry(entry))
  }
  return definitions
}

export function discoveryMatchedModels(entries: ModelCatalogEntry[], discovery: ProviderAdapterDiscovery): ProviderModel[] {
  return discovery.models.filter((model) => Boolean(preferredCatalogEntryForModelId(entries, model.id)))
}

export function discoveryUnmatchedModels(
  entries: ModelCatalogEntry[],
  discovery: ProviderAdapterDiscovery,
): ProviderModel[] {
  return discovery.models.filter((model) => !preferredCatalogEntryForModelId(entries, model.id))
}

export function buildAdaptersPatchFromDraftSelection(input: {
  catalogEntries: ModelCatalogEntry[]
  discoveries: ProviderAdapterDiscovery[]
  selectedAdapterIds: string[]
  defaultAdapterId: string
  defaultModelId: string
}): Record<string, ProviderAdapterPatch> {
  const adaptersPatch: Record<string, ProviderAdapterPatch> = {}

  for (const discovery of input.discoveries) {
    if (!discovery.supported || !input.selectedAdapterIds.includes(discovery.adapter_id)) continue
    const matchedModels = matchedCatalogModelDefinitions(input.catalogEntries, discovery.models)
    adaptersPatch[discovery.adapter_id] = {
      enabled: true,
      ...(Object.keys(matchedModels).length ? { models: matchedModels } : {}),
    }
  }

  const defaultModelEntry = preferredCatalogEntryForModelId(input.catalogEntries, input.defaultModelId)
  adaptersPatch[input.defaultAdapterId] = {
    enabled: true,
    models: {
      ...(adaptersPatch[input.defaultAdapterId]?.models || {}),
      [input.defaultModelId]: defaultModelEntry
        ? buildConfiguredProviderModelFromDraft(createModelCatalogDraftFromEntry(defaultModelEntry))
        : {},
    },
  }

  return adaptersPatch
}
