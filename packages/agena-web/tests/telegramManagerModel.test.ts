import { describe, expect, test } from 'bun:test'
import {
  connectionFromSettings,
  validateTelegramConnection,
  copyTelegramChatAccess,
  defaultTelegramChatAccess,
  telegramChatAllowed,
} from '../src/features/telegram/model'
const connection = connectionFromSettings({ account: 'My_Agent_Bot' })
describe('Telegram connection and binding ownership', () => {
  test('never treats a raw token as a valid environment variable name', () => {
    expect(() => validateTelegramConnection({ ...connection, bot_token_env: '123:ABC-secret' })).toThrow()
    expect(() => validateTelegramConnection({ ...connection, api_base: 'http://api.telegram.org' })).toThrow()
    expect(() =>
      validateTelegramConnection({ ...connection, api_base: 'https://user:pass@api.telegram.org' }),
    ).toThrow()
    expect(validateTelegramConnection(connection).account).toBe('my_agent_bot')
  })
})

import {
  botConfig,
  matchesTelegramBot,
  telegramChatRows,
  filterTelegramChats,
  type TelegramBot,
  type TelegramInbox,
} from '../src/features/telegram/model'
import type { IntegrationBinding } from '../src/generated/agenaState'
function botView(id: string, account: string): TelegramBot {
  return {
    id,
    name: id,
    enabled: true,
    credential_kind: 'vault',
    connection: connectionFromSettings({ account }),
    ai_bot_id: 'ai-' + id,
    ai_bot: { id: 'ai-' + id, name: 'AI ' + id },
    source_session_id: id === 'work' ? 1 : 2,
    chat_access: defaultTelegramChatAccess(),
    routing_error: null,
    has_token: true,
    managed: true,
    updated_at_ms: 1,
    verification: null,
    last_poll_ms: 0,
    last_error: null,
    room_count: 0,
    buffered_updates: 0,
  }
}
function bindingFor(bot: TelegramBot, chatId = '-1001234567890'): IntegrationBinding {
  return {
    id: 'binding-' + bot.id,
    owner_bot_id: bot.ai_bot_id,
    spec: {
      title: 'Test group',
      provider: 'agena.telegram',
      mode: 'channel',
      resource: chatId,
      config: { bot_id: bot.id },
      actors: [],
      event_types: ['message'],
      instruction: '',
      poll_seconds: 1,
      daily_limit: 0,
      total_limit: 0,
      expires_at_ms: 0,
      max_execution_seconds: 300,
      use_source_permissions: true,
      channel_policy: {
        identity: { tenant: 'telegram', account: bot.connection.account },
        kinds: ['group'],
        members: [],
        session_mode: 'per_conversation',
        thread_mode: 'per_thread',
        trigger: 'direct_or_mention',
        per_user_daily_limit: 0,
      },
    },
    state: 'disabled',
    revision: 1,
    admitted: 0,
    channel_session_id: null,
    created_at_ms: 1,
    cursor: null,
    day: '',
    day_admitted: 0,
    epoch: 1,
    next_poll_ms: 0,
    poll_failures: 0,
    provider_fingerprint: 'p',
    request_key: 'test-' + bot.id + '-' + chatId,
    source_session_id: bot.source_session_id!,
    updated_at_ms: 1,
  }
}
describe('multiple Telegram bots and rich chat management', () => {
  test('a BotFather token can discover a new bot identity without requiring a typed username', () => {
    const pending = connectionFromSettings({})
    expect(validateTelegramConnection(pending, true).account).toBe('')
    expect(() => validateTelegramConnection(pending)).toThrow()
    expect(() => validateTelegramConnection({ ...pending, account: 'invalid name' }, true)).toThrow()
  })
  test('allowlists enable selected rooms only within enabled chat types', () => {
    const access = { ...defaultTelegramChatAccess(), chat_ids: ['42', '-10042', '-10043'] }
    expect(telegramChatAllowed(access, '42', 'direct')).toBe(true)
    expect(telegramChatAllowed(access, '-10042', 'group')).toBe(true)
    expect(telegramChatAllowed(access, '-10043', 'broadcast')).toBe(false)
    expect(telegramChatAllowed(access, '99', 'direct')).toBe(false)
    expect(telegramChatAllowed({ ...access, private_enabled: false }, '42', 'direct')).toBe(false)
  })
  test('blocklists apply to new chats without maintaining an allowlist snapshot', () => {
    const access = { ...defaultTelegramChatAccess(), mode: 'blacklist' as const, chat_ids: ['42', '-10042'] }
    expect(telegramChatAllowed(access, '42', 'direct')).toBe(false)
    expect(telegramChatAllowed(access, '-10042', 'group')).toBe(false)
    expect(telegramChatAllowed(access, '-1009999999999999', 'group')).toBe(true)
    expect(telegramChatAllowed(access, '999999999999999', 'private')).toBe(true)
    expect(telegramChatAllowed(access, '-10043', 'channel')).toBe(false)
    expect(telegramChatAllowed(access, '99', 'unknown')).toBe(false)
  })
  test('editing one bot keeps AI ownership, room selection and saved configuration isolated', () => {
    const work = botView('work', 'work_bot'),
      home = botView('home', 'home_bot')
    work.chat_access.chat_ids = ['42']
    const draft = botConfig(work)
    draft.ai_bot_id = home.ai_bot_id
    draft.chat_access.chat_ids.push('-10042')
    expect(work.ai_bot_id).toBe('ai-work')
    expect(work.chat_access.chat_ids).toEqual(['42'])
    expect(home.chat_access.chat_ids).toEqual([])
    expect(botConfig(work)).not.toHaveProperty('source_session_id')
    expect(copyTelegramChatAccess(null)).toEqual(defaultTelegramChatAccess())
  })
  test('a room binding keeps its stable bot identity after credential or endpoint changes', () => {
    const target = botView('work', 'work_bot')
    const binding = bindingFor(target)
    target.connection.bot_token_env = 'ROTATED_TELEGRAM_TOKEN'
    target.connection.api_base = 'https://telegram.example.test'
    expect(matchesTelegramBot(binding, target)).toBe(true)
    expect(botConfig(target)).not.toHaveProperty('has_token')
  })
  test('the same numeric chat ID in two bots never merges authorization', () => {
    const work = botView('work', 'work_bot'),
      home = botView('home', 'home_bot')
    const workBinding = bindingFor(work),
      homeBinding = bindingFor(home)
    expect(matchesTelegramBot(homeBinding, work)).toBe(false)
    expect(matchesTelegramBot(workBinding, work)).toBe(true)
    const rows = telegramChatRows(work, null, [workBinding, homeBinding])
    expect(rows).toHaveLength(1)
    expect(rows[0]!.bindings.map((b) => b.id)).toEqual(['binding-work'])
    expect(
      matchesTelegramBot({ ...workBinding, spec: { ...workBinding.spec, config: { bot_id: 'home' } } }, work),
    ).toBe(false)
  })
  test('recreated local sources show current chat routes without retired faults or controls', () => {
    const target = botView('work', 'work_bot')
    const previous = { ...bindingFor(target), id: 'old-route', state: 'faulted' as const }
    target.source_session_id = 8
    const current = { ...bindingFor(target), id: 'current-route', state: 'enabled' as const }
    const rows = telegramChatRows(target, null, [previous, current])
    expect(matchesTelegramBot(previous, target)).toBe(false)
    expect(matchesTelegramBot(current, target)).toBe(true)
    expect(rows).toHaveLength(1)
    expect(rows[0]!.state).toBe('enabled')
    expect(rows[0]!.bindings.map((binding) => binding.id)).toEqual(['current-route'])
  })
  test('legacy bindings are identified by verified account instead of leaking into other bot tabs', () => {
    const work = botView('work', 'work_bot'),
      home = botView('home', 'home_bot')
    const binding = bindingFor(work)
    binding.spec.config = {}
    expect(matchesTelegramBot(binding, work)).toBe(true)
    expect(matchesTelegramBot(binding, home)).toBe(false)
  })
  test('discovery remains unbound until explicitly authorized and supports type/status/identity search', () => {
    const work = botView('work', 'work_bot')
    const inbox = {
      rooms: [
        {
          chat_id: '77',
          title: 'Private room',
          kind: 'direct',
          username: 'alice',
          last_seen_ms: 50,
          last_sender_id: '77',
          last_sender_name: 'Alice',
        },
      ],
    } as TelegramInbox
    const rows = telegramChatRows(work, inbox, [bindingFor(work)])
    const found = filterTelegramChats(rows, 'alice', 'direct', 'unbound')
    expect(found).toHaveLength(1)
    expect(found[0]!.bindings).toEqual([])
    expect(filterTelegramChats(rows, 'alice', 'group', '')).toEqual([])
    expect(filterTelegramChats(rows, '', '', 'disabled')).toHaveLength(1)
  })
})
