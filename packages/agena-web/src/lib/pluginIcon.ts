/** Keep this in sync with agena_plugin_sdk::normalize_plugin_icon. Icons are
 * image URLs consumed by <img>; the host never evaluates markup or proxies a URL.
 */
export const MAX_PLUGIN_ICON_BYTES = 256 * 1024

const imageMimeTypes = new Set([
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
  'image/avif',
  'image/svg+xml',
  'image/x-icon',
  'image/vnd.microsoft.icon',
])

export function pluginIconSource(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined
  const source = value.trim()
  if (
    !source ||
    source.length > MAX_PLUGIN_ICON_BYTES ||
    new TextEncoder().encode(source).length > MAX_PLUGIN_ICON_BYTES ||
    /[\u0000-\u0020\u007f\\]/.test(source)
  )
    return undefined

  if (source.startsWith('/')) return source.startsWith('//') ? undefined : source

  if (/^https?:\/\//i.test(source)) {
    try {
      const url = new URL(source)
      return url.hostname && !url.username && !url.password ? source : undefined
    } catch {
      return undefined
    }
  }

  if (!/^data:/i.test(source)) return undefined
  const separator = source.indexOf(',')
  if (separator < 0) return undefined
  const [mime, ...parameters] = source.slice(5, separator).toLowerCase().split(';')
  if (!mime || !imageMimeTypes.has(mime)) return undefined
  const payload = source.slice(separator + 1)
  if (!payload) return undefined

  let base64 = false
  let charset = false
  for (const [index, parameter] of parameters.entries()) {
    if (parameter === 'base64' && !base64 && index === parameters.length - 1) base64 = true
    else if (!charset && ['charset=utf-8', 'charset=utf8', 'utf8'].includes(parameter)) charset = true
    else return undefined
  }

  if (base64) {
    if (!/^[A-Za-z0-9+/]+={0,2}$/.test(payload)) return undefined
    if (payload.includes('=') ? payload.length % 4 !== 0 : payload.length % 4 === 1) return undefined
    return source
  }

  return mime === 'image/svg+xml' && !/%(?![\da-f]{2})/i.test(payload) ? source : undefined
}
