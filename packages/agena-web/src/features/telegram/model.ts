import { integrationTemplate } from '@/features/integrations/model'
import type {
  ConversationKind,
  CreateIntegration,
  IntegrationProvider,
  IntegrationBinding,
  ChannelSessionMode,
  ChannelTrigger,
  ChannelThreadMode,
} from '@/generated/agenaState'

export const TELEGRAM_ID = 'agena.telegram'

export type TelegramConnection = {
  bot_token_env: string
  account: string
  api_base: string
  long_poll_seconds: number
  allow_loopback_http: boolean
}

export type TelegramRoomDraft = {
  title: string
  chatId: string
  kind: ConversationKind
  ownerId: string
  members: string
  useSourcePermissions: boolean
  botId?: string
  sessionMode?: ChannelSessionMode
  trigger?: ChannelTrigger
  threadMode?: ChannelThreadMode
  threadId?: string
  dailyLimit?: number
  totalLimit?: number
  perUserLimit?: number
  expiresDays?: number
  executionSeconds?: number
  pollSeconds?: number
  allowUnlisted?: boolean
}

export function connectionFromSettings(value: unknown): TelegramConnection {
  const source = value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
  return {
    bot_token_env: typeof source.bot_token_env === 'string' ? source.bot_token_env : 'AGENA_TELEGRAM_BOT_TOKEN',
    account: typeof source.account === 'string' ? source.account : '',
    api_base: typeof source.api_base === 'string' ? source.api_base : 'https://api.telegram.org',
    long_poll_seconds: typeof source.long_poll_seconds === 'number' ? source.long_poll_seconds : 0,
    allow_loopback_http: source.allow_loopback_http === true,
  }
}

export function validateTelegramConnection(input: TelegramConnection): TelegramConnection {
  const account = input.account.trim().replace(/^@/, '').toLowerCase()
  if (!/^[A-Za-z0-9_]{1,64}$/.test(account))
    throw new Error('Bot username must contain 1–64 letters, digits or underscores.')
  if (!/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(input.bot_token_env.trim()))
    throw new Error('Provide an environment variable name, not the BotFather token.')
  const api_base = input.api_base.trim().replace(/\/+$/, '')
  let url: URL
  try {
    url = new URL(api_base)
  } catch {
    throw new Error('Enter a valid Telegram Bot API URL.')
  }
  if (
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    !(
      url.protocol === 'https:' ||
      (input.allow_loopback_http &&
        url.protocol === 'http:' &&
        ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname))
    )
  )
    throw new Error('The Bot API must use HTTPS, except explicitly permitted loopback test servers.')
  if (!Number.isSafeInteger(input.long_poll_seconds) || input.long_poll_seconds < 0 || input.long_poll_seconds > 10)
    throw new Error('Long poll duration must be between 0 and 10 seconds.')
  return { ...input, account, bot_token_env: input.bot_token_env.trim(), api_base }
}

/** Build a denied-by-default draft; the host alone grants eventual access.
 * Telegram room and user identifiers are kept as strings to avoid loss of
 * precision when crossing the browser's JSON number representation.
 */
export function telegramBindingRequest(
  provider: IntegrationProvider,
  connection: TelegramConnection,
  draft: TelegramRoomDraft,
  now = Date.now(),
  requestKey = crypto.randomUUID(),
): CreateIntegration {
  if (provider.plugin_id !== TELEGRAM_ID || provider.adapter.kind !== 'channel')
    throw new Error('The selected provider is not the Telegram channel adapter.')
  const bot = validateTelegramConnection(connection)
  if (draft.useSourcePermissions !== true)
    throw new Error('Explicit consent to the source-session permission boundary is required.')
  const chatId = draft.chatId.trim()
  const ownerId = draft.ownerId.trim()
  if (!/^-?[1-9]\d{0,18}$/.test(chatId)) throw new Error('Enter the exact numeric Telegram chat ID.')
  if (!/^[1-9]\d{0,18}$/.test(ownerId)) throw new Error('Enter the Telegram numeric user ID of a channel owner.')
  if (chatId.startsWith('-') && draft.kind === 'direct')
    throw new Error('Direct chats require a positive Telegram chat ID.')
  if (!chatId.startsWith('-') && draft.kind !== 'direct')
    throw new Error('Telegram group and channel IDs must be negative.')
  const title = draft.title.trim() || 'Telegram ' + chatId
  if (title.length > 120) throw new Error('The chat title is too long.')
  const others = draft.members
    .split(/[\n,;]+/)
    .map((item) => item.trim())
    .filter(Boolean)
  if (others.length > 64 || others.some((item) => !/^[1-9]\d{0,18}$/.test(item)))
    throw new Error('Enter at most 64 exact numeric member IDs.')
  const members = [...new Set(others)]
    .filter((id) => id !== ownerId)
    .map((actor) => ({
      actor,
      role: 'member' as const,
      expires_at_ms: null,
    }))
  if (draft.kind === 'broadcast') {
    members.push({ actor: `channel:${chatId}`, role: 'member', expires_at_ms: null })
  }
  const spec = integrationTemplate(provider, now)
  spec.title = title
  spec.provider = TELEGRAM_ID
  spec.mode = 'channel'
  spec.resource = chatId
  spec.config = draft.botId ? { bot_id: draft.botId } : {}
  spec.actors = []
  spec.event_types = ['message']
  const bounded = (value: number | undefined, fallback: number, min: number, max: number, label: string) => {
    const next = value ?? fallback
    if (!Number.isSafeInteger(next) || next < min || next > max) throw new Error(`${label}: ${min}–${max}`)
    return next
  }
  if (draft.kind === 'direct' && draft.threadId?.trim()) throw new Error('Private chat bindings cannot select a topic.')
  if (draft.threadId?.trim() && !/^[1-9]\d{0,15}$/.test(draft.threadId.trim()))
    throw new Error('Use a numeric topic ID.')
  spec.thread = draft.kind !== 'direct' && draft.threadMode === 'exact' ? draft.threadId?.trim() || null : null
  spec.poll_seconds = bounded(
    draft.pollSeconds,
    Math.max(1, provider.adapter.minimum_poll_seconds),
    Math.max(1, provider.adapter.minimum_poll_seconds),
    86400,
    'Polling seconds',
  )
  spec.daily_limit = bounded(draft.dailyLimit, 5, 1, 100, 'Daily quota')
  spec.total_limit = bounded(draft.totalLimit, 50, 1, 10000, 'Total quota')
  spec.max_execution_seconds = bounded(draft.executionSeconds, 300, 30, 3600, 'Execution seconds')
  spec.expires_at_ms = now + bounded(draft.expiresDays, 7, 1, 90, 'Expiry days') * 86400000
  spec.use_source_permissions = true
  spec.channel_policy = {
    ...spec.channel_policy!,
    identity: { tenant: 'telegram', account: bot.account },
    kinds: [draft.kind],
    session_mode: draft.sessionMode || 'per_conversation',
    thread_mode:
      draft.kind === 'direct' ? 'exact' : draft.threadMode || (draft.kind === 'group' ? 'per_thread' : 'exact'),
    trigger: draft.trigger || (draft.kind === 'group' ? 'direct_or_mention' : 'every_message'),
    members: [{ actor: ownerId, role: 'owner', expires_at_ms: null }, ...members],
    allow_unlisted_chat: draft.kind === 'group' && draft.allowUnlisted === true,
    per_user_daily_limit: bounded(draft.perUserLimit, 50, 1, 100, 'Per-member daily quota'),
    allow_automated_reports: false,
    grant: { tools: [], mcp_tools: {}, private_memory: false, source_workspace: false, network_hosts: [] },
  }
  return { request_key: requestKey, spec }
}

export type TelegramBotConfig = {
  id: string
  name: string
  enabled: boolean
  connection: TelegramConnection
  credential_kind: 'environment' | 'vault'
}
export type TelegramVerification = {
  bot_id: string
  username: string
  checked_at_ms: number
  webhook_active: boolean
  pending_updates: number | null
  webhook_warning: string | null
}
export type TelegramBot = TelegramBotConfig & {
  has_token: boolean
  managed: boolean
  updated_at_ms: number
  verification: TelegramVerification | null
  last_poll_ms: number
  last_error: string | null
  room_count: number
  buffered_updates: number
}
export type TelegramCatalog = { revision: number; bots: TelegramBot[]; max_bots: number }
export type TelegramRoom = {
  chat_id: string
  title: string
  username: string | null
  kind: string
  first_seen_ms: number
  last_seen_ms: number
  messages_seen: number
  last_sender_id: string | null
  last_sender_name: string | null
  last_text: string
  description: string | null
  topics: Record<string, string>
  bot_membership: {
    status?: string
    can_post_messages?: boolean
    can_manage_topics?: boolean
    can_delete_messages?: boolean
  } | null
}
export type TelegramInbox = {
  bot_id: string
  rooms: TelegramRoom[]
  recent: Array<{
    sequence: number
    update_id: number
    chat_id: string | null
    sender_id: string | null
    text: string
    received_at_ms: number
  }>
  verification: TelegramVerification | null
  buffered_updates: number
  last_poll_ms: number
  retry_at_ms: number
  last_error: string | null
  retention_days: number
  room_limit: number
  stream_id: string
}
export type TelegramChatRow = {
  id: string
  title: string
  kind: string
  username: string
  lastSeen: number
  room?: TelegramRoom
  bindings: IntegrationBinding[]
  state: string
}
export type TelegramView = 'overview' | 'chats' | 'activity' | 'settings'
export function botConfig(bot: TelegramBot): TelegramBotConfig {
  return {
    id: bot.id,
    name: bot.name,
    enabled: bot.enabled,
    connection: { ...bot.connection },
    credential_kind: bot.credential_kind,
  }
}
export function newBotConfig(): TelegramBotConfig {
  return {
    id: crypto.randomUUID(),
    name: '',
    enabled: true,
    credential_kind: 'vault',
    connection: connectionFromSettings({}),
  }
}
export function matchesTelegramBot(binding: IntegrationBinding, bot: TelegramBotConfig): boolean {
  if (binding.spec.provider !== TELEGRAM_ID) return false
  const identity = binding.spec.channel_policy?.identity
  if (
    !identity ||
    identity.tenant !== 'telegram' ||
    identity.account.toLowerCase() !== bot.connection.account.toLowerCase()
  )
    return false
  const config = binding.spec.config
  const selected =
    config && typeof config === 'object' && !Array.isArray(config) && 'bot_id' in config ? config.bot_id : null
  return typeof selected === 'string' ? selected === bot.id : true
}
export function telegramChatRows(
  bot: TelegramBotConfig,
  inbox: TelegramInbox | null,
  bindings: IntegrationBinding[],
): TelegramChatRow[] {
  const rows = new Map<string, TelegramChatRow>()
  for (const room of inbox?.rooms || [])
    rows.set(room.chat_id, {
      id: room.chat_id,
      title: room.title || room.chat_id,
      kind: room.kind,
      username: room.username || '',
      lastSeen: room.last_seen_ms,
      room,
      bindings: [],
      state: 'unbound',
    })
  for (const binding of bindings.filter((entry) => matchesTelegramBot(entry, bot))) {
    const id = typeof binding.spec.resource === 'string' ? binding.spec.resource : JSON.stringify(binding.spec.resource)
    let row = rows.get(id)
    if (!row) {
      row = {
        id,
        title: binding.spec.title,
        kind: binding.spec.channel_policy?.kinds?.[0] || 'unknown',
        username: '',
        lastSeen: 0,
        bindings: [],
        state: binding.state,
      }
      rows.set(id, row)
    }
    row.bindings.push(binding)
    row.state = row.bindings.some((entry) => entry.state === 'faulted')
      ? 'faulted'
      : row.bindings.some((entry) => entry.state === 'enabled')
        ? 'enabled'
        : binding.state
  }
  return [...rows.values()].sort((a, b) => b.lastSeen - a.lastSeen || a.title.localeCompare(b.title))
}
export function filterTelegramChats(
  rows: TelegramChatRow[],
  query: string,
  kind: string,
  state: string,
): TelegramChatRow[] {
  const needle = query.trim().toLowerCase()
  return rows.filter(
    (row) =>
      (!kind || row.kind === kind) &&
      (!state || row.state === state) &&
      (!needle ||
        [row.title, row.id, row.username, row.room?.last_sender_id, row.room?.last_sender_name]
          .join(' ')
          .toLowerCase()
          .includes(needle)),
  )
}
