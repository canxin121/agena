import type { JsonValue as JsonLike } from '@/types/json'

import { emitAuthRequired } from './authEvents.ts'
import { isBackendUrl, readActiveBackendBaseUrl, resolveBackendUrl } from './backend'
import { buildActiveServerAuthHeaders, readServerAuthTokenVersion } from './serverAuthToken'

export class ApiError extends Error {
  status: number
  bodyText?: string
  bodyJson?: JsonLike
  code?: string
  hint?: string

  constructor(message: string, status: number, bodyText?: string) {
    super(message)
    this.status = status
    this.bodyText = bodyText
  }
}

export function apiUrl(path: string): string {
  return resolveBackendUrl(path, readActiveBackendBaseUrl())
}

function hasHeader(initHeaders: RequestInit['headers'] | undefined, name: string): boolean {
  const needle = String(name || '')
    .trim()
    .toLowerCase()
  if (!needle) return false

  const h = initHeaders
  if (!h) return false

  try {
    if (typeof Headers !== 'undefined' && h instanceof Headers) {
      return h.has(needle)
    }
  } catch {
    // ignore
  }

  if (Array.isArray(h)) {
    for (const pair of h) {
      if (!Array.isArray(pair) || pair.length < 1) continue
      const key = String(pair[0] || '')
        .trim()
        .toLowerCase()
      if (key === needle) return true
    }
    return false
  }

  if (typeof h === 'object') {
    for (const k of Object.keys(h as Record<string, string>)) {
      if (
        String(k || '')
          .trim()
          .toLowerCase() === needle
      )
        return true
    }
  }
  return false
}

function asRecord(value: JsonLike): Record<string, JsonLike> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, JsonLike>
}

function nonEmptyString(value: JsonLike): string {
  return typeof value === 'string' ? value.trim() : ''
}

function apiErrorDetails(bodyJson: JsonLike, fallbackMessage: string): { message: string; code: string; hint: string } {
  const body = asRecord(bodyJson)
  if (!body) return { message: fallbackMessage, code: '', hint: '' }

  const errorValue = body.error
  const errorRecord = asRecord(errorValue)
  const problem = asRecord(body.problem)
  const user = asRecord(problem?.user)

  // Workbench endpoints return { error, code, hint }; resource endpoints use
  // { problem: { code, user: { fallback } } }.
  const message =
    nonEmptyString(errorValue) ||
    nonEmptyString(body.message) ||
    nonEmptyString(errorRecord?.message) ||
    nonEmptyString(user?.fallback) ||
    fallbackMessage
  const code = nonEmptyString(body.code) || nonEmptyString(problem?.code)
  const hint = nonEmptyString(body.hint) || nonEmptyString(problem?.hint)

  return {
    message: hint ? `${message}\n${hint}` : message,
    code,
    hint,
  }
}

async function throwApiResponseError(resp: Response, url: string, authTokenVersion: number): Promise<never> {
  const bodyText = await resp.text().catch(() => '')

  // Decode once for the public message and the endpoint-specific diagnostics.
  let bodyJson: JsonLike = undefined
  const contentType = (resp.headers.get('content-type') || '').toLowerCase()
  const looksJson =
    contentType.includes('application/json') || bodyText.trim().startsWith('{') || bodyText.trim().startsWith('[')
  if (bodyText && looksJson) {
    try {
      bodyJson = JSON.parse(bodyText)
    } catch {
      bodyJson = undefined
    }
  }

  const details = apiErrorDetails(bodyJson, bodyText || `Request failed (${resp.status})`)
  const error = new ApiError(details.message, resp.status, bodyText)
  if (bodyJson !== undefined) error.bodyJson = bodyJson
  if (details.code) error.code = details.code
  if (details.hint) error.hint = details.hint

  if (error.status === 401 && details.code === 'auth.required') {
    emitAuthRequired({
      message: error.message,
      status: error.status,
      code: details.code,
      url,
      authTokenVersion,
    })
  }
  throw error
}

export function apiErrorBodyRecord(error: Error | JsonLike): Record<string, JsonLike> | null {
  if (!(error instanceof ApiError)) return null
  return asRecord(error.bodyJson)
}

/** Authenticated fetch with standard HTTP status semantics and an unread body.
 * Callers that consume streamed bodies can impose a limit before reading them. */
export async function apiFetch(url: string, init?: RequestInit): Promise<Response> {
  const trustedBackend = isBackendUrl(url)
  const authHeaders = trustedBackend ? buildActiveServerAuthHeaders() : {}
  const headers = new Headers(init?.headers)
  if (!headers.has('accept')) headers.set('accept', 'application/json')
  if (authHeaders.authorization && !headers.has('authorization'))
    headers.set('authorization', authHeaders.authorization)
  return await fetch(apiUrl(url), {
    ...init,
    // Token auth works without cookies; keep cookie compatibility unless caller overrides.
    credentials: init?.credentials ?? (!trustedBackend || authHeaders.authorization ? 'omit' : 'include'),
    headers,
  })
}

export async function apiResponse(url: string, init?: RequestInit): Promise<Response> {
  const authTokenVersion = readServerAuthTokenVersion()
  const resp = await apiFetch(url, init)

  if (!resp.ok && resp.status !== 304) {
    await throwApiResponseError(resp, url, authTokenVersion)
  }

  return resp
}

export async function apiJson<T>(url: string, init?: RequestInit): Promise<T> {
  return (await (await apiResponse(url, init)).json()) as T
}

export async function apiText(url: string, init?: RequestInit): Promise<string> {
  const trustedBackend = isBackendUrl(url)
  const authHeaders = trustedBackend ? buildActiveServerAuthHeaders() : {}
  const authTokenVersion = readServerAuthTokenVersion()
  const resp = await fetch(apiUrl(url), {
    ...init,
    credentials: init?.credentials ?? (!trustedBackend || authHeaders.authorization ? 'omit' : 'include'),
    headers: {
      ...(init?.headers ?? {}),
      ...(authHeaders.authorization && !hasHeader(init?.headers, 'authorization') ? authHeaders : {}),
    },
  })
  if (!resp.ok) {
    await throwApiResponseError(resp, url, authTokenVersion)
  }

  return await resp.text()
}

export async function apiBlob(url: string, init?: RequestInit): Promise<Blob> {
  const trustedBackend = isBackendUrl(url)
  const authHeaders = trustedBackend ? buildActiveServerAuthHeaders() : {}
  const authTokenVersion = readServerAuthTokenVersion()
  const resp = await fetch(apiUrl(url), {
    ...init,
    credentials: init?.credentials ?? (!trustedBackend || authHeaders.authorization ? 'omit' : 'include'),
    headers: {
      ...(init?.headers ?? {}),
      ...(authHeaders.authorization && !hasHeader(init?.headers, 'authorization') ? authHeaders : {}),
    },
  })
  if (!resp.ok) {
    await throwApiResponseError(resp, url, authTokenVersion)
  }

  return await resp.blob()
}
