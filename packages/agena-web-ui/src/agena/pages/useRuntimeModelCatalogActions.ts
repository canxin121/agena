import { userErrorMessage } from '@/lib/api'
import type { Ref } from 'vue'

import type {
  ModelCatalogEntry,
  ModelCatalogRefreshResponse,
  ProviderModel,
  ProviderModelPricing,
  ProviderModelSpeedMode,
  ProviderModelSpeedModeRequestOverride,
  ProviderModelThinkingMode,
} from '../lib/agenaApi'
import { providerModelThinkingModeSelector, refreshModelCatalog } from '../lib/agenaApi'

export type RuntimeModelCatalogActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  reloadCatalogEntries?: () => Promise<void>
}

export type RuntimeModelCatalogActionsDeps = {
  refreshModelCatalog: typeof refreshModelCatalog
}

const defaultDeps: RuntimeModelCatalogActionsDeps = {
  refreshModelCatalog,
}

export type ModelCatalogEditableDraft = {
  adapter_id: string
  model_id: string
  lifecycle: string
  context_window_tokens: string
  max_input_tokens: string
  max_output_tokens: string
  display_name: string
  origin: string
  description: string
  default_temperature: string
  default_top_p: string
  default_top_k: string
  assistant_reasoning_interleaved: boolean
  assistant_reasoning_field: string
  output_modalities_json: string
  pricing_json: string
  agena_tool_mode: 'provider_protocol' | 'prompt_envelope' | 'disabled'
  provider_native_json: string
  tool_calling: boolean
  streaming: boolean
  reasoning: boolean
  structured_output: boolean
  temperature_supported: boolean
  thinking_modes: ModelCatalogThinkingModeEditableDraft[]
  speed_modes: ModelCatalogSpeedModeEditableDraft[]
}

export type ModelCatalogThinkingModeEditableDraft = {
  name: string
  default: boolean
  display_name: string
  description: string
  disabled: boolean
  thinking_json: string
  request_override_json: string
  adapter_overrides_json: string
}

export type ModelCatalogSpeedModeEditableDraft = {
  name: string
  default: boolean
  display_name: string
  description: string
  disabled: boolean
  request_override_json: string
  adapter_overrides_json: string
}

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

function stringifyJsonValue(value: unknown): string {
  if (value == null) return ''
  return JSON.stringify(value, null, 2)
}

function normalizeOptionalStringArray(value: string, fieldLabel: string): string[] | null {
  const normalized = String(value || '').trim()
  if (!normalized) return null

  let parsed: unknown
  try {
    parsed = JSON.parse(normalized)
  } catch {
    throw new Error(`${fieldLabel} must be valid JSON.`)
  }

  if (!Array.isArray(parsed) || parsed.some((item) => typeof item !== 'string')) {
    throw new Error(`${fieldLabel} must be a JSON array of strings.`)
  }

  const values = parsed.map((item) => String(item).trim()).filter(Boolean)
  return values.length ? [...new Set(values)] : null
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

type ProviderModelThinkingModeWithDisabled = ProviderModelThinkingMode & {
  disabled?: boolean
}

type ProviderModelSpeedModeWithDisabled = ProviderModelSpeedMode & {
  disabled?: boolean
}

type ProviderModelThinkingModeWriteValue = ProviderModelThinkingMode & {
  disabled?: boolean
}

type ProviderModelSpeedModeWriteValue = ProviderModelSpeedMode & {
  disabled?: boolean
}

function normalizeOptionalSpeedModeRequestOverride(
  value: string,
  fieldLabel: string,
): ProviderModelSpeedModeRequestOverride | null {
  return normalizeOptionalJsonObject(value, fieldLabel) as ProviderModelSpeedModeRequestOverride | null
}

function normalizeOptionalSpeedModeRequestOverrideMap(
  value: string,
  fieldLabel: string,
): Record<string, ProviderModelSpeedModeRequestOverride> | null {
  return normalizeOptionalJsonObject(value, fieldLabel) as Record<string, ProviderModelSpeedModeRequestOverride> | null
}

export function createEmptyModelCatalogDraft(adapterId = '', modelId = ''): ModelCatalogEditableDraft {
  return {
    adapter_id: adapterId,
    model_id: modelId,
    lifecycle: '',
    context_window_tokens: '',
    max_input_tokens: '',
    max_output_tokens: '',
    display_name: '',
    origin: '',
    description: '',
    default_temperature: '',
    default_top_p: '',
    default_top_k: '',
    assistant_reasoning_interleaved: false,
    assistant_reasoning_field: '',
    output_modalities_json: '',
    pricing_json: '',
    agena_tool_mode: 'disabled',
    provider_native_json: '',
    tool_calling: false,
    streaming: false,
    reasoning: false,
    structured_output: false,
    temperature_supported: false,
    thinking_modes: [],
    speed_modes: [],
  }
}

export function createEmptyModelCatalogThinkingModeDraft(name = ''): ModelCatalogThinkingModeEditableDraft {
  return {
    name,
    default: false,
    display_name: '',
    description: '',
    disabled: false,
    thinking_json: '',
    request_override_json: '',
    adapter_overrides_json: '',
  }
}

export function createEmptyModelCatalogSpeedModeDraft(name = ''): ModelCatalogSpeedModeEditableDraft {
  return {
    name,
    default: false,
    display_name: '',
    description: '',
    disabled: false,
    request_override_json: '',
    adapter_overrides_json: '',
  }
}

function createThinkingModeDraftFromEntry(
  mode: ProviderModelThinkingModeWithDisabled,
): ModelCatalogThinkingModeEditableDraft {
  return {
    name: providerModelThinkingModeSelector(mode),
    default: Boolean(mode.default),
    display_name: String(mode.display_name || ''),
    description: String(mode.description || ''),
    disabled: Boolean(mode.disabled),
    thinking_json: stringifyJson(
      mode.thinking ||
        (mode.strategy === 'disabled'
          ? { type: 'disabled' }
          : mode.strategy === 'effort'
            ? { type: 'effort', effort: mode.effort }
            : mode.strategy === 'budget'
              ? { type: 'budget', budget_tokens: mode.budget_tokens }
              : mode.strategy === 'adaptive'
                ? { type: 'adaptive', effort: mode.effort, display: mode.display }
                : null),
    ),
    request_override_json: stringifyJson(mode.request_override || null),
    adapter_overrides_json: stringifyJson(mode.adapter_overrides || null),
  }
}

function createSpeedModeDraftFromEntry(
  name: string,
  mode: ProviderModelSpeedModeWithDisabled,
): ModelCatalogSpeedModeEditableDraft {
  return {
    name,
    default: Boolean(mode.default),
    display_name: String(mode.display_name || ''),
    description: String(mode.description || ''),
    disabled: Boolean(mode.disabled),
    request_override_json: stringifyJson(mode.request_override || null),
    adapter_overrides_json: stringifyJson(mode.adapter_overrides || null),
  }
}

function createThinkingModeDrafts(
  modes: ProviderModelThinkingMode[] | null | undefined,
): ModelCatalogThinkingModeEditableDraft[] {
  return (modes || []).map((mode) => createThinkingModeDraftFromEntry(mode))
}

function createCatalogThinkingModeDrafts(
  modes: Record<string, ProviderModelThinkingMode | string | null> | null | undefined,
): ModelCatalogThinkingModeEditableDraft[] {
  const defaultName = typeof modes?.default === 'string' ? modes.default : ''
  return Object.entries(modes || {}).flatMap(([name, mode]) =>
    name !== 'default' && mode && typeof mode === 'object'
      ? [{ ...createThinkingModeDraftFromEntry(mode), name, default: name === defaultName }]
      : [],
  )
}

function createSpeedModeDrafts(
  modes: Record<string, ProviderModelSpeedMode | string | null> | null | undefined,
): ModelCatalogSpeedModeEditableDraft[] {
  const defaultName = typeof modes?.default === 'string' ? modes.default : ''
  return Object.entries(modes || {})
    .filter(([name, mode]) => name !== 'default' && mode && typeof mode === 'object')
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, mode]) => ({
      ...createSpeedModeDraftFromEntry(name, mode as ProviderModelSpeedModeWithDisabled),
      default: name === defaultName,
    }))
}

export function createModelCatalogDraftFromEntry(entry: ModelCatalogEntry): ModelCatalogEditableDraft {
  return {
    adapter_id: '',
    model_id: entry.model_id,
    lifecycle: String(entry.lifecycle || ''),
    context_window_tokens: entry.context_window_tokens == null ? '' : String(entry.context_window_tokens),
    max_input_tokens: entry.max_input_tokens == null ? '' : String(entry.max_input_tokens),
    max_output_tokens: entry.max_output_tokens == null ? '' : String(entry.max_output_tokens),
    display_name: String(entry.display_name || ''),
    origin: String(entry.origin || ''),
    description: String(entry.description || ''),
    default_temperature: String(entry.default_temperature || ''),
    default_top_p: String(entry.default_top_p || ''),
    default_top_k: entry.default_top_k == null ? '' : String(entry.default_top_k),
    assistant_reasoning_interleaved: Boolean(entry.assistant_reasoning_interleaved),
    assistant_reasoning_field: String(entry.assistant_reasoning_field || ''),
    output_modalities_json: stringifyJsonValue(entry.output_modalities || null),
    pricing_json: stringifyJsonValue(entry.pricing || null),
    agena_tool_mode: readCapabilityFlag(entry, 'tool_calling') ? 'provider_protocol' : 'disabled',
    provider_native_json: '',
    tool_calling: readCapabilityFlag(entry, 'tool_calling'),
    streaming: readCapabilityFlag(entry, 'streaming'),
    reasoning: readCapabilityFlag(entry, 'reasoning'),
    structured_output: readCapabilityFlag(entry, 'structured_output'),
    temperature_supported: readCapabilityFlag(entry, 'temperature_supported'),
    thinking_modes: createCatalogThinkingModeDrafts(entry.thinking_modes),
    speed_modes: createSpeedModeDrafts(entry.speed_modes),
  }
}

export function createModelCatalogDraftFromProviderModel(model: ProviderModel): ModelCatalogEditableDraft {
  const adapterId = String(model.adapter_id || '').trim()
  return {
    adapter_id: adapterId,
    model_id: model.id,
    lifecycle: String(model.metadata?.lifecycle || ''),
    context_window_tokens:
      model.metadata?.limits?.context_window_tokens == null ? '' : String(model.metadata.limits.context_window_tokens),
    max_input_tokens:
      model.metadata?.limits?.max_input_tokens == null ? '' : String(model.metadata.limits.max_input_tokens),
    max_output_tokens:
      model.metadata?.limits?.max_output_tokens == null ? '' : String(model.metadata.limits.max_output_tokens),
    display_name: String(model.display_name || ''),
    origin: '',
    description: String(model.metadata?.description || ''),
    default_temperature: String(model.metadata?.default_temperature || ''),
    default_top_p: String(model.metadata?.default_top_p || ''),
    default_top_k: model.metadata?.default_top_k == null ? '' : String(model.metadata.default_top_k),
    assistant_reasoning_interleaved: Boolean(model.metadata?.assistant_reasoning_interleaved),
    assistant_reasoning_field: String(model.metadata?.assistant_reasoning_field || ''),
    output_modalities_json: stringifyJsonValue(model.metadata?.output_modalities || null),
    pricing_json: stringifyJsonValue(model.metadata?.pricing || null),
    agena_tool_mode: readCapabilityFlag(model, 'tool_calling') ? 'provider_protocol' : 'disabled',
    provider_native_json: '',
    tool_calling: readCapabilityFlag(model, 'tool_calling'),
    streaming: readCapabilityFlag(model, 'streaming'),
    reasoning: readCapabilityFlag(model, 'reasoning'),
    structured_output: readCapabilityFlag(model, 'structured_output'),
    temperature_supported: readCapabilityFlag(model, 'temperature_supported'),
    thinking_modes: createThinkingModeDrafts(model.thinking_modes),
    speed_modes: createSpeedModeDrafts(model.speed_modes),
  }
}

export function catalogLookupIdForProviderModel(model: ProviderModel): string {
  const lookupId = String(model.catalog_model_id || '').trim()
  return lookupId || String(model.id || '').trim()
}

export function findCatalogEntryForProviderModel(
  entries: ModelCatalogEntry[],
  model: ProviderModel,
): ModelCatalogEntry | null {
  const lookupIds = [
    ...new Set([String(model.id || '').trim(), catalogLookupIdForProviderModel(model)].filter(Boolean)),
  ]
  const matches = entries.filter((entry) => lookupIds.includes(entry.model_id))
  return matches[0] || null
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

function buildModelCatalogThinkingModes(
  modes: ModelCatalogThinkingModeEditableDraft[],
): Record<string, ProviderModelThinkingMode | string> | undefined {
  const normalized: Record<string, ProviderModelThinkingMode | string> = {}
  const seenNames = new Set<string>()
  let hasDefault = false

  for (const mode of modes) {
    const name = String(mode.name || '').trim()
    const displayName = normalizeOptionalText(mode.display_name)
    const description = normalizeOptionalText(mode.description)
    const thinking = normalizeOptionalJsonObject(mode.thinking_json, `Thinking mode ${name || '(unnamed)'}`)
    const requestOverride = normalizeOptionalSpeedModeRequestOverride(
      mode.request_override_json,
      `Thinking mode ${name || '(unnamed)'} request override`,
    )
    const adapterOverrides = normalizeOptionalSpeedModeRequestOverrideMap(
      mode.adapter_overrides_json,
      `Thinking mode ${name || '(unnamed)'} adapter overrides`,
    )
    const disabled = Boolean(mode.disabled)
    const hasDetails = Boolean(
      displayName || description || thinking || requestOverride || adapterOverrides || disabled || mode.default,
    )

    if (!name) {
      if (!hasDetails) continue
      throw new Error('Thinking mode name is required when thinking mode details are provided.')
    }
    if (seenNames.has(name)) {
      throw new Error(`Thinking mode ${name} is listed more than once.`)
    }
    seenNames.add(name)
    if (mode.default && hasDefault) {
      throw new Error('Only one thinking mode can be the default.')
    }
    hasDefault ||= mode.default
    if (mode.default && disabled) {
      throw new Error(`Thinking mode ${name} cannot be both default and disabled.`)
    }

    const nextMode: ProviderModelThinkingModeWriteValue = {}
    if (displayName) nextMode.display_name = displayName
    if (description) nextMode.description = description
    if (thinking?.type === 'disabled') {
      nextMode.strategy = 'disabled'
    } else if (thinking?.type === 'effort') {
      nextMode.strategy = 'effort'
      nextMode.effort = typeof thinking.effort === 'string' ? thinking.effort : null
    } else if (thinking?.type === 'budget') {
      nextMode.strategy = 'budget'
      nextMode.budget_tokens = typeof thinking.budget_tokens === 'number' ? thinking.budget_tokens : null
    } else if (thinking?.type === 'adaptive') {
      nextMode.strategy = 'adaptive'
      nextMode.effort = typeof thinking.effort === 'string' ? thinking.effort : null
      nextMode.display = typeof thinking.display === 'string' ? thinking.display : null
    } else if (!thinking && (requestOverride || adapterOverrides)) {
      nextMode.strategy = 'request_only'
    }
    if (requestOverride) nextMode.request_override = requestOverride
    if (adapterOverrides) nextMode.adapter_overrides = adapterOverrides
    if (disabled) nextMode.disabled = true
    normalized[name] = nextMode
    if (mode.default) normalized.default = name
  }

  return Object.keys(normalized).length ? normalized : undefined
}

function buildModelCatalogSpeedModes(
  modes: ModelCatalogSpeedModeEditableDraft[],
): Record<string, ProviderModelSpeedMode | string> | undefined {
  const normalized: Record<string, ProviderModelSpeedModeWriteValue | string> = {}
  const seenNames = new Set<string>()
  let hasDefault = false

  for (const mode of modes) {
    const name = String(mode.name || '').trim()
    const displayName = normalizeOptionalText(mode.display_name)
    const description = normalizeOptionalText(mode.description)
    const requestOverride = normalizeOptionalSpeedModeRequestOverride(
      mode.request_override_json,
      `Speed mode ${name || '(unnamed)'} request override`,
    )
    const adapterOverrides = normalizeOptionalSpeedModeRequestOverrideMap(
      mode.adapter_overrides_json,
      `Speed mode ${name || '(unnamed)'} adapter overrides`,
    )
    const disabled = Boolean(mode.disabled)
    const hasDetails = Boolean(
      displayName || description || requestOverride || adapterOverrides || disabled || mode.default,
    )

    if (!name) {
      if (!hasDetails) continue
      throw new Error('Speed mode name is required when speed mode details are provided.')
    }
    if (seenNames.has(name)) {
      throw new Error(`Speed mode ${name} is listed more than once.`)
    }
    seenNames.add(name)
    if (mode.default && hasDefault) {
      throw new Error('Only one speed mode can be the default.')
    }
    hasDefault ||= mode.default
    if (mode.default && disabled) {
      throw new Error(`Speed mode ${name} cannot be both default and disabled.`)
    }

    const nextMode: ProviderModelSpeedModeWriteValue = {}
    if (displayName) nextMode.display_name = displayName
    if (description) nextMode.description = description
    if (requestOverride) nextMode.request_override = requestOverride
    if (adapterOverrides) nextMode.adapter_overrides = adapterOverrides
    if (disabled) nextMode.disabled = true
    normalized[name] = nextMode
    if (mode.default) normalized.default = name
  }

  return Object.keys(normalized).length ? normalized : undefined
}

function buildModelCatalogDefinitionRecord(draft: ModelCatalogEditableDraft): Record<string, unknown> {
  const supportedFeatures: string[] = []
  if (draft.tool_calling) supportedFeatures.push('tool_calling')
  if (draft.streaming) supportedFeatures.push('streaming')
  if (draft.reasoning) supportedFeatures.push('reasoning')
  if (draft.structured_output) supportedFeatures.push('structured_output')
  if (draft.temperature_supported) supportedFeatures.push('temperature')

  return {
    model_id: String(draft.model_id || '').trim(),
    lifecycle: normalizeOptionalText(draft.lifecycle),
    context_window_tokens: normalizeOptionalInteger(draft.context_window_tokens),
    max_input_tokens: normalizeOptionalInteger(draft.max_input_tokens),
    max_output_tokens: normalizeOptionalInteger(draft.max_output_tokens),
    display_name: normalizeOptionalText(draft.display_name),
    origin: normalizeOptionalText(draft.origin),
    description: normalizeOptionalText(draft.description),
    default_temperature: normalizeOptionalText(draft.default_temperature),
    default_top_p: normalizeOptionalText(draft.default_top_p),
    default_top_k: normalizeOptionalInteger(draft.default_top_k),
    assistant_reasoning_interleaved: draft.assistant_reasoning_interleaved ? true : null,
    assistant_reasoning_field: normalizeOptionalText(draft.assistant_reasoning_field),
    output_modalities: normalizeOptionalStringArray(draft.output_modalities_json, 'Output modalities'),
    pricing: normalizeOptionalJsonObject(draft.pricing_json, 'Pricing') as ProviderModelPricing | null,
    thinking_modes: buildModelCatalogThinkingModes(draft.thinking_modes),
    speed_modes: buildModelCatalogSpeedModes(draft.speed_modes),
    features: supportedFeatures.length ? { supported: supportedFeatures } : undefined,
  }
}

export function buildConfiguredProviderModelFromDraft(draft: ModelCatalogEditableDraft): Record<string, unknown> {
  const request = buildModelCatalogDefinitionRecord(draft)
  delete request.model_id
  delete request.origin
  const providerNative =
    draft.agena_tool_mode === 'provider_protocol'
      ? normalizeOptionalJsonObject(draft.provider_native_json, 'agena_tools.provider_native')
      : null
  request.agena_tools =
    draft.agena_tool_mode === 'provider_protocol'
      ? { mode: draft.agena_tool_mode, provider_native: providerNative }
      : { mode: draft.agena_tool_mode, provider_native: null }

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
  async function refreshCatalogAction(): Promise<ModelCatalogRefreshResponse> {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const result = await deps.refreshModelCatalog()
      input.actionMessage.value = result.started
        ? 'Started model catalog refresh.'
        : 'Model catalog refresh is already running.'
      await input.load()
      return result
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
      throw err
    }
  }

  return {
    refreshCatalogAction,
  }
}
