import { afterEach, describe, expect, test } from 'bun:test'

import { sessionListResourceKey } from '../src/stores/chat/api'
import { BOT_SURFACE_ORDER } from '../src/features/bots/useBotConversations'

const originalFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = originalFetch
})

const BOT_ID = '8f2c0000-0000-4000-8000-000000000001'

describe('Bot conversation listing', () => {
  test('requests the owner filter and keeps Bot lists in their own cache entry', async () => {
    const urls: string[] = []
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      urls.push(String(input))
      return new Response(
        JSON.stringify({
          items: [
            {
              id: 1,
              title: 'Local',
              owner_bot_id: BOT_ID,
              owner_surface: 'local',
              depth: 0,
              root_id: 1,
              version: 1,
              child_session_count: 0,
              relation_kind: 'root',
              lifecycle_state: 'ready',
              message_count: 1,
              state: { kind: 'ready', data: {} },
              created_at: '2026-01-01T00:00:00Z',
              updated_at: '2026-01-01T00:00:00Z',
            },
          ],
          page: { has_more: false, returned: 1 },
          total: 1,
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      )
    }) as typeof fetch

    const { listSessions } = await import('../src/stores/chat/api')
    const page = await listSessions({ ownerBotId: BOT_ID, limit: 50 })
    expect(urls).toHaveLength(1)
    expect(urls[0]).toContain(`owner_bot_id=${BOT_ID}`)
    expect(page.sessions[0].owner_surface).toBe('local')
    expect(page.sessions[0].owner_bot_id).toBe(BOT_ID)
  })

  test('Bot lists never share a cache key with projects or another Bot', () => {
    const bot = sessionListResourceKey({ ownerBotId: BOT_ID })
    expect(bot).toBe(`bot:${BOT_ID}:sessions`)
    expect(bot).not.toBe(sessionListResourceKey({}))
    expect(bot).not.toBe(sessionListResourceKey({ workspaceId: 7 }))
    expect(bot).not.toBe(
      sessionListResourceKey({ ownerBotId: '8f2c0000-0000-4000-8000-000000000002' }),
    )
    expect(sessionListResourceKey({ ownerBotId: BOT_ID, ownerSurface: 'group' })).toBe(
      `bot:${BOT_ID}:sessions:surface:group`,
    )
  })

  test('surface order matches the sidebar and covers every entrance shape', () => {
    expect(BOT_SURFACE_ORDER).toEqual(['local', 'direct', 'group', 'broadcast'])
  })
})

describe('deleting a busy conversation', () => {
  test('stops the live execution before removing the session', async () => {
    const calls: Array<{ method: string; url: string }> = []
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      const method = (init?.method || 'GET').toUpperCase()
      calls.push({ method, url })
      if (url.endsWith('/cancel')) {
        return new Response(JSON.stringify({ result: 'cancellation_requested' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        })
      }
      return new Response(JSON.stringify({ id: 7 }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    }) as typeof fetch

    const { deleteSession } = await import('../src/stores/chat/api')
    const { cancelSession } = await import('../src/stores/chat/api')

    // The stop-then-delete sequence a confirmed busy delete performs.
    const brief = await cancelSession('7', null)
    expect(brief.result).toBe('cancellation_requested')
    await deleteSession('7')

    expect(calls.map((call) => `${call.method} ${call.url.split('?')[0]}`)).toEqual([
      'POST /api/v1/sessions/7/cancel',
      'DELETE /api/v1/sessions/7',
    ])
    // The stop always precedes the delete, so the transcript cannot vanish
    // while the provider is still writing into it.
    expect(calls[0].url).toContain('/cancel')
    expect(calls.at(-1)!.method).toBe('DELETE')
  })
})
