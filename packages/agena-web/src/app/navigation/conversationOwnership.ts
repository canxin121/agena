import { readSessionIdFromQuery } from './sessionQuery'

type ConversationRoute = { path: string; query: Record<string, unknown> }

/** A focused composer must agree with every current navigation authority. */
export function matchesConversationRoute(
  route: ConversationRoute,
  path: string,
  sessionId: string | null,
  windowId?: string,
  botId?: string | null,
): boolean {
  if (route.path !== path || (readSessionIdFromQuery(route.query) || null) !== sessionId) return false
  if (path === '/bot' && (!botId || route.query.botId !== botId)) return false
  const targetWindow = typeof route.query.windowId === 'string' ? route.query.windowId.trim() : ''
  return !windowId || !targetWindow || targetWindow === windowId
}
