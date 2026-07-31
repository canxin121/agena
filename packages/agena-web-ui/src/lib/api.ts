import type { JsonValue as JsonLike } from '@/types/json'

import { emitAuthRequired } from './authEvents.ts'
import { readActiveBackendBaseUrl, resolveBackendUrl } from './backend'
import { buildActiveUiAuthHeaders } from './uiAuthToken'

export class ApiError extends Error {
  status: number
  bodyText?: string
  bodyJson?: JsonLike
  problem?: ApiFailure

  constructor(message: string, status: number, problem?: ApiFailure, bodyText?: string) {
    super(message)
    this.status = status
    this.problem = problem
    this.bodyText = bodyText
  }
}

export interface ApiFailureUser {
  key: string
  fallback: string
  args?: Record<string, JsonLike>
  detail_key?: string | null
}

export interface ApiFailure {
  id: string
  code: string
  category: string
  responsibility: string
  retry: string
  recovery: string
  impact: string
  user: ApiFailureUser
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

async function readBodyText(resp: Response): Promise<string> {
  return await resp.text().catch(() => '')
}

function asRecord(value: JsonLike): Record<string, JsonLike> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, JsonLike>
}

export function apiErrorBodyRecord(error: Error | JsonLike): Record<string, JsonLike> | null {
  if (!(error instanceof ApiError)) return null
  return asRecord(error.bodyJson)
}

function parseFailure(value: JsonLike): ApiFailure | undefined {
  const envelope = asRecord(value)
  const problem = asRecord(envelope?.problem)
  const user = asRecord(problem?.user)
  if (
    !problem ||
    !user ||
    typeof problem.id !== 'string' ||
    typeof problem.code !== 'string' ||
    typeof problem.category !== 'string' ||
    typeof problem.responsibility !== 'string' ||
    typeof problem.retry !== 'string' ||
    typeof problem.recovery !== 'string' ||
    typeof problem.impact !== 'string' ||
    typeof user.key !== 'string' ||
    typeof user.fallback !== 'string'
  ) {
    return undefined
  }
  return problem as unknown as ApiFailure
}

function parseJsonBody(text: string, contentType: string): JsonLike {
  const looksJson =
    contentType.toLowerCase().includes('application/json') ||
    text.trim().startsWith('{') ||
    text.trim().startsWith('[')
  if (!text || !looksJson) return undefined
  try {
    return JSON.parse(text) as JsonLike
  } catch {
    return undefined
  }
}

function responseError(resp: Response, text: string, url: string): ApiError {
  const bodyJson = parseJsonBody(text, resp.headers.get('content-type') || '')
  const problem = parseFailure(bodyJson)
  const fallback = problem?.user.fallback.trim() || `Request failed (${resp.status})`
  const message =
    problem && (problem.category === 'internal' || problem.category === 'data_corruption')
      ? `${fallback} Reference: ${problem.id}`
      : fallback
  const error = new ApiError(message, resp.status, problem, text)
  error.bodyJson = bodyJson

  if (resp.status === 401) {
    emitAuthRequired({ message, status: resp.status, code: problem?.code || 'auth_required', url })
  }
  return error
}

export async function apiResponseError(resp: Response, url: string): Promise<ApiError> {
  return responseError(resp, await readBodyText(resp), url)
}

function transportError(url: string, diagnostic: unknown): ApiError {
  console.error('API transport failed', { url, diagnostic })
  return new ApiError('The service could not be reached. Check the connection and try again.', 0)
}

function invalidSuccessResponse(url: string, diagnostic: unknown): ApiError {
  console.error('API returned an invalid success response', { url, diagnostic })
  return new ApiError('The service returned an invalid response. Try again.', 0)
}

/**
 * Safe display projection for catches that may also receive browser/library
 * exceptions. Only ApiError carries server-approved prose; unknown errors use
 * the caller's fixed fallback and remain diagnostic-only.
 */
export function userErrorMessage(
  error: unknown,
  fallback = 'The operation could not be completed. Try again.',
): string {
  if (error instanceof ApiError && error.message.trim()) return error.message.trim()
  console.error('Unexpected UI operation failure', { diagnostic: error })
  return fallback
}

export async function apiJson<T>(url: string, init?: RequestInit): Promise<T> {
  const authHeaders = buildActiveUiAuthHeaders()
  const resp = await fetch(apiUrl(url), {
    ...init,
    // Token auth works without cookies; keep cookie compatibility unless caller overrides.
    credentials: init?.credentials ?? (authHeaders.authorization ? 'omit' : 'include'),
    headers: {
      ...(init?.headers ?? {}),
      accept: 'application/json',
      ...(authHeaders.authorization && !hasHeader(init?.headers, 'authorization') ? authHeaders : {}),
    },
  }).catch((error) => {
    throw transportError(url, error)
  })

  if (!resp.ok) {
    const txt = await readBodyText(resp)
    throw responseError(resp, txt, url)
  }

  try {
    return (await resp.json()) as T
  } catch (error) {
    throw invalidSuccessResponse(url, error)
  }
}

export async function apiText(url: string, init?: RequestInit): Promise<string> {
  const authHeaders = buildActiveUiAuthHeaders()
  const resp = await fetch(apiUrl(url), {
    ...init,
    credentials: init?.credentials ?? (authHeaders.authorization ? 'omit' : 'include'),
    headers: {
      ...(init?.headers ?? {}),
      ...(authHeaders.authorization && !hasHeader(init?.headers, 'authorization') ? authHeaders : {}),
    },
  }).catch((error) => {
    throw transportError(url, error)
  })
  if (!resp.ok) {
    const txt = await readBodyText(resp)
    throw responseError(resp, txt, url)
  }
  return await resp.text()
}

export async function apiBlob(url: string, init?: RequestInit): Promise<Blob> {
  const authHeaders = buildActiveUiAuthHeaders()
  const resp = await fetch(apiUrl(url), {
    ...init,
    credentials: init?.credentials ?? (authHeaders.authorization ? 'omit' : 'include'),
    headers: {
      ...(init?.headers ?? {}),
      ...(authHeaders.authorization && !hasHeader(init?.headers, 'authorization') ? authHeaders : {}),
    },
  }).catch((error) => {
    throw transportError(url, error)
  })
  if (!resp.ok) {
    const txt = await readBodyText(resp)
    throw responseError(resp, txt, url)
  }

  return await resp.blob()
}
