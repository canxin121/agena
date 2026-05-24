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
  enabled: boolean
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
  native_tools?: ProviderNativeToolsSummaryResource | null
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
  display_name?: string | null
  description?: string | null
  thinking?: Record<string, unknown> | null
  request_override?: ProviderModelSpeedModeRequestOverride | null
  adapter_overrides?: Record<string, ProviderModelSpeedModeRequestOverride> | null
  disabled?: boolean
}

export type ProviderModelSpeedMode = {
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
  default_thinking_mode?: string | null
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
  thinking_modes?: Record<string, ProviderModelThinkingMode>
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

export type ProviderAdapterModelsRequest = {
  provider_id?: string | null
  base_url: string
  protocol_paths?: ProviderProtocolPaths
  api_key?: string | null
  api_key_env?: string | null
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
      ...(String(request.api_key || '').trim() ? { api_key: String(request.api_key).trim() } : {}),
      ...(String(request.api_key_env || '').trim() ? { api_key_env: String(request.api_key_env).trim() } : {}),
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
  apiKey?: string
  apiKeyEnv?: string
  adapterIds: string[]
}): Promise<ProviderAdapterModels[]> {
  const response = await listProviderAdapterModels({
    provider_id: input.providerId?.trim() || undefined,
    base_url: input.baseUrl.trim(),
    protocol_paths: input.protocolPaths,
    api_key: input.apiKey?.trim() || undefined,
    api_key_env: input.apiKeyEnv?.trim() || undefined,
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
