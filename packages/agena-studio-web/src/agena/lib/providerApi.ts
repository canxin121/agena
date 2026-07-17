import { apiJson } from '../../lib/api'

export type ProviderProtocolPaths = {
  openai?: string
  anthropic?: string
  gemini?: string
}

export type ProviderAdapterSummaryResource = {
  adapter_id: string
  enabled: boolean
  configured_model_count: number
}

export type ProviderNativeToolBindingResource = {
  tool: string
  route: string
}

export type ProviderNativeToolsSummaryResource = {
  active: boolean
  model_count: number
  bindings?: ProviderNativeToolBindingResource[]
}

export type ProviderDefaultsResource = {
  adapter?: string | null
  model: string
}

export type ProviderSummaryResource = {
  provider_id: string
  defaults: ProviderDefaultsResource
  adapters?: ProviderAdapterSummaryResource[]
  provider_native_tools?: ProviderNativeToolsSummaryResource | null
}

export type ProviderAdapterSummary = ProviderAdapterSummaryResource
export type ProviderSummary = ProviderSummaryResource

export type CapabilitySupportValue = 'supported' | 'unsupported' | 'unknown'

export type ProviderModelPricingTier = {
  tier_type?: string | null
  size_tokens?: number | null
  input_usd_per_million_tokens?: string | null
  output_usd_per_million_tokens?: string | null
  cache_read_usd_per_million_tokens?: string | null
  cache_write_usd_per_million_tokens?: string | null
}

export type ProviderModelPricing = {
  input_usd_per_million_tokens?: string | null
  output_usd_per_million_tokens?: string | null
  cache_read_usd_per_million_tokens?: string | null
  cache_write_usd_per_million_tokens?: string | null
  tiers?: ProviderModelPricingTier[] | null
}

export type ProviderModelSpeedModeRequestOverride = {
  headers?: Record<string, string> | null
  body_patch?: Record<string, unknown> | null
}

export type ProviderModelThinkingMode = {
  default?: boolean
  preset?: string | null
  display_name?: string | null
  description?: string | null
  thinking?: Record<string, unknown> | null
  strategy?: 'disabled' | 'effort' | 'budget' | 'adaptive' | 'request_only' | null
  effort?: string | null
  budget_tokens?: number | null
  display?: string | null
  request_override?: ProviderModelSpeedModeRequestOverride | null
  adapter_overrides?: Record<string, ProviderModelSpeedModeRequestOverride> | null
  disabled?: boolean
}

export function providerModelThinkingModeSelector(mode: ProviderModelThinkingMode): string {
  const configuredName = String(mode.preset || '').trim()
  if (configuredName) return configuredName
  const thinking = mode.thinking
  if (thinking?.type === 'disabled') return 'off'
  if (
    (thinking?.type === 'effort' || thinking?.type === 'adaptive') &&
    typeof thinking.effort === 'string' &&
    thinking.effort.trim()
  ) {
    return thinking.effort.trim()
  }
  return ''
}

export type ProviderModelSpeedMode = {
  default?: boolean
  display_name?: string | null
  description?: string | null
  request_override?: ProviderModelSpeedModeRequestOverride | null
  adapter_overrides?: Record<string, ProviderModelSpeedModeRequestOverride> | null
  disabled?: boolean
}

export type ProviderModelCapabilities = {
  text_input?: CapabilitySupportValue | null
  image_input?: CapabilitySupportValue | null
  document_input?: CapabilitySupportValue | null
  audio_input?: CapabilitySupportValue | null
  video_input?: CapabilitySupportValue | null
  file_input?: CapabilitySupportValue | null
  tool_calling?: CapabilitySupportValue | null
  streaming?: CapabilitySupportValue | null
  reasoning?: CapabilitySupportValue | null
  structured_output?: CapabilitySupportValue | null
  temperature_supported?: CapabilitySupportValue | null
}

export type ProviderModelMetadata = {
  lifecycle?: string | null
  description?: string | null
  knowledge_cutoff?: string | null
  release_date?: string | null
  last_updated?: string | null
  open_weights?: boolean | null
  supports_parallel_tool_calls?: boolean | null
  supports_verbosity?: boolean | null
  default_verbosity?: string | null
  default_temperature?: string | null
  default_top_p?: string | null
  default_top_k?: number | null
  assistant_reasoning_interleaved?: boolean | null
  assistant_reasoning_field?: string | null
  output_modalities?: string[] | null
  pricing?: ProviderModelPricing | null
  limits?: {
    context_window_tokens?: number | null
    max_input_tokens?: number | null
    max_output_tokens?: number | null
  } | null
}

export type ProviderModel = {
  provider_id: string
  adapter_id?: string | null
  id: string
  catalog_model_id?: string | null
  display_name?: string | null
  capabilities?: ProviderModelCapabilities | null
  metadata?: ProviderModelMetadata | null
  thinking_modes?: ProviderModelThinkingMode[]
  speed_modes?: Record<string, ProviderModelSpeedMode>
}

export type ProviderModelsResponse = {
  provider_id: string
  models: ProviderModel[]
}

export type ProviderAdapterModelsResource = {
  adapter_id: string
  enabled: boolean
  resolved_base_url?: string | null
  models: ProviderModel[]
  error?: string | null
}

export type ProviderAdapterModelsResponse = {
  provider_id: string
  adapters: ProviderAdapterModelsResource[]
}

export type ProviderAdapterModels = ProviderAdapterModelsResource

export type ProviderSecretSource = {
  kind: 'inline' | 'env'
  value: string
}

export type ProviderAdapterModelsRequest = {
  provider_id?: string | null
  base_url: string
  protocol_paths?: ProviderProtocolPaths
  api_key?: ProviderSecretSource | null
  adapter_ids: string[]
}

export type SavedProviderAdapterModelsRequest = {
  adapter_ids: string[]
}

export async function listProviders(): Promise<ProviderSummary[]> {
  return await apiJson<ProviderSummaryResource[]>('/api/v1/providers')
}

export async function listProviderModels(providerId: string): Promise<ProviderModel[]> {
  const response = await apiJson<ProviderModelsResponse>(`/api/v1/providers/${encodeURIComponent(providerId)}/models`)
  return response.models ?? []
}

export async function listProviderAdapterModels(
  request: ProviderAdapterModelsRequest,
): Promise<ProviderAdapterModelsResponse> {
  const adapterIds = request.adapter_ids.map((adapterId) => String(adapterId || '').trim())
  return await apiJson<ProviderAdapterModelsResponse>('/api/v1/providers/models', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(String(request.provider_id || '').trim() ? { provider_id: String(request.provider_id).trim() } : {}),
      base_url: String(request.base_url || '').trim(),
      ...(request.protocol_paths ? { protocol_paths: request.protocol_paths } : {}),
      ...(request.api_key?.value?.trim()
        ? {
            api_key: {
              kind: request.api_key.kind,
              value: request.api_key.value.trim(),
            },
          }
        : {}),
      adapter_ids: adapterIds,
    }),
  })
}

export async function listSavedProviderAdapterModelsResponse(
  providerId: string,
  request: SavedProviderAdapterModelsRequest,
): Promise<ProviderAdapterModelsResponse> {
  const adapterIds = request.adapter_ids.map((adapterId) => String(adapterId || '').trim())
  return await apiJson<ProviderAdapterModelsResponse>(`/api/v1/providers/${encodeURIComponent(providerId)}/models`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      adapter_ids: adapterIds,
    }),
  })
}

export async function listDraftProviderAdapterModels(input: {
  providerId?: string
  baseUrl: string
  protocolPaths?: ProviderProtocolPaths
  apiKey?: ProviderSecretSource
  adapterIds: string[]
}): Promise<ProviderAdapterModels[]> {
  const response = await listProviderAdapterModels({
    provider_id: input.providerId?.trim() || undefined,
    base_url: input.baseUrl.trim(),
    protocol_paths: input.protocolPaths,
    api_key: input.apiKey?.value?.trim()
      ? {
          kind: input.apiKey.kind,
          value: input.apiKey.value.trim(),
        }
      : undefined,
    adapter_ids: input.adapterIds,
  })
  return response.adapters ?? []
}

export async function listSavedProviderAdapterModels(
  providerId: string,
  input: {
    adapterIds: string[]
  },
): Promise<ProviderAdapterModels[]> {
  const response = await listSavedProviderAdapterModelsResponse(providerId, {
    adapter_ids: input.adapterIds,
  })
  return response.adapters ?? []
}
