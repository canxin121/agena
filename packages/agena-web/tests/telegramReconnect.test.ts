import { describe, expect, test } from 'bun:test'
import {
  reconnectTelegramBinding,
  telegramBindingNeedsReview,
  telegramReconnectAttempt,
} from '../src/features/telegram/reconnect'
import type {
  IntegrationBinding,
  IntegrationControl,
  IntegrationDetail,
  CreateIntegration,
} from '../src/generated/agenaState'

function binding(): IntegrationBinding {
  return {
    id: 'old-binding',
    source_session_id: 7,
    owner_bot_id: 'ai-bot',
    revision: 4,
    state: 'faulted',
    last_error: 'plugin identity/configuration changed; review and explicitly re-enable the grant',
    spec: {
      provider: 'agena.telegram',
      mode: 'channel',
      resource: '-88',
      thread: '42',
      config: { bot_id: 'telegram-bot' },
      channel_policy: {
        administrator_actors: ['77'],
        members: [{ actor: '77', role: 'administrator' }],
        grant: { tools: ['agena.fs.read'] },
      },
    },
  } as IntegrationBinding
}
function fakeApi(original = binding()) {
  const calls: string[] = []
  const requests: CreateIntegration[] = []
  let replacement: IntegrationBinding | null = null
  let failEnable = false
  const api = {
    async detail() {
      return { binding: original } as IntegrationDetail
    },
    async create(source: number, request: CreateIntegration) {
      expect(source).toBe(original.source_session_id)
      calls.push('create')
      requests.push(request)
      replacement ||= { ...original, id: 'replacement', state: 'disabled', revision: 1, spec: request.spec }
      return replacement
    },
    async control(source: number, id: string, request: IntegrationControl) {
      calls.push(request.action + ':' + id)
      expect(source).toBe(original.source_session_id)
      if (id === original.id) {
        expect(request.expected_revision).toBe(original.revision)
        original = { ...original, state: 'paused', revision: original.revision + 1 }
        return original
      }
      if (failEnable) {
        failEnable = false
        throw new Error('temporary network failure')
      }
      expect(request.not_before_ms).toBe(12345)
      replacement = { ...replacement!, state: 'enabled', revision: replacement!.revision + 1 }
      return replacement
    },
  }
  return {
    api,
    calls,
    requests,
    failNextEnable() {
      failEnable = true
    },
  }
}
describe('reviewing a changed Telegram integration', () => {
  test('creates a disabled replacement, pauses the old route, then enables at the explicit review time', async () => {
    const original = binding(),
      fixture = fakeApi(original)
    const attempt = telegramReconnectAttempt(original, 12345)
    const replacement = await reconnectTelegramBinding(fixture.api, attempt)
    expect(fixture.calls).toEqual(['create', 'pause:old-binding', 'enable:replacement'])
    expect(replacement.id).toBe('replacement')
    expect(replacement.spec).toEqual(original.spec)
    expect(attempt.request.spec).not.toBe(original.spec)
    expect(attempt.request.request_key).toBe('telegram-reconnect:old-binding')
  })
  test('a failed enable retries the same replacement and cutoff without re-pausing the old route', async () => {
    const original = binding(),
      fixture = fakeApi(original)
    const attempt = telegramReconnectAttempt(original, 12345)
    fixture.failNextEnable()
    await expect(reconnectTelegramBinding(fixture.api, attempt)).rejects.toThrow('temporary network failure')
    await reconnectTelegramBinding(fixture.api, attempt)
    expect(fixture.calls).toEqual(['create', 'pause:old-binding', 'enable:replacement', 'create', 'enable:replacement'])
    expect(fixture.requests[0]!.request_key).toBe(fixture.requests[1]!.request_key)
  })
  test('a stale review does not create or enable a replacement after another policy edit', async () => {
    const original = binding(),
      attempt = telegramReconnectAttempt(original, 12345)
    const fixture = fakeApi({ ...original, revision: original.revision + 1 })
    await expect(reconnectTelegramBinding(fixture.api, attempt)).rejects.toThrow('Review its latest settings')
    expect(fixture.calls).toEqual([])
  })
  test('faulted channel connections remain reviewable when access edits clear the error', () => {
    expect(telegramBindingNeedsReview(binding())).toBe(true)
    expect(
      telegramBindingNeedsReview({
        ...binding(),
        last_error: 'adapter or source permission boundary changed; new work is stopped until explicit review',
      }),
    ).toBe(true)
    expect(telegramBindingNeedsReview({ ...binding(), state: 'paused' })).toBe(false)
    expect(telegramBindingNeedsReview({ ...binding(), last_error: null })).toBe(true)
    expect(telegramBindingNeedsReview({ ...binding(), last_error: 'rate limit exceeded' })).toBe(true)
    expect(telegramBindingNeedsReview({ ...binding(), state: 'enabled' })).toBe(false)
    expect(telegramBindingNeedsReview({ ...binding(), spec: { ...binding().spec, mode: 'session' } })).toBe(false)
  })
})
