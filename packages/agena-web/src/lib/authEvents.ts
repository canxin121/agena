import { sessionStorageKeys } from './persistence/storageKeys'

export const OC_AUTH_REQUIRED_EVENT = 'agena.auth-required'

export const OC_AUTH_REQUIRED_STORAGE_KEY = sessionStorageKeys.auth.authRequired

export type AuthRequiredDetail = {
  message?: string
  status?: number
  code?: string
  url?: string
  /** Version of the UI token used by the request that received 401. */
  authTokenVersion?: number
}

export type StoredAuthRequired = {
  at: number
  detail: AuthRequiredDetail
}

export function extractAuthRequiredMessageFromBodyText(bodyText: string): string {
  const txt = String(bodyText || '').trim()
  if (!txt) return ''

  try {
    const parsed = JSON.parse(txt) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return ''
    const problem = (parsed as Record<string, unknown>).problem
    if (!problem || typeof problem !== 'object' || Array.isArray(problem)) return ''
    const user = (problem as Record<string, unknown>).user
    if (!user || typeof user !== 'object' || Array.isArray(user)) return ''
    const fallback = (user as Record<string, unknown>).fallback
    return typeof fallback === 'string' ? fallback.trim() : ''
  } catch {
    return ''
  }
}

export function readAuthRequiredFromStorage(): StoredAuthRequired | null {
  if (typeof sessionStorage === 'undefined') return null
  try {
    const raw = sessionStorage.getItem(OC_AUTH_REQUIRED_STORAGE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
    const record = parsed as Record<string, unknown>
    const at = typeof record.at === 'number' && Number.isFinite(record.at) ? Math.floor(record.at) : 0
    const detail = record.detail
    if (!detail || typeof detail !== 'object' || Array.isArray(detail)) return null
    return { at, detail: detail as AuthRequiredDetail }
  } catch {
    return null
  }
}

export function clearAuthRequiredFromStorage() {
  if (typeof sessionStorage === 'undefined') return
  try {
    sessionStorage.removeItem(OC_AUTH_REQUIRED_STORAGE_KEY)
  } catch {
    // ignore
  }
}

function persistAuthRequired(detail: AuthRequiredDetail) {
  if (typeof sessionStorage === 'undefined') return
  try {
    const payload: StoredAuthRequired = { at: Date.now(), detail }
    sessionStorage.setItem(OC_AUTH_REQUIRED_STORAGE_KEY, JSON.stringify(payload))
  } catch {
    // ignore
  }
}

function dispatchAuthRequired(target: EventTarget, detail: AuthRequiredDetail) {
  try {
    // Prefer CustomEvent so listeners can read `evt.detail`.
    if (typeof CustomEvent === 'function') {
      target.dispatchEvent(new CustomEvent<AuthRequiredDetail>(OC_AUTH_REQUIRED_EVENT, { detail }))
      return
    }
  } catch {
    // ignore
  }

  try {
    // Fallback for environments where CustomEvent isn't available.
    const evt = new Event(OC_AUTH_REQUIRED_EVENT)
    ;(evt as unknown as { detail?: AuthRequiredDetail }).detail = detail
    target.dispatchEvent(evt)
  } catch {
    // ignore
  }
}

export function emitAuthRequired(detail: AuthRequiredDetail) {
  persistAuthRequired(detail)

  if (typeof window !== 'undefined' && typeof window.dispatchEvent === 'function') {
    dispatchAuthRequired(window, detail)
  }
  if (typeof document !== 'undefined' && typeof document.dispatchEvent === 'function') {
    dispatchAuthRequired(document, detail)
  }
}
