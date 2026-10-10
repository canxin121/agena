export type AgenaToolMode = 'provider_protocol' | 'disabled'

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

/** Resolve an inherited mode without writing a user override into the draft. */
export function effectiveProviderToolMode(config: unknown, discoveredToolSupport?: unknown): AgenaToolMode {
  const model = record(config)
  const mode = record(model.agena_tools).mode
  if (mode === 'provider_protocol' || mode === 'disabled') return mode

  const features = model.features
  const declared = record(features)
  const supported = Array.isArray(features) ? features : declared.supported
  if (Array.isArray(supported) && supported.includes('tool_calling')) return 'provider_protocol'
  if (Array.isArray(declared.unsupported) && declared.unsupported.includes('tool_calling')) return 'disabled'

  return discoveredToolSupport === 'unsupported' ? 'disabled' : 'provider_protocol'
}
