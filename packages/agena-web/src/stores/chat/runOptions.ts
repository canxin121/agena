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
  parallel_tool_calls?: boolean
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
  if (typeof source.parallel_tool_calls === 'boolean') body.parallel_tool_calls = source.parallel_tool_calls
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

export function buildRunOptions(opts: {
  providerID?: string
  adapterID?: string
  modelID?: string
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
}): RunOptionsPayload {
  const providerID = (opts.providerID || '').trim()
  const adapterID = (opts.adapterID || '').trim()
  const modelID = (opts.modelID || '').trim()
  const options: RunOptionsPayload = {}
  if (providerID && modelID) {
    const model: AgenaModelRef = {
      provider_id: providerID,
      ...(adapterID ? { adapter_id: adapterID } : {}),
      model_id: modelID,
    }
    options.model = model
  }
  const thinkingMode = String(opts.thinkingMode || '').trim()
  const speedMode = String(opts.speedMode || '').trim()
  const verbosity = String(opts.verbosity || '').trim()
  if (thinkingMode) options.thinking_mode = thinkingMode
  if (speedMode) options.speed_mode = speedMode
  if (verbosity) options.verbosity = verbosity
  return options
}
