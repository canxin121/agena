import { shallowRef } from 'vue'

import { getSession, listSessions } from '@/stores/chat/api'
import type { Session, SessionOwnerSurface } from '@/types/chat'

export const BOT_SURFACE_ORDER: SessionOwnerSurface[] = ['local', 'direct', 'group', 'broadcast']
const PAGE_SIZE = 50

export type BotConversation = {
  id: string
  title: string
  surface: SessionOwnerSurface
  favorite: boolean
  pinned: boolean
  state?: string
  running: boolean
  localHistory: boolean
  updatedAtMs: number
  lastMessageAtMs: number
  messageCount: number
}

export type BotConversationGroup = {
  surface: SessionOwnerSurface
  conversations: BotConversation[]
  history: BotConversation[]
  total: number
  loaded: boolean
  loading: boolean
  error: string
  hasMore: boolean
  nextCursor: string | null
}

function emptyGroup(surface: SessionOwnerSurface): BotConversationGroup {
  return {
    surface,
    conversations: [],
    history: [],
    total: 0,
    loaded: false,
    loading: false,
    error: '',
    hasMore: false,
    nextCursor: null,
  }
}

function updatedMs(value: unknown): number {
  if (typeof value !== 'string') return 0
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : 0
}

export function botConversationFromSession(
  session: Session,
  botId: string,
  surface: SessionOwnerSurface,
  localSessionId: number | null,
): BotConversation | null {
  if (!session.id || session.owner_bot_id?.toLowerCase() !== botId.toLowerCase() || session.owner_surface !== surface)
    return null
  const state = session.state?.kind
  return {
    id: session.id,
    title: session.title?.trim() || `#${session.id}`,
    surface,
    favorite: session.favorite === true,
    pinned: session.pinned === true,
    state,
    running: state === 'running' || state === 'awaiting_interaction',
    localHistory: surface === 'local' && session.id !== String(localSessionId),
    updatedAtMs: updatedMs(session.updated_at),
    lastMessageAtMs: updatedMs(session.last_message_at),
    messageCount: typeof session.message_count === 'number' ? session.message_count : 0,
  }
}

/** Independent, paged lists let several expanded Bots stay visible together. */
export function useBotConversations() {
  const lists = shallowRef<Record<string, BotConversationGroup[]>>({})
  const localSessionIds = new Map<string, number | null>()
  const requests = new Map<string, AbortController>()

  function groups(botId: string): BotConversationGroup[] {
    return lists.value[botId] ?? BOT_SURFACE_ORDER.map(emptyGroup)
  }

  function update(botId: string, surface: SessionOwnerSurface, patch: Partial<BotConversationGroup>) {
    lists.value = {
      ...lists.value,
      [botId]: groups(botId).map((group) => (group.surface === surface ? { ...group, ...patch } : group)),
    }
  }

  function reset(botId?: string) {
    for (const [key, request] of requests) {
      if (!botId || key.startsWith(`${botId}:`)) {
        request.abort()
        requests.delete(key)
      }
    }
    if (!botId) {
      localSessionIds.clear()
      lists.value = {}
    } else {
      localSessionIds.delete(botId)
      const next = { ...lists.value }
      delete next[botId]
      lists.value = next
    }
  }

  async function loadGroup(botId: string, surface: SessionOwnerSurface, force = false, more = false) {
    const id = botId.trim()
    if (!id) return
    const key = `${id}:${surface}`
    const previous = groups(id).find((group) => group.surface === surface)!
    if (more ? previous.loading || !previous.hasMore : !force && (previous.loaded || previous.loading)) return
    const cursor = more ? previous.nextCursor : null
    if (more && !cursor) return

    requests.get(key)?.abort()
    const request = new AbortController()
    requests.set(key, request)
    update(id, surface, { loading: true, error: '' })
    const localSessionId = localSessionIds.get(id) ?? null
    try {
      const page = await listSessions({
        ownerBotId: id,
        ownerSurface: surface,
        limit: PAGE_SIZE,
        cursor: cursor || undefined,
        excludeSubagents: true,
        includeTotal: true,
        force,
        signal: request.signal,
      })
      if (requests.get(key) !== request) return
      const rows = new Map<string, BotConversation>()
      if (more) {
        for (const conversation of [...previous.conversations, ...previous.history])
          rows.set(conversation.id, conversation)
      }
      for (const session of page.sessions) {
        const conversation = botConversationFromSession(session, id, surface, localSessionId)
        if (conversation) rows.set(conversation.id, conversation)
      }
      // The canonical local conversation can be older than the first history
      // page. Read it directly instead of choosing a recent historical session.
      if (surface === 'local' && localSessionId && !rows.has(String(localSessionId))) {
        const session = await getSession(String(localSessionId), request.signal)
        if (requests.get(key) !== request) return
        const conversation = botConversationFromSession(session, id, surface, localSessionId)
        if (conversation) rows.set(conversation.id, conversation)
      }
      const sorted = [...rows.values()].sort(
        (a, b) => b.lastMessageAtMs - a.lastMessageAtMs || b.updatedAtMs - a.updatedAtMs || a.id.localeCompare(b.id),
      )
      update(id, surface, {
        conversations: sorted.filter((conversation) => !conversation.localHistory),
        history: sorted.filter((conversation) => conversation.localHistory),
        total: Math.max(sorted.length, page.total ?? 0),
        hasMore: page.hasMore,
        nextCursor: page.nextCursor ?? null,
        loaded: true,
      })
    } catch (cause) {
      if (requests.get(key) !== request || request.signal.aborted) return
      update(id, surface, { error: cause instanceof Error ? cause.message : String(cause) })
    } finally {
      if (requests.get(key) === request) {
        requests.delete(key)
        update(id, surface, { loading: false })
      }
    }
  }

  async function load(botId: string, localSessionId?: number | null, force = false) {
    const id = botId.trim()
    if (!id) return
    const canonical = Number.isSafeInteger(localSessionId) && Number(localSessionId) > 0 ? Number(localSessionId) : null
    const changed = localSessionIds.has(id) && localSessionIds.get(id) !== canonical
    localSessionIds.set(id, canonical)
    await Promise.all(
      BOT_SURFACE_ORDER.map((surface) => loadGroup(id, surface, force || (surface === 'local' && changed))),
    )
  }

  return {
    groups,
    load,
    reloadGroup: (botId: string, surface: SessionOwnerSurface) => loadGroup(botId, surface, true),
    loadMore: (botId: string, surface: SessionOwnerSurface) => loadGroup(botId, surface, false, true),
    reset,
  }
}
