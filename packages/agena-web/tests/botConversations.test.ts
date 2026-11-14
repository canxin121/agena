import { afterEach, describe, expect, test } from 'bun:test'

import { sessionListResourceKey } from '../src/stores/chat/api'
import { BOT_SURFACE_ORDER, useBotConversations } from '../src/features/bots/useBotConversations'

const originalFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = originalFetch
})

const BOT_ID = '8f2c0000-0000-4000-8000-000000000001'

function storedSession(id: number, botId: string, surface: string, state = 'ready') {
  return {
    id,
    title: `Conversation ${id}`,
    owner_bot_id: botId,
    owner_surface: surface,
    root_id: id,
    depth: 0,
    version: 1,
    relation_kind: 'root',
    lifecycle_state: 'ready',
    state: { kind: state, data: {} },
    message_count: 2,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-02T00:00:00Z',
  }
}

describe('independent Bot sidebar trees', () => {
  test('normalized sessions remain visible and the canonical local root is read even beyond the first history page', async () => {
    const calls: string[] = []
    globalThis.fetch = (async (input, init) => {
      const url = new URL(String(input), 'http://agena.test')
      calls.push(`${init?.method || 'GET'} ${url.pathname}${url.search}`)
      if (url.pathname === '/api/v1/sessions/441') return Response.json(storedSession(441, BOT_ID, 'local'))
      const surface = url.searchParams.get('owner_surface')
      const items =
        surface === 'local'
          ? [storedSession(442, BOT_ID, 'local')]
          : surface === 'group'
            ? [storedSession(471, BOT_ID, 'group', 'awaiting_interaction')]
            : []
      return Response.json({ items, total: surface === 'local' ? 2 : items.length, page: { has_more: false } })
    }) as typeof fetch
    const tree = useBotConversations()
    await tree.load(BOT_ID, 441, true)
    const local = tree.groups(BOT_ID).find((group) => group.surface === 'local')!
    expect(local.error).toBe('')
    expect(local.conversations.map((item) => item.id)).toEqual(['441'])
    expect(local.history.map((item) => item.id)).toEqual(['442'])
    expect(local.history[0].localHistory).toBe(true)
    const room = tree.groups(BOT_ID).find((group) => group.surface === 'group')!.conversations[0]
    expect(room.id).toBe('471')
    expect(room.state).toBe('awaiting_interaction')
    expect(room.running).toBe(true)
    expect(room.localHistory).toBe(false)
    expect(calls.some((call) => call === 'GET /api/v1/sessions/441')).toBe(true)
    expect(calls.every((call) => call.startsWith('GET '))).toBe(true)
    tree.reset()
  })

  test('two expanded Bots retain independent groups and remote pagination appends without duplicates', async () => {
    const otherBot = '8f2c0000-0000-4000-8000-000000000002'
    const calls: string[] = []
    globalThis.fetch = (async (input) => {
      const url = new URL(String(input), 'http://agena.test')
      calls.push(url.search)
      const owner = url.searchParams.get('owner_bot_id')!
      const surface = url.searchParams.get('owner_surface')
      const more = url.searchParams.get('cursor') === 'page-two'
      const items =
        surface === 'direct'
          ? owner === BOT_ID
            ? more
              ? [storedSession(481, owner, 'direct'), storedSession(482, owner, 'direct')]
              : [storedSession(481, owner, 'direct')]
            : [storedSession(483, owner, 'direct')]
          : []
      return Response.json({
        items,
        total: owner === BOT_ID && surface === 'direct' ? 2 : items.length,
        page: { has_more: owner === BOT_ID && surface === 'direct' && !more, next_cursor: more ? null : 'page-two' },
      })
    }) as typeof fetch
    const tree = useBotConversations()
    await Promise.all([tree.load(BOT_ID, null, true), tree.load(otherBot, null, true)])
    expect(
      tree
        .groups(BOT_ID)
        .find((group) => group.surface === 'direct')!
        .conversations.map((item) => item.id),
    ).toEqual(['481'])
    expect(
      tree
        .groups(otherBot)
        .find((group) => group.surface === 'direct')!
        .conversations.map((item) => item.id),
    ).toEqual(['483'])
    await tree.loadMore(BOT_ID, 'direct')
    const direct = tree.groups(BOT_ID).find((group) => group.surface === 'direct')!
    expect(direct.conversations.map((item) => item.id)).toEqual(['481', '482'])
    expect(direct.hasMore).toBe(false)
    expect(calls.at(-1)).toContain('cursor=page-two')
    expect(tree.groups(otherBot).find((group) => group.surface === 'direct')!.conversations[0].id).toBe('483')
    tree.reset()
  })

  test('reset retires an in-flight Bot tree even if its fetch resolves late', async () => {
    let complete!: (response: Response) => void
    let markStarted!: () => void
    const started = new Promise<void>((resolve) => {
      markStarted = resolve
    })
    globalThis.fetch = (async (input) => {
      const url = new URL(String(input), 'http://agena.test')
      if (url.searchParams.get('owner_surface') === 'group') {
        return new Promise<Response>((resolve) => {
          complete = resolve
          markStarted()
        })
      }
      return Response.json({ items: [], total: 0, page: { has_more: false } })
    }) as typeof fetch
    const tree = useBotConversations()
    const pending = tree.load(BOT_ID, null, true)
    await started
    tree.reset(BOT_ID)
    complete(Response.json({ items: [storedSession(491, BOT_ID, 'group')], total: 1, page: { has_more: false } }))
    await pending
    expect(
      tree.groups(BOT_ID).every((group) => !group.loaded && !group.loading && group.conversations.length === 0),
    ).toBe(true)
  })
})

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
    expect(bot).not.toBe(sessionListResourceKey({ ownerBotId: '8f2c0000-0000-4000-8000-000000000002' }))
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
