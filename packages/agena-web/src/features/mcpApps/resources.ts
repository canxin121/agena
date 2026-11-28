export function record(value: unknown): value is Record<string, unknown> {
  return (
    !!value && typeof value === 'object' && !Array.isArray(value) && Object.getPrototypeOf(value) === Object.prototype
  )
}

export function mcpAppResourceUri(value: unknown) {
  if (typeof value !== 'string' || value.length > 2048 || /[\u0000-\u0020\u007f]/.test(value)) return null
  try {
    return new URL(value)
  } catch {
    return null
  }
}

export function mcpAppResourceContents(result: unknown, uri: string) {
  if (!record(result) || !Array.isArray(result.contents)) return null
  const resources = result.contents.filter((item) => record(item) && item.uri === uri)
  return resources.length === 1 ? resources[0] : null
}
