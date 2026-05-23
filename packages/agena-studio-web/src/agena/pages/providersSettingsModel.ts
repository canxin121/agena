import type { ModelCatalogEntry, ProviderAdapterModels, ProviderModel } from '../lib/agenaApi'
import {
  buildConfiguredProviderModelFromDraft,
  catalogLookupIdForProviderModel,
  createModelCatalogDraftFromEntry,
  createModelCatalogDraftFromProviderSelection,
} from './useRuntimeModelCatalogActions'

export type ProviderAdapterPatch = {
  enabled: boolean
  models?: Record<string, Record<string, unknown>>
}

export function catalogEntriesForModelId(entries: ModelCatalogEntry[], modelId: string): ModelCatalogEntry[] {
  return entries
    .filter((entry) => entry.model_id === modelId)
    .sort((left, right) => left.model_id.localeCompare(right.model_id))
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
  return matches.sort((left, right) => left.model_id.localeCompare(right.model_id))[0] || null
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

export function configuredProviderModelDefinitions(
  entries: ModelCatalogEntry[],
  models: ProviderModel[],
): Record<string, Record<string, unknown>> {
  const definitions: Record<string, Record<string, unknown>> = {}
  for (const model of models) {
    definitions[model.id] = buildConfiguredProviderModelFromDraft(
      createModelCatalogDraftFromProviderSelection(entries, model),
    )
  }
  return definitions
}

export function adapterModelsMatchedModels(entries: ModelCatalogEntry[], adapterModels: ProviderAdapterModels): ProviderModel[] {
  return adapterModels.models.filter((model) => Boolean(preferredCatalogEntryForProviderModel(entries, model)))
}

export function adapterModelsUnmatchedModels(
  entries: ModelCatalogEntry[],
  adapterModels: ProviderAdapterModels,
): ProviderModel[] {
  return adapterModels.models.filter((model) => !preferredCatalogEntryForProviderModel(entries, model))
}

export function buildAdaptersPatchFromDraftSelection(input: {
  catalogEntries: ModelCatalogEntry[]
  adapterModelLists: ProviderAdapterModels[]
  selectedAdapterIds: string[]
  defaultAdapterId: string
  defaultModelId: string
  defaultCatalogModelId?: string
}): Record<string, ProviderAdapterPatch> {
  const adaptersPatch: Record<string, ProviderAdapterPatch> = {}

  for (const adapterModels of input.adapterModelLists) {
    if (adapterModels.error || !input.selectedAdapterIds.includes(adapterModels.adapter_id)) continue
    const configuredModels = configuredProviderModelDefinitions(input.catalogEntries, adapterModels.models)
    adaptersPatch[adapterModels.adapter_id] = {
      enabled: true,
      models: configuredModels,
    }
  }

  const defaultModelEntry = preferredCatalogEntryForLookupIds(input.catalogEntries, [
    input.defaultModelId,
    input.defaultCatalogModelId || '',
  ])
  const existingDefaultModel =
    adaptersPatch[input.defaultAdapterId]?.models?.[input.defaultModelId] || {}
  adaptersPatch[input.defaultAdapterId] = {
    enabled: true,
    models: {
      ...(adaptersPatch[input.defaultAdapterId]?.models || {}),
      [input.defaultModelId]: defaultModelEntry
        ? buildConfiguredProviderModelFromDraft(createModelCatalogDraftFromEntry(defaultModelEntry))
        : existingDefaultModel,
    },
  }

  return adaptersPatch
}
