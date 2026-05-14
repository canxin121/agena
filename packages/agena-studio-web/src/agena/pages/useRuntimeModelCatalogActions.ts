import type { Ref } from 'vue'

import type {
  ModelCatalogEntry,
  ModelCatalogEntryWriteRequest,
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

export const MODEL_LIFECYCLE_OPTIONS = [
  'active',
  'preview',
  'beta',
  'alpha',
  'experimental',
  'deprecated',
] as const

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

function readCapabilityFlag(
  entry: Pick<ModelCatalogEntry, 'capabilities'>,
  key: 'tool_calling' | 'streaming' | 'reasoning' | 'structured_output' | 'temperature_supported',
): boolean {
  const value = entry.capabilities?.[key]
  return value === 'supported' || value === true
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
  }
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
  }
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
    variants: {} satisfies Record<string, ProviderModelVariant>,
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
    const request = buildModelCatalogWriteRequest(draft)
    if (!request.provider_id || !request.model_id) {
      input.actionError.value = 'provider_id and model_id are required.'
      return
    }

    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
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
