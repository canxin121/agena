import { afterEach, describe, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import {
  archiveBot,
  botIdQuery,
  botSessionIdQuery,
  createBot,
  emptyBotPersona,
  listBots,
  resolveBotSession,
  resolveBotConversation,
  updateBot,
} from '../src/lib/bots'
import { useUiStore } from '../src/stores/ui'

const A = '11111111-1111-4111-8111-111111111111'
const B = '22222222-2222-4222-8222-222222222222'
const originalFetch = globalThis.fetch
const profile = (id: string, session_id: number, name: string) => ({
  id,
  session_id,
  local_session_id: session_id,
  name,
  persona: emptyBotPersona(),
  revision: 1,
  archived: false,
  created_at_ms: 1,
  updated_at_ms: 1,
})
afterEach(() => {
  globalThis.fetch = originalFetch
})
describe('named Bot contracts', () => {
  test('an explicit remote session performs only reads and never ensures a local conversation', async () => {
    const calls: Array<{ path: string; method: string }> = []
    globalThis.fetch = (async (url, init) => {
      const path = String(url)
      calls.push({ path, method: init?.method || 'GET' })
      if (path === `/api/v1/bots/${A}`) return Response.json(profile(A, 42, 'Atlas'))
      return Response.json({ id: 73, owner_bot_id: A, owner_surface: 'group' })
    }) as typeof fetch
    const resolved = await resolveBotConversation(A, '73')
    expect(resolved.session_id).toBe(73)
    expect(resolved.surface).toBe('group')
    expect(resolved.localHistory).toBe(false)
    expect(calls).toEqual([
      { path: `/api/v1/bots/${A}`, method: 'GET' },
      { path: '/api/v1/sessions/73', method: 'GET' },
    ])
  })
  test('only the canonical local session is writable; history keeps its own ID', async () => {
    globalThis.fetch = (async (url) => {
      const path = String(url)
      if (path === `/api/v1/bots/${A}`) return Response.json(profile(A, 42, 'Atlas'))
      const id = Number(path.split('/').at(-1))
      return Response.json({ id, owner_bot_id: A, owner_surface: 'local' })
    }) as typeof fetch
    expect((await resolveBotConversation(A, '42')).localHistory).toBe(false)
    const history = await resolveBotConversation(A, '41')
    expect(history.session_id).toBe(41)
    expect(history.localHistory).toBe(true)
  })
  test('explicit invalid or foreign session links never fall back to the canonical endpoint', async () => {
    const calls: string[] = []
    globalThis.fetch = (async (url) => {
      calls.push(String(url))
      return String(url).includes('/bots/')
        ? Response.json(profile(A, 42, 'Atlas'))
        : Response.json({ id: 73, owner_bot_id: B, owner_surface: 'direct' })
    }) as typeof fetch
    await expect(resolveBotConversation(A, '!invalid')).rejects.toThrow('Invalid Bot session ID')
    expect(calls).toHaveLength(0)
    await expect(resolveBotConversation(A, '73')).rejects.toThrow('Invalid Bot session response')
    expect(calls.every((path) => !path.endsWith('/session'))).toBe(true)
    expect(botSessionIdQuery({})).toBe('')
    for (const sessionId of ['', '0', '-1', '1.5', ['42'], '9007199254740992']) {
      expect(botSessionIdQuery({ sessionId })).toBe('!invalid')
    }
  })
  test('opening a Bot without a session ensures once and returns the canonical profile', async () => {
    const calls: string[] = []
    globalThis.fetch = (async (url, init) => {
      const path = String(url)
      calls.push(`${init?.method || 'GET'} ${path}`)
      return path.endsWith('/session')
        ? Response.json({ bot_id: A, session_id: 42, name: 'Atlas' })
        : Response.json(profile(A, 42, 'Atlas'))
    }) as typeof fetch
    const resolved = await resolveBotConversation(A)
    expect(resolved.session_id).toBe(42)
    expect(resolved.profile.local_session_id).toBe(42)
    expect(resolved.localHistory).toBe(false)
    expect(calls).toEqual([`POST /api/v1/bots/${A}/session`, `GET /api/v1/bots/${A}`])
  })
  test('list and exact ensure select each server session, while default remains bodyless', async () => {
    const calls: { path: string; body: unknown }[] = []
    globalThis.fetch = (async (url, init) => {
      const path = String(url)
      calls.push({ path, body: init?.body })
      if (path === '/api/v1/bots')
        return Response.json({ items: [profile(A, 42, 'Atlas'), profile(B, 43, 'Muse')], default_bot_id: A })
      const second = path.includes(B)
      return Response.json({ bot_id: second ? B : A, session_id: second ? 43 : 42, name: second ? 'Muse' : 'Atlas' })
    }) as typeof fetch
    expect((await listBots()).items.map((p) => p.name)).toEqual(['Atlas', 'Muse'])
    expect((await resolveBotSession(A)).session_id).toBe(42)
    expect((await resolveBotSession(B)).session_id).toBe(43)
    expect((await resolveBotSession()).bot_id).toBe(A)
    expect(calls).toEqual([
      { path: '/api/v1/bots', body: undefined },
      { path: `/api/v1/bots/${A}/session`, body: undefined },
      { path: `/api/v1/bots/${B}/session`, body: undefined },
      { path: '/api/v1/bot/session', body: undefined },
    ])
  })
  test('explicit malformed/archived identities never fall back and mismatched ensures are rejected', async () => {
    let calls = 0
    globalThis.fetch = (async () => {
      calls++
      return Response.json({ bot_id: B, session_id: 43, name: 'Muse' })
    }) as typeof fetch
    await expect(resolveBotSession('invalid')).rejects.toThrow('Invalid Bot ID')
    expect(calls).toBe(0)
    await expect(resolveBotSession(A)).rejects.toThrow('Invalid Bot session response')
    globalThis.fetch = (async () => Response.json({ error: 'Bot archived' }, { status: 404 })) as typeof fetch
    await expect(resolveBotSession(A)).rejects.toThrow('Bot archived')
    expect(botIdQuery({ botId: '' })).toBe('!invalid')
    expect(botIdQuery({ botId: [A, B] })).toBe('!invalid')
    expect(botIdQuery({ sessionId: '43' })).toBe('')
  })
  test('mutations carry stable operation keys, explicit persona and expected revision on exact endpoints', async () => {
    const calls: unknown[] = []
    globalThis.fetch = (async (url, init) => {
      calls.push([url, init?.method, JSON.parse(String(init?.body))])
      return Response.json(profile(A, 42, 'Atlas'))
    }) as typeof fetch
    const create = { request_key: 'create-retry-key', name: 'Atlas' }
    await createBot(create)
    await createBot(create)
    const update = {
      request_key: 'edit-retry-key',
      expected_revision: 3,
      name: 'Atlas II',
      persona: { ...emptyBotPersona(), instructions: 'Be precise.' },
    }
    await updateBot(A, update)
    const archive = { request_key: 'archive-retry-key', expected_revision: 4 }
    await archiveBot(A, archive)
    expect(calls).toEqual([
      ['/api/v1/bots', 'POST', create],
      ['/api/v1/bots', 'POST', create],
      [`/api/v1/bots/${A}`, 'PUT', update],
      [`/api/v1/bots/${A}/archive`, 'POST', archive],
    ])
  })
  test('two named tabs survive persistence independently and recovered sessionId is not entity identity', () => {
    const pinia = createPinia()
    setActivePinia(pinia)
    try {
      const ui = useUiStore()
      const a = ui.openWorkspaceWindow('bot', {
        path: '/bot',
        query: { botId: A, sessionId: '42' },
        title: 'Atlas',
        matchKeys: ['botId'],
      })
      const b = ui.openWorkspaceWindow('bot', {
        path: '/bot',
        query: { botId: B, sessionId: '43' },
        title: 'Muse',
        matchKeys: ['botId'],
      })
      expect(a).not.toBe(b)
      expect(ui.workspaceWindows).toHaveLength(2)
      expect(
        ui.openWorkspaceWindow('bot', { path: '/bot', query: { botId: A, sessionId: '999' }, matchKeys: ['botId'] }),
      ).toBe(a)
      expect(ui.findWorkspaceWindowByTabAndQuery('bot', { botId: B }, { keys: ['botId'] })?.id).toBe(b)
      expect(ui.findWorkspaceWindowByTabAndQuery('bot', { botId: 'unknown' }, { keys: ['botId'] })).toBeNull()
      expect(ui.workspaceWindows.map((w) => w.title)).toEqual(['Atlas', 'Muse'])
    } finally {
      disposePinia(pinia)
      setActivePinia(undefined)
    }
  })
})
