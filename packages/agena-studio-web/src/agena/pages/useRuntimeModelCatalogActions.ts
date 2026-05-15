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
  provider_id: string
  model_id: string
  set_default_for_provider: boolean
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

function readCapabilityFlag(
  entry: { capabilities?: Record<string, unknown> | null },
  key: 'tool_calling' | 'streaming' | 'reasoning' | 'structured_output' | 'temperature_supported',
): boolean {
  const value = entry.capabilities?.[key]
  if (value === 'supported' || value === true) return true

  const compactKey = key === 'temperature_supported' ? 'temperature' : key
  const features = entry.capabilities?.features
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

export function createEmptyModelCatalogDraft(providerId = '', modelId = ''): ModelCatalogEditableDraft {
  return {
    provider_id: providerId,
    model_id: modelId,
    set_default_for_provider: false,
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
    .map(([name, variant]) => createModelCatalogVariantDraftFromEntry(name, variant as ProviderModelVariantWithDisabled))
}

export function createModelCatalogDraftFromEntry(entry: ModelCatalogEntry): ModelCatalogEditableDraft {
  return {
    provider_id: entry.provider_id,
    model_id: entry.model_id,
    set_default_for_provider: entry.default_model_for_provider === entry.model_id,
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
  return {
    provider_id: model.provider_id,
    model_id: model.id,
    set_default_for_provider: false,
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
  const matches = entries.filter((entry) => entry.provider_id === model.provider_id && entry.model_id === model.id)
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
  const capabilities: Record<string, unknown> = {}
  if (draft.tool_calling) capabilities.tool_calling = 'supported'
  if (draft.streaming) capabilities.streaming = 'supported'
  if (draft.reasoning) capabilities.reasoning = 'supported'
  if (draft.structured_output) capabilities.structured_output = 'supported'
  if (draft.temperature_supported) capabilities.temperature_supported = 'supported'

  return {
    provider_id: String(draft.provider_id || '').trim(),
    model_id: String(draft.model_id || '').trim(),
    set_default_for_provider: Boolean(draft.set_default_for_provider),
    family: normalizeOptionalText(draft.family),
    lifecycle: normalizeOptionalText(draft.lifecycle),
    context_window_tokens: normalizeOptionalInteger(draft.context_window_tokens),
    max_output_tokens: normalizeOptionalInteger(draft.max_output_tokens),
    display_name: normalizeOptionalText(draft.display_name),
    description: normalizeOptionalText(draft.description),
    variants: buildModelCatalogVariants(draft.variants),
    capabilities,
  }
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
      if (!request.provider_id || !request.model_id) {
        input.actionError.value = 'provider_id and model_id are required.'
        return
      }

      const response = await deps.upsertModelCatalogEntry(request)
      replaceEntries(response.entries ?? [])
      input.actionMessage.value = `Saved catalog entry ${request.provider_id}/${request.model_id}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function deleteCatalogEntryAction(providerId: string, modelId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const response = await deps.deleteModelCatalogEntry(providerId, modelId)
      replaceEntries(response.entries ?? [])
      input.actionMessage.value = `Deleted local catalog override ${providerId}/${modelId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function setCatalogDefaultModelAction(providerId: string, modelId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const response = await deps.setModelCatalogProviderDefault({
        provider_id: providerId,
        model_id: modelId,
      })
      replaceEntries(response.entries ?? [])
      input.actionMessage.value = `Set catalog default for ${providerId} to ${modelId}.`
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
