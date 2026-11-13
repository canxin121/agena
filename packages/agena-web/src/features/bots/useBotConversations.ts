import { computed, ref, shallowRef } from 'vue'

import { listSessions, type AgenaSession } from '@/stores/chat/api'
import type { SessionOwnerSurface } from '@/types/chat'

/**
 * Surface groups a Bot's conversations are listed under, in the order the
 * sidebar shows them.
 *
 * These are the four entrance shapes the server records on
 * `owner_surface`; they are not user-defined folders, because the shape is a
 * property of the room a message arrived in.
 */
export const BOT_SURFACE_ORDER: SessionOwnerSurface[] = ['local', 'direct', 'group', 'broadcast']

export type BotConversation = {
  id: string
  title: string
  surface: SessionOwnerSurface
  favorite: boolean
  pinned: boolean
  state?: string
  updatedAtMs: number
  lastMessageAtMs: number
  messageCount: number
}

export type BotConversationGroup = {
  surface: SessionOwnerSurface
  conversations: BotConversation[]
}

function surfaceOf(value: unknown): SessionOwnerSurface | null {
  return value === 'local' || value === 'direct' || value === 'group' || value === 'broadcast'
    ? value
    : null
}

function updatedMs(value: unknown): number {
  if (typeof value !== 'string') return 0
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : 0
}

function toConversation(raw: AgenaSession, fallback: SessionOwnerSurface): BotConversation | null {
  const id = typeof raw.id === 'number' ? String(raw.id) : ''
  if (!id) return null
  const surface = surfaceOf(raw.owner_surface) ?? fallback
  return {
    id,
    title: typeof raw.title === 'string' && raw.title.trim() ? raw.title : `#${id}`,
    surface,
    favorite: raw.favorite === true,
    pinned: raw.pinned === true,
    state: typeof raw.state === 'string' ? raw.state : undefined,
    updatedAtMs: updatedMs(raw.updated_at),
    lastMessageAtMs: updatedMs(raw.last_message_at),
    messageCount: typeof raw.message_count === 'number' ? raw.message_count : 0,
  }
}

/**
 * One Bot's conversations, fetched from the owner-filtered session list.
 *
 * The project sidebar keeps its own store because its lists are workspace
 * scoped. A Bot's conversations are not: they live in the instance scope and
 * are identified by `owner_bot_id`, so they get their own small cache keyed by
 * Bot. A request is abandoned when the Bot changes, matching the sidebar's
 * "never show another identity's rows" rule.
 */
export function useBotConversations() {
  const groups = shallowRef<BotConversationGroup[]>([])
  const loading = ref(false)
  const error = ref('')
  let current = ''
  let sequence = 0
  let controller: AbortController | null = null

  const total = computed(() => groups.value.reduce((sum, group) => sum + group.conversations.length, 0))

  function reset() {
    sequence++
    controller?.abort()
    controller = null
    groups.value = []
    loading.value = false
    error.value = ''
    current = ''
  }

  async function load(botId: string, force = false) {
    const id = (botId || '').trim()
    if (!id) {
      reset()
      return
    }
    if (!force && id === current && (loading.value || groups.value.length)) return
    sequence++
    controller?.abort()
    const abort = new AbortController()
    controller = abort
    const mine = sequence
    current = id
    loading.value = true
    error.value = ''
    try {
      const page = await listSessions({
        ownerBotId: id,
        // A Bot has one conversation per room; the page bound keeps a busy
        // account from turning the sidebar request into an unbounded read.
        limit: 200,
        excludeSubagents: true,
        force,
        signal: abort.signal,
      })
      if (mine !== sequence) return
      const buckets = new Map<SessionOwnerSurface, BotConversation[]>(
        BOT_SURFACE_ORDER.map((surface) => [surface, [] as BotConversation[]]),
      )
      for (const raw of page.sessions as unknown as AgenaSession[]) {
        const conversation = toConversation(raw, 'local')
        if (!conversation) continue
        buckets.get(conversation.surface)?.push(conversation)
      }
      groups.value = BOT_SURFACE_ORDER.map((surface) => ({
        surface,
        conversations: (buckets.get(surface) ?? []).sort(
          (a, b) => b.lastMessageAtMs - a.lastMessageAtMs || b.updatedAtMs - a.updatedAtMs,
        ),
      }))
    } catch (cause) {
      if (mine !== sequence || abort.signal.aborted) return
      error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      if (mine === sequence) loading.value = false
    }
  }

  return { groups, loading, error, total, load, reset }
}
