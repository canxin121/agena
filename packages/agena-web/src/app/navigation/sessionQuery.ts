import type { LocationQuery, LocationQueryRaw } from 'vue-router'
import { isConversationSettingsPath } from './mainTabs'

const SESSION_QUERY_KEY = 'sessionId' as const

type ReadQueryLike = LocationQuery | LocationQueryRaw | Record<string, unknown> | null | undefined
type WriteQueryLike = LocationQuery | LocationQueryRaw | null | undefined

function readQueryValue(query: ReadQueryLike, key: string): unknown {
  if (!query || typeof query !== 'object') return undefined
  return (query as Record<string, unknown>)[key]
}

function firstTrimmedString(value: unknown): string {
  if (typeof value === 'string') return value.trim()
  if (Array.isArray(value)) {
    for (const item of value) {
      if (typeof item !== 'string') continue
      const trimmed = item.trim()
      if (trimmed) return trimmed
    }
  }
  return ''
}

export function readSessionIdFromQuery(query: ReadQueryLike): string {
  return firstTrimmedString(readQueryValue(query, SESSION_QUERY_KEY))
}

export function readSessionIdFromFullPath(fullPath: string): string {
  const raw = String(fullPath || '').trim()
  if (!raw) return ''

  const path = raw.split('#', 1)[0] || ''
  const qmark = path.indexOf('?')
  if (qmark < 0) return ''
  const queryPart = path.slice(qmark + 1)
  if (!queryPart) return ''

  const params = new URLSearchParams(queryPart)
  return readSessionIdFromQuery({
    sessionId: params.getAll(SESSION_QUERY_KEY),
  })
}

/** Only conversation routes can update the client's selected conversation. */
export function readConversationSessionId(route: { path: string; fullPath: string; query: ReadQueryLike }): string {
  const path = String(route.path || '')
    .trim()
    .toLowerCase()
  const conversationRoute =
    path === '/' || path === '/chat' || path.startsWith('/chat/') || isConversationSettingsPath(path)
  if (!conversationRoute) return ''
  return readSessionIdFromQuery(route.query) || readSessionIdFromFullPath(route.fullPath)
}

export function patchSessionIdInQuery(query: WriteQueryLike, sessionId: string): LocationQueryRaw {
  const next: LocationQueryRaw = {}
  if (query && typeof query === 'object') {
    for (const [key, value] of Object.entries(query)) {
      next[key] = value as LocationQueryRaw[string]
    }
  }
  const sid = String(sessionId || '').trim()

  if (!sid) {
    delete next[SESSION_QUERY_KEY]
    return next
  }

  next[SESSION_QUERY_KEY] = sid
  return next
}
