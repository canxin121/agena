export type ProviderModelPair = {
  provider?: string
  adapter?: string
  model?: string
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
  if (parts.length === 2) {
    return {
      provider: decodePart(parts[0] || ''),
      adapter: '',
      model: decodePart(parts[1] || ''),
    }
  }
  if (parts.length !== 3) return { provider: '', adapter: '', model: '' }
  return {
    provider: decodePart(parts[0] || ''),
    adapter: decodePart(parts[1] || ''),
    model: decodePart(parts[2] || ''),
  }
}
