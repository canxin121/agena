export type ProviderModelPair = {
  provider?: string
  adapter?: string
  model?: string
}

export type EffectiveModelDefaults = {
  provider: string
  adapter: string
  model: string
  thinkingMode: string
  speedMode: string
  verbosity: string
  parallelToolCalls?: boolean
}

function clean(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function encodePart(value: string): string {
  return encodeURIComponent(clean(value))
}

function decodePart(value: string): string {
  try {
    return decodeURIComponent(value).trim()
  } catch {
    return ''
  }
}

export function encodeModelSelectionKey(selection: ProviderModelPair): string {
  const provider = clean(selection.provider)
  const adapter = clean(selection.adapter)
  const model = clean(selection.model)
  if (!provider || !model) return ''
  return `${encodePart(provider)}/${encodePart(adapter)}/${encodePart(model)}`
}

export function parseModelSlug(slug: string): { provider: string; adapter: string; model: string } {
  const parts = clean(slug).split('/')
  if (parts.length !== 3) return { provider: '', adapter: '', model: '' }
  return {
    provider: decodePart(parts[0] || ''),
    adapter: decodePart(parts[1] || ''),
    model: decodePart(parts[2] || ''),
  }
}

export function resolveEffectiveDefaults(input: {
  runtime?: Partial<EffectiveModelDefaults> | null
  fallback?: ProviderModelPair | null
}): EffectiveModelDefaults {
  const runtime = input.runtime || null
  const fallback = input.fallback || null
  const runtimeProvider = clean(runtime?.provider)
  const runtimeModel = clean(runtime?.model)
  const hasRuntimeModel = Boolean(runtimeProvider && runtimeModel)
  return {
    provider: hasRuntimeModel ? runtimeProvider : clean(fallback?.provider),
    adapter: hasRuntimeModel ? clean(runtime?.adapter) : clean(fallback?.adapter),
    model: hasRuntimeModel ? runtimeModel : clean(fallback?.model),
    thinkingMode: clean(runtime?.thinkingMode),
    speedMode: clean(runtime?.speedMode),
    verbosity: clean(runtime?.verbosity),
    ...(typeof runtime?.parallelToolCalls === 'boolean' ? { parallelToolCalls: runtime.parallelToolCalls } : {}),
  }
}
