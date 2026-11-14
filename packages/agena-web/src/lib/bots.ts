import { apiJson } from './api'
import type { SessionOwnerSurface } from '@/types/chat'
import type {
  BotPersona,
  BotProfile,
  BotList,
  BotSession,
  BotCreate,
  BotUpdate,
  BotArchive,
} from '@/generated/agenaState'
export type {
  BotPersona,
  BotProfile,
  BotList,
  BotSession,
  BotCreate,
  BotUpdate,
  BotArchive,
} from '@/generated/agenaState'

export const emptyBotPersona = (): BotPersona => ({
  instructions: '',
})
export function isBotId(value: unknown): value is string {
  return typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)
}
/** Keep a malformed explicit query distinct from the bodyless default resolver. */
export function botIdQuery(query: Record<string, unknown>): string {
  return Object.prototype.hasOwnProperty.call(query, 'botId')
    ? (typeof query.botId === 'string' ? query.botId : '!invalid') || '!invalid'
    : ''
}
export function botSessionIdQuery(query: Record<string, unknown>): string {
  if (!Object.prototype.hasOwnProperty.call(query, 'sessionId')) return ''
  const value = query.sessionId
  return typeof value === 'string' && /^\d+$/.test(value) && Number.isSafeInteger(Number(value)) && Number(value) > 0
    ? value
    : '!invalid'
}
function botPath(id: string) {
  if (!isBotId(id)) throw new Error('Invalid Bot ID')
  return `/api/v1/bots/${encodeURIComponent(id)}`
}
function jsonBody(method: string, value: BotCreate | BotUpdate | BotArchive | { title?: string }): RequestInit {
  return { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(value) }
}
export const listBots = (signal?: AbortSignal) => apiJson<BotList>('/api/v1/bots', { signal })
export const getBot = (id: string, signal?: AbortSignal) => apiJson<BotProfile>(botPath(id), { signal })
export const createBot = (value: BotCreate) => apiJson<BotProfile>('/api/v1/bots', jsonBody('POST', value))
export const updateBot = (id: string, value: BotUpdate) => apiJson<BotProfile>(botPath(id), jsonBody('PUT', value))
export const archiveBot = (id: string, value: BotArchive) =>
  apiJson<BotProfile>(`${botPath(id)}/archive`, jsonBody('POST', value))

/** Open the Bot's canonical local conversation; the title applies on first creation. */
export const createBotConversation = (id: string, title?: string) => {
  const body = title && title.trim() ? { title: title.trim() } : {}
  return apiJson<BotSession>(`${botPath(id)}/sessions`, jsonBody('POST', body))
}

export async function resolveBotSession(id?: string, signal?: AbortSignal): Promise<BotSession> {
  const response = await apiJson<BotSession>(id ? `${botPath(id)}/session` : '/api/v1/bot/session', {
    method: 'POST',
    signal,
  })
  if (
    !Number.isSafeInteger(response.session_id) ||
    response.session_id <= 0 ||
    !isBotId(response.bot_id) ||
    typeof response.name !== 'string' ||
    !response.name.trim() ||
    (id && response.bot_id.toLowerCase() !== id.toLowerCase())
  ) {
    throw new Error('Invalid Bot session response')
  }
  return response
}

export type ResolvedBotConversation = BotSession & {
  profile: BotProfile
  surface: SessionOwnerSurface
  localHistory: boolean
}

/** Explicit conversation links retain their identity and only perform reads. */
export async function resolveBotConversation(
  id?: string,
  sessionId?: string,
  signal?: AbortSignal,
): Promise<ResolvedBotConversation> {
  if (!sessionId) {
    const resolved = await resolveBotSession(id, signal)
    const profile = await getBot(resolved.bot_id, signal)
    if (profile.archived) throw new Error('Bot is archived')
    return { ...resolved, profile, surface: 'local', localHistory: false }
  }
  if (botSessionIdQuery({ sessionId }) === '!invalid') throw new Error('Invalid Bot session ID')
  if (!id || !isBotId(id)) throw new Error('Invalid Bot ID')
  const [profile, session] = await Promise.all([
    getBot(id, signal),
    apiJson<{ id: number; owner_bot_id?: string | null; owner_surface?: SessionOwnerSurface | null }>(
      `/api/v1/sessions/${encodeURIComponent(sessionId)}`,
      { signal },
    ),
  ])
  if (
    session.id !== Number(sessionId) ||
    session.owner_bot_id?.toLowerCase() !== profile.id.toLowerCase() ||
    !session.owner_surface ||
    !['local', 'direct', 'group', 'broadcast'].includes(session.owner_surface)
  )
    throw new Error('Invalid Bot session response')
  if (profile.archived) throw new Error('Bot is archived')
  return {
    bot_id: profile.id,
    session_id: session.id,
    name: profile.name,
    profile,
    surface: session.owner_surface,
    localHistory: session.owner_surface === 'local' && session.id !== profile.local_session_id,
  }
}
