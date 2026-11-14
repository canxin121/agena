import { describe, expect, test } from 'bun:test'
import {
  connectionFromSettings,
  telegramBindingRequest,
  validateTelegramConnection,
  type TelegramRoomDraft,
} from '../src/features/telegram/model'
import type { IntegrationProvider } from '../src/generated/agenaState'

const provider: IntegrationProvider = {
  plugin_id: 'agena.telegram',
  fingerprint: 'f1',
  adapter: {
    kind: 'channel',
    title: 'Telegram',
    description: '',
    minimum_poll_seconds: 1,
    channel_context_version: 1,
    config_schema: { properties: {} },
    idempotent_send: false,
  },
}
const connection = connectionFromSettings({ account: 'My_Agent_Bot' })
const room: TelegramRoomDraft = {
  title: 'Test group',
  chatId: '-1001234567890',
  kind: 'group',
  ownerId: '987654321',
  members: '12345, 56789\n12345',
  useSourcePermissions: true,
}
describe('Telegram connection and binding ownership', () => {
  test('never treats a raw token as a valid environment variable name', () => {
    expect(() => validateTelegramConnection({ ...connection, bot_token_env: '123:ABC-secret' })).toThrow()
    expect(() => validateTelegramConnection({ ...connection, api_base: 'http://api.telegram.org' })).toThrow()
    expect(() =>
      validateTelegramConnection({ ...connection, api_base: 'https://user:pass@api.telegram.org' }),
    ).toThrow()
    expect(validateTelegramConnection(connection).account).toBe('my_agent_bot')
  })
  test('channel creation requires affirmative permission-boundary consent', () => {
    expect(() => telegramBindingRequest(provider, connection, { ...room, useSourcePermissions: false })).toThrow(
      'Explicit consent',
    )
  })
  test('a group always has explicitly authorized members and no inherited privileges', () => {
    const binding = telegramBindingRequest(provider, connection, room, 1000, 'fixed-request')
    expect(binding.request_key).toBe('fixed-request')
    expect(binding.spec.provider).toBe('agena.telegram')
    expect(binding.spec.mode).toBe('channel')
    expect(binding.spec.resource).toBe('-1001234567890')
    expect(binding.spec.actors).toEqual([])
    expect(binding.spec.use_source_permissions).toBe(true)
    expect(binding.spec.config).toEqual({})
    expect(binding.spec.channel_policy?.identity).toEqual({ tenant: 'telegram', account: 'my_agent_bot' })
    expect(binding.spec.channel_policy?.trigger).toBe('direct_or_mention')
    expect(binding.spec.channel_policy?.thread_mode).toBe('per_thread')
    expect(binding.spec.channel_policy?.members).toEqual([
      { actor: '987654321', role: 'owner', expires_at_ms: null },
      { actor: '12345', role: 'member', expires_at_ms: null },
      { actor: '56789', role: 'member', expires_at_ms: null },
    ])
    expect(binding.spec.channel_policy?.grant?.tools).toEqual([])
    expect(binding.spec.channel_policy?.grant?.private_memory).toBe(false)
    expect(binding.spec.channel_policy?.allow_unlisted_chat).toBe(false)
    expect(binding.spec.expires_at_ms).toBe(1000 + 7 * 86400000)
  })
  test('broadcast messages are attributed to the exact channel actor, not all channel members', () => {
    const binding = telegramBindingRequest(provider, connection, { ...room, kind: 'broadcast', members: '' }, 1000, 'b')
    expect(binding.spec.channel_policy?.members.map((member) => member.actor)).toEqual([
      '987654321',
      'channel:-1001234567890',
    ])
    expect(binding.spec.channel_policy?.trigger).toBe('every_message')
    expect(binding.spec.channel_policy?.thread_mode).toBe('exact')
  })
  test('invalid chat identifiers, member IDs, and provider identities fail before API calls', () => {
    for (const chatId of ['@group', '0', '1e9', ' -100 1 '])
      expect(() => telegramBindingRequest(provider, connection, { ...room, chatId })).toThrow()
    expect(() => telegramBindingRequest(provider, connection, { ...room, ownerId: '@boss' })).toThrow()
    expect(() => telegramBindingRequest(provider, connection, { ...room, members: '1, hacker' })).toThrow()
    expect(() => telegramBindingRequest(provider, connection, { ...room, kind: 'direct' })).toThrow()
    expect(() => telegramBindingRequest({ ...provider, plugin_id: 'other' }, connection, room)).toThrow()
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
  const request = telegramBindingRequest(provider, bot.connection, { ...room, chatId, botId: bot.id })
  return {
    id: 'binding-' + bot.id,
    spec: request.spec,
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
    request_key: request.request_key,
    source_session_id: 1,
    updated_at_ms: 1,
  }
}
describe('multiple Telegram bots and rich chat management', () => {
  test('a room binding selects a stable bot ID and never imports its credential or endpoint', () => {
    const target = botView('work', 'work_bot')
    const binding = bindingFor(target)
    expect(binding.spec.config).toEqual({ bot_id: 'work' })
    expect(JSON.stringify(binding.spec)).not.toContain('bot_token_env')
    expect(JSON.stringify(binding.spec)).not.toContain('api.telegram.org')
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
  test('legacy bindings are identified by verified account instead of leaking into other bot tabs', () => {
    const work = botView('work', 'work_bot'),
      home = botView('home', 'home_bot')
    const binding = bindingFor(work)
    binding.spec.config = {}
    expect(matchesTelegramBot(binding, work)).toBe(true)
    expect(matchesTelegramBot(binding, home)).toBe(false)
  })
  test('advanced grants enforce quota, expiry and topic bounds before submission', () => {
    const request = telegramBindingRequest(
      provider,
      connection,
      {
        ...room,
        botId: 'work',
        dailyLimit: 20,
        totalLimit: 1000,
        expiresDays: 30,
        perUserLimit: 15,
        threadMode: 'exact',
        threadId: '77',
        sessionMode: 'per_actor',
      },
      1000,
      'r',
    )
    expect(request.spec.thread).toBe('77')
    expect(request.spec.channel_policy?.session_mode).toBe('per_actor')
    expect(request.spec.expires_at_ms).toBe(1000 + 30 * 86400000)
    expect(request.spec.daily_limit).toBe(20)
    expect(request.spec.channel_policy?.per_user_daily_limit).toBe(15)
    for (const patch of [
      { dailyLimit: 0 },
      { totalLimit: 10001 },
      { expiresDays: 91 },
      { executionSeconds: 15 },
      { pollSeconds: 0 },
      { perUserLimit: 0 },
    ])
      expect(() => telegramBindingRequest(provider, connection, { ...room, ...patch })).toThrow()
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
