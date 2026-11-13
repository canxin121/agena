import { apiJson } from './api'
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
/**
 * Start a new conversation for this Bot.
 *
 * Distinct from `resolveBotSession`, which reopens the latest one: this always
 * creates an empty conversation, so a Bot can hold several at once.
 */
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
