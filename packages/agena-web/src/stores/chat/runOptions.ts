import type { JsonObject } from '../../types/json'

export type AgenaModelRef = {
  provider_id: string
  adapter_id?: string
  model_id: string
}

export type RunOptionsPayload = {
  model?: AgenaModelRef
  thinking_mode?: string
  speed_mode?: string
  verbosity?: string
  system?: string
  temperature?: number
  max_output_tokens?: number
}

export function buildRunRequestBody(options?: RunOptionsPayload): JsonObject {
  const source = options || {}
  const body: JsonObject = {}
  const model = source.model
  if (model) {
    const providerId = String(model.provider_id || '').trim()
    const adapterId = String(model.adapter_id || '').trim()
    const modelId = String(model.model_id || '').trim()
    if (providerId && modelId) {
      body.model = {
        provider_id: providerId,
        ...(adapterId ? { adapter_id: adapterId } : {}),
        model_id: modelId,
      }
    }
  }

  for (const key of ['thinking_mode', 'speed_mode', 'verbosity'] as const) {
    const value = source[key]
    if (typeof value === 'string' && value.trim()) body[key] = value.trim()
  }
  // An explicit empty system override disables the inherited addition.
  if (typeof source.system === 'string') body.system = source.system
  if (typeof source.temperature === 'number' && Number.isFinite(source.temperature))
    body.temperature = source.temperature
  if (
    typeof source.max_output_tokens === 'number' &&
    Number.isSafeInteger(source.max_output_tokens) &&
    source.max_output_tokens > 0
  ) {
    body.max_output_tokens = source.max_output_tokens
  }
  return body
}
