// ---------------------------------------------------------------------------
// Agena backend resolution — single server at the current origin.
//
// Agena runs one axum server (default bind 127.0.0.1:3210). In dev the vite
// proxy forwards /api, /auth, /health to it; in production the SPA is served
// from the same origin. There is no backend selection, no desktop sidecar, and
// no stored backend list — the base URL is always `window.location.origin`.
// ---------------------------------------------------------------------------

export function currentServerOrigin(): string {
  try {
    if (typeof window !== 'undefined' && window.location && typeof window.location.origin === 'string') {
      const origin = window.location.origin
      // `file://` and some sandboxed contexts return "null".
      if (origin === 'null') return ''
      if (!origin.startsWith('http://') && !origin.startsWith('https://')) return ''
      return origin
    }
  } catch {
    // ignore
  }
  return ''
}

export function resolveBackendUrl(path: string, baseUrl?: string | null): string {
  const rawPath = String(path || '').trim()
  if (!rawPath) return ''

  // Absolute URL passthrough.
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(rawPath)) return rawPath

  const base = String(baseUrl || '').trim()
  if (!base) return rawPath

  const b = base.replace(/\/+$/g, '')
  const p = rawPath.startsWith('/') ? rawPath : `/${rawPath}`
  return `${b}${p}`
}

/** The single Agena server base URL (current origin). */
export function readActiveBackendBaseUrl(): string {
  return currentServerOrigin() || ''
}
