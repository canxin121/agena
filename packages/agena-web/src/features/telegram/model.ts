import type { IntegrationBinding } from '@/generated/agenaState'

export const TELEGRAM_ID = 'agena.telegram'

export type TelegramConnection = {
  bot_token_env: string
  account: string
  api_base: string
  long_poll_seconds: number
  allow_loopback_http: boolean
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

export function validateTelegramConnection(input: TelegramConnection, allowEmptyAccount = false): TelegramConnection {
  const account = input.account.trim().replace(/^@/, '').toLowerCase()
  if (!(allowEmptyAccount && !account) && !/^[A-Za-z0-9_]{1,64}$/.test(account))
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

export type TelegramAiBot = {
  id: string
  name: string
  description?: string
  archived?: boolean
}
export type TelegramChatAccess = {
  mode: 'whitelist' | 'blacklist'
  private_enabled: boolean
  group_enabled: boolean
  channel_enabled: boolean
  chat_ids: string[]
}
export function defaultTelegramChatAccess(): TelegramChatAccess {
  return { mode: 'whitelist', private_enabled: true, group_enabled: true, channel_enabled: false, chat_ids: [] }
}
export function copyTelegramChatAccess(access?: TelegramChatAccess | null): TelegramChatAccess {
  return { ...defaultTelegramChatAccess(), ...access, chat_ids: [...new Set(access?.chat_ids || [])] }
}
export function telegramChatAllowed(access: TelegramChatAccess, id: string, kind: string): boolean {
  const enabled =
    kind === 'direct' || kind === 'private'
      ? access.private_enabled
      : kind === 'group' || kind === 'supergroup'
        ? access.group_enabled
        : kind === 'broadcast' || kind === 'channel'
          ? access.channel_enabled
          : false
  return enabled && (access.mode === 'whitelist' ? access.chat_ids.includes(id) : !access.chat_ids.includes(id))
}
export type TelegramBotConfig = {
  id: string
  name: string
  enabled: boolean
  connection: TelegramConnection
  credential_kind: 'environment' | 'vault'
  ai_bot_id: string | null
  chat_access: TelegramChatAccess
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
  source_session_id: number | null
  ai_bot: TelegramAiBot | null
  routing_error: string | null
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
  source_session_id: number | null
  chat_access: TelegramChatAccess
  routing_error: string | null
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
    ai_bot_id: bot.ai_bot_id || null,
    chat_access: copyTelegramChatAccess(bot.chat_access),
  }
}
export function newBotConfig(): TelegramBotConfig {
  return {
    id: crypto.randomUUID(),
    name: '',
    enabled: true,
    credential_kind: 'vault',
    connection: connectionFromSettings({}),
    ai_bot_id: null,
    chat_access: defaultTelegramChatAccess(),
  }
}
export function matchesTelegramBot(
  binding: IntegrationBinding,
  bot: TelegramBotConfig & { source_session_id?: number | null },
): boolean {
  if (binding.spec.provider !== TELEGRAM_ID) return false
  // Bot-owned integration listings also retain connections from a deleted
  // local source. The manager presents the selected Bot's current routes.
  if (bot.source_session_id != null && binding.source_session_id !== bot.source_session_id) return false
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
