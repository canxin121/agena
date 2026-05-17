import type { ModelCatalogEntry, ProviderAdapterDiscovery, ProviderModel } from '../lib/agenaApi'
import {
  buildConfiguredProviderModelFromDraft,
  catalogLookupIdForProviderModel,
  createModelCatalogDraftFromEntry,
} from './useRuntimeModelCatalogActions'

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

export function preferredCatalogEntryForLookupIds(
  entries: ModelCatalogEntry[],
  modelIds: string[],
): ModelCatalogEntry | null {
  const lookupIds = [...new Set(modelIds.map((value) => String(value || '').trim()).filter(Boolean))]
  if (!lookupIds.length) return null
  const matches = entries.filter((entry) => lookupIds.includes(entry.model_id))
  return matches.sort((left, right) => {
    if (left.kind !== right.kind) return left.kind === 'custom' ? -1 : 1
    return left.model_id.localeCompare(right.model_id)
  })[0] || null
}

export function preferredCatalogEntryForProviderModel(
  entries: ModelCatalogEntry[],
  model: ProviderModel,
): ModelCatalogEntry | null {
  return preferredCatalogEntryForLookupIds(entries, [model.id, catalogLookupIdForProviderModel(model)])
}

export function matchedCatalogModelDefinitions(
  entries: ModelCatalogEntry[],
  models: ProviderModel[],
): Record<string, Record<string, unknown>> {
  const definitions: Record<string, Record<string, unknown>> = {}
  for (const model of models) {
    const entry = preferredCatalogEntryForProviderModel(entries, model)
    if (!entry) continue
    definitions[model.id] = buildConfiguredProviderModelFromDraft(createModelCatalogDraftFromEntry(entry))
  }
  return definitions
}

export function discoveryMatchedModels(entries: ModelCatalogEntry[], discovery: ProviderAdapterDiscovery): ProviderModel[] {
  return discovery.models.filter((model) => Boolean(preferredCatalogEntryForProviderModel(entries, model)))
}

export function discoveryUnmatchedModels(
  entries: ModelCatalogEntry[],
  discovery: ProviderAdapterDiscovery,
): ProviderModel[] {
  return discovery.models.filter((model) => !preferredCatalogEntryForProviderModel(entries, model))
}

export function buildAdaptersPatchFromDraftSelection(input: {
  catalogEntries: ModelCatalogEntry[]
  discoveries: ProviderAdapterDiscovery[]
  selectedAdapterIds: string[]
  defaultAdapterId: string
  defaultModelId: string
  defaultCatalogModelId?: string
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

  const defaultModelEntry = preferredCatalogEntryForLookupIds(input.catalogEntries, [
    input.defaultModelId,
    input.defaultCatalogModelId || '',
  ])
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
