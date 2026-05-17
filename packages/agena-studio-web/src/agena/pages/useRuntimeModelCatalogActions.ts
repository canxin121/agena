import type { Ref } from 'vue'

import type {
  ModelCatalogEntry,
  ModelCatalogEntryWriteRequest,
  ProviderModel,
  ProviderModelVariant,
} from '../lib/agenaApi'
import {
  deleteModelCatalogEntry,
  refreshModelCatalog,
  setModelCatalogProviderDefault,
  upsertModelCatalogEntry,
} from '../lib/agenaApi'

export type RuntimeModelCatalogActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  catalogEntries: Ref<ModelCatalogEntry[]>
  load: () => Promise<void>
}

export type RuntimeModelCatalogActionsDeps = {
  deleteModelCatalogEntry: typeof deleteModelCatalogEntry
  refreshModelCatalog: typeof refreshModelCatalog
  setModelCatalogProviderDefault: typeof setModelCatalogProviderDefault
  upsertModelCatalogEntry: typeof upsertModelCatalogEntry
}

const defaultDeps: RuntimeModelCatalogActionsDeps = {
  deleteModelCatalogEntry,
  refreshModelCatalog,
  setModelCatalogProviderDefault,
  upsertModelCatalogEntry,
}

export type ModelCatalogEditableDraft = {
  adapter_id: string
  model_id: string
  set_default_for_adapter: boolean
  family: string
  lifecycle: string
  context_window_tokens: string
  max_output_tokens: string
  display_name: string
  description: string
  tool_calling: boolean
  streaming: boolean
  reasoning: boolean
  structured_output: boolean
  temperature_supported: boolean
  variants: ModelCatalogVariantEditableDraft[]
}

export type ModelCatalogVariantEditableDraft = {
  name: string
  display_name: string
  description: string
  disabled: boolean
  thinking_json: string
}

export const MODEL_FAMILY_OPTIONS = [
  'gpt',
  'codex',
  'claude',
  'gemini',
  'llama',
  'mistral',
  'deepseek',
  'qwen',
  'nova',
  'grok',
  'phi',
  'command',
] as const

export const MODEL_LIFECYCLE_OPTIONS = ['active', 'preview', 'beta', 'alpha', 'experimental', 'deprecated'] as const

function normalizeOptionalText(value: string): string | null {
  const normalized = String(value || '').trim()
  return normalized || null
}

function normalizeOptionalInteger(value: string): number | null {
  const normalized = String(value || '').trim()
  if (!normalized) return null
  const parsed = Number(normalized)
  if (!Number.isFinite(parsed) || parsed < 0) return null
  return Math.floor(parsed)
}

function normalizeOptionalJsonObject(value: string, fieldLabel: string): Record<string, unknown> | null {
  const normalized = String(value || '').trim()
  if (!normalized) return null

  let parsed: unknown
  try {
    parsed = JSON.parse(normalized)
  } catch {
    throw new Error(`${fieldLabel} must be valid JSON.`)
  }

  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error(`${fieldLabel} must be a JSON object.`)
  }

  return parsed as Record<string, unknown>
}

function stringifyJson(value: Record<string, unknown> | null | undefined): string {
  if (!value) return ''
  return JSON.stringify(value, null, 2)
}

export function splitProviderModelRoute(modelId: string): { adapterId: string; modelId: string } {
  const normalized = String(modelId || '').trim()
  const slashIndex = normalized.indexOf('/')
  if (slashIndex > 0 && slashIndex < normalized.length - 1) {
    return {
      adapterId: normalized.slice(0, slashIndex).trim(),
      modelId: normalized.slice(slashIndex + 1).trim(),
    }
  }
  return { adapterId: '', modelId: normalized }
}

function readCapabilityFlag(
  entry: { capabilities?: Record<string, unknown> | null; features?: unknown; [key: string]: unknown },
  key: 'tool_calling' | 'streaming' | 'reasoning' | 'structured_output' | 'temperature_supported',
): boolean {
  const value = entry[key] ?? entry.capabilities?.[key]
  if (value === 'supported' || value === true) return true

  const compactKey = key === 'temperature_supported' ? 'temperature' : key
  const features = entry.features ?? entry.capabilities?.features
  if (Array.isArray(features)) {
    return features.includes(compactKey)
  }
  if (features && typeof features === 'object' && !Array.isArray(features)) {
    const supported = (features as Record<string, unknown>).supported
    return Array.isArray(supported) && supported.includes(compactKey)
  }
  return false
}

type ProviderModelVariantWithDisabled = ProviderModelVariant & {
  disabled?: boolean
}

type ProviderModelVariantWriteValue = ProviderModelVariant & {
  disabled?: boolean
}

export function createEmptyModelCatalogDraft(adapterId = '', modelId = ''): ModelCatalogEditableDraft {
  return {
    adapter_id: adapterId,
    model_id: modelId,
    set_default_for_adapter: false,
    family: '',
    lifecycle: '',
    context_window_tokens: '',
    max_output_tokens: '',
    display_name: '',
    description: '',
    tool_calling: false,
    streaming: false,
    reasoning: false,
    structured_output: false,
    temperature_supported: false,
    variants: [],
  }
}

export function createEmptyModelCatalogVariantDraft(name = ''): ModelCatalogVariantEditableDraft {
  return {
    name,
    display_name: '',
    description: '',
    disabled: false,
    thinking_json: '',
  }
}

function createModelCatalogVariantDraftFromEntry(
  name: string,
  variant: ProviderModelVariantWithDisabled,
): ModelCatalogVariantEditableDraft {
  return {
    name,
    display_name: String(variant.display_name || ''),
    description: String(variant.description || ''),
    disabled: Boolean(variant.disabled),
    thinking_json: stringifyJson(variant.thinking || null),
  }
}

function createModelCatalogVariantDrafts(
  variants: Record<string, ProviderModelVariant> | null | undefined,
): ModelCatalogVariantEditableDraft[] {
  return Object.entries(variants || {})
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, variant]) =>
      createModelCatalogVariantDraftFromEntry(name, variant as ProviderModelVariantWithDisabled),
    )
}

export function createModelCatalogDraftFromEntry(entry: ModelCatalogEntry): ModelCatalogEditableDraft {
  return {
    adapter_id: entry.adapter_id || entry.provider_id || '',
    model_id: entry.model_id,
    set_default_for_adapter: (entry.default_model_for_adapter || entry.default_model_for_provider) === entry.model_id,
    family: String(entry.family || ''),
    lifecycle: String(entry.lifecycle || ''),
    context_window_tokens: entry.context_window_tokens == null ? '' : String(entry.context_window_tokens),
    max_output_tokens: entry.max_output_tokens == null ? '' : String(entry.max_output_tokens),
    display_name: String(entry.display_name || ''),
    description: String(entry.description || ''),
    tool_calling: readCapabilityFlag(entry, 'tool_calling'),
    streaming: readCapabilityFlag(entry, 'streaming'),
    reasoning: readCapabilityFlag(entry, 'reasoning'),
    structured_output: readCapabilityFlag(entry, 'structured_output'),
    temperature_supported: readCapabilityFlag(entry, 'temperature_supported'),
    variants: createModelCatalogVariantDrafts(entry.variants),
  }
}

export function createModelCatalogDraftFromProviderModel(model: ProviderModel): ModelCatalogEditableDraft {
  const route = splitProviderModelRoute(model.id)
  const adapterId = route.adapterId || String(model.provider_id || '').trim()
  return {
    adapter_id: adapterId,
    model_id: route.modelId,
    set_default_for_adapter: false,
    family: String(model.metadata?.family || ''),
    lifecycle: String(model.metadata?.lifecycle || ''),
    context_window_tokens:
      model.metadata?.limits?.context_window_tokens == null ? '' : String(model.metadata.limits.context_window_tokens),
    max_output_tokens:
      model.metadata?.limits?.max_output_tokens == null ? '' : String(model.metadata.limits.max_output_tokens),
    display_name: String(model.display_name || ''),
    description: String(model.metadata?.description || ''),
    tool_calling: readCapabilityFlag(model, 'tool_calling'),
    streaming: readCapabilityFlag(model, 'streaming'),
    reasoning: readCapabilityFlag(model, 'reasoning'),
    structured_output: readCapabilityFlag(model, 'structured_output'),
    temperature_supported: readCapabilityFlag(model, 'temperature_supported'),
    variants: createModelCatalogVariantDrafts(model.variants),
  }
}

export function findCatalogEntryForProviderModel(
  entries: ModelCatalogEntry[],
  model: ProviderModel,
): ModelCatalogEntry | null {
  const route = splitProviderModelRoute(model.id)
  const adapterId = route.adapterId || String(model.provider_id || '').trim()
  const matches = entries.filter(
    (entry) => (entry.adapter_id || entry.provider_id) === adapterId && entry.model_id === route.modelId,
  )
  return matches.find((entry) => entry.kind === 'custom') || matches[0] || null
}

export function createModelCatalogDraftFromProviderSelection(
  entries: ModelCatalogEntry[],
  model: ProviderModel,
): ModelCatalogEditableDraft {
  const matchingEntry = findCatalogEntryForProviderModel(entries, model)
  return matchingEntry
    ? createModelCatalogDraftFromEntry(matchingEntry)
    : createModelCatalogDraftFromProviderModel(model)
}

function buildModelCatalogVariants(
  variants: ModelCatalogVariantEditableDraft[],
): Record<string, ProviderModelVariant> | undefined {
  const normalized: Record<string, ProviderModelVariantWriteValue> = {}
  const seenNames = new Set<string>()

  for (const variant of variants) {
    const name = String(variant.name || '').trim()
    const displayName = normalizeOptionalText(variant.display_name)
    const description = normalizeOptionalText(variant.description)
    const thinking = normalizeOptionalJsonObject(variant.thinking_json, `Variant ${name || '(unnamed)'} thinking`)
    const disabled = Boolean(variant.disabled)
    const hasDetails = Boolean(displayName || description || thinking || disabled)

    if (!name) {
      if (!hasDetails) continue
      throw new Error('Variant name is required when variant details are provided.')
    }

    if (seenNames.has(name)) {
      throw new Error(`Variant ${name} is listed more than once.`)
    }
    seenNames.add(name)

    const nextVariant: ProviderModelVariantWriteValue = {}
    if (displayName) nextVariant.display_name = displayName
    if (description) nextVariant.description = description
    if (thinking) nextVariant.thinking = thinking
    if (disabled) nextVariant.disabled = true
    normalized[name] = nextVariant
  }

  return Object.keys(normalized).length ? (normalized as Record<string, ProviderModelVariant>) : undefined
}

export function buildModelCatalogWriteRequest(draft: ModelCatalogEditableDraft): ModelCatalogEntryWriteRequest {
  const supportedFeatures: string[] = []
  if (draft.tool_calling) supportedFeatures.push('tool_calling')
  if (draft.streaming) supportedFeatures.push('streaming')
  if (draft.reasoning) supportedFeatures.push('reasoning')
  if (draft.structured_output) supportedFeatures.push('structured_output')
  if (draft.temperature_supported) supportedFeatures.push('temperature')

  return {
    adapter_id: String(draft.adapter_id || '').trim(),
    model_id: String(draft.model_id || '').trim(),
    set_default_for_adapter: Boolean(draft.set_default_for_adapter),
    family: normalizeOptionalText(draft.family),
    lifecycle: normalizeOptionalText(draft.lifecycle),
    context_window_tokens: normalizeOptionalInteger(draft.context_window_tokens),
    max_output_tokens: normalizeOptionalInteger(draft.max_output_tokens),
    display_name: normalizeOptionalText(draft.display_name),
    description: normalizeOptionalText(draft.description),
    variants: buildModelCatalogVariants(draft.variants),
    features: supportedFeatures.length ? { supported: supportedFeatures } : undefined,
  }
}

export function buildConfiguredProviderModelFromDraft(draft: ModelCatalogEditableDraft): Record<string, unknown> {
  const request = buildModelCatalogWriteRequest(draft) as Record<string, unknown>
  delete request.adapter_id
  delete request.provider_id
  delete request.model_id
  delete request.set_default_for_adapter
  delete request.set_default_for_provider

  for (const [key, value] of Object.entries({ ...request })) {
    const isEmptyObject =
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value as Record<string, unknown>).length === 0
    if (value == null || value === '' || isEmptyObject) {
      delete request[key]
    }
  }

  return request
}

export function useRuntimeModelCatalogActions(
  input: RuntimeModelCatalogActionsInput,
  deps: RuntimeModelCatalogActionsDeps = defaultDeps,
) {
  function replaceEntries(nextEntries: ModelCatalogEntry[]) {
    input.catalogEntries.value = nextEntries
  }

  async function refreshCatalogAction() {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const response = await deps.refreshModelCatalog()
      replaceEntries(response.entries ?? [])
      input.actionMessage.value = 'Refreshed model catalog.'
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function saveCatalogEntryAction(draft: ModelCatalogEditableDraft) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const request = buildModelCatalogWriteRequest(draft)
      if (!request.adapter_id || !request.model_id) {
        input.actionError.value = 'adapter_id and model_id are required.'
        return
      }

      const response = await deps.upsertModelCatalogEntry(request)
      replaceEntries(response.entries ?? [])
      input.actionMessage.value = `Saved catalog entry ${request.adapter_id}/${request.model_id}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function deleteCatalogEntryAction(adapterId: string, modelId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const response = await deps.deleteModelCatalogEntry(adapterId, modelId)
      replaceEntries(response.entries ?? [])
      input.actionMessage.value = `Deleted local catalog override ${adapterId}/${modelId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function setCatalogDefaultModelAction(adapterId: string, modelId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const response = await deps.setModelCatalogProviderDefault({
        adapter_id: adapterId,
        model_id: modelId,
      })
      replaceEntries(response.entries ?? [])
      input.actionMessage.value = `Set catalog default for ${adapterId} to ${modelId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  return {
    deleteCatalogEntryAction,
    refreshCatalogAction,
    saveCatalogEntryAction,
    setCatalogDefaultModelAction,
  }
}
