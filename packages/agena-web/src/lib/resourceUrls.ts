import { isBackendUrl, readActiveBackendBaseUrl } from './backend'

/** Only our own API requires the active server's credentials. External media
 * remains a normal browser URL and never receives an Authorization header. */
export function isAuthenticatedResourceUrl(url: string, baseUrl = readActiveBackendBaseUrl()): boolean {
  if (!isBackendUrl(url, baseUrl)) return false
  try {
    const parsed = new URL(url, baseUrl || 'http://agena.invalid')
    return parsed.pathname.startsWith('/api/')
  } catch {
    return false
  }
}

/** Reject executable/custom schemes before either navigation or media loading. */
export function browserResourceUrl(url: string): string {
  const value = String(url || '').trim()
  if (!value || /[\u0000-\u0020\\]/.test(value)) return ''
  if (value.startsWith('/') && !value.startsWith('//')) return value
  return /^(?:https?:|blob:|data:)/i.test(value) ? value : ''
}

/** Blob navigation loses response CSP: active and unknown types must download. */
export function isPassiveResourceMime(type: string): boolean {
  const mime = type.split(';')[0]?.trim().toLowerCase() || ''
  return (
    /^(?:image\/(?:png|jpeg|gif|webp|avif|bmp|x-icon|vnd\.microsoft\.icon)|audio\/[\w.+-]+|video\/[\w.+-]+)$/.test(
      mime,
    ) || ['application/pdf', 'application/json', 'text/plain'].includes(mime)
  )
}
