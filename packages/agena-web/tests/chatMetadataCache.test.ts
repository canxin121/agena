import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia'
import { useChatStoreUnscoped } from '../src/stores/chat'
import * as chatApi from '../src/stores/chat/api'
import { readLastSessionId, writeLastSessionId } from '../src/lib/lastSession'

const browserGlobals = ['window', 'document', 'localStorage', 'sessionStorage'] as const
const originalGlobals = new Map(browserGlobals.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]))
const originalFetch = globalThis.fetch
let pinia: Pinia
let previousLastSession = ''
let requests: string[]

function memoryStorage() {
  const values = new Map<string, string>()
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  }
}

beforeEach(() => {
  previousLastSession = readLastSessionId()
  requests = []
  globalThis.fetch = (async (url) => {
    requests.push(String(url))
    throw new Error(`Unexpected fixture request: ${String(url)}`)
  }) as typeof fetch
  const values = {
    window: Object.assign(new EventTarget(), {
      location: {
        origin: 'https://agena.test',
        href: 'https://agena.test/settings/permissions',
        pathname: '/settings/permissions',
        search: '',
        hash: '',
      },
      setTimeout,
      clearTimeout,
    }),
    document: undefined,
    localStorage: memoryStorage(),
    sessionStorage: memoryStorage(),
  }
  for (const name of browserGlobals)
    Object.defineProperty(globalThis, name, { configurable: true, value: values[name] })
  pinia = createPinia()
  setActivePinia(pinia)
  writeLastSessionId('7')
})

afterEach(() => {
  disposePinia(pinia)
  setActivePinia(undefined)
  mock.restore()
  writeLastSessionId(previousLastSession)
  globalThis.fetch = originalFetch
  for (const name of browserGlobals) {
    const descriptor = originalGlobals.get(name)
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

function permissionResponse(id: string, version = 2): chatApi.AgenaExecutionState {
  return {
    session: {
      id: Number(id),
      title: 'Updated ' + id,
      version,
      workspace_id: 2,
      ...(id === '42' ? { owner_bot_id: 'bot-fixture' } : {}),
      state: { kind: 'ready', data: {} },
    },
    parts: [],
    part_page: null,
    latest_event_seq: version,
    execution: { selected_permission: { tools: { default: 'deny' } } },
    queued_messages: [],
    queue_paused: false,
  }
}

describe('settings session metadata and permission writes', () => {
  test('catalog metadata does not insert, reorder, trim, or activate recent conversations', () => {
    const chat = useChatStoreUnscoped(pinia)
    chat.selectedSessionId = '7'
    chat.cacheSessions(Array.from({ length: 30 }, (_, index) => ({ id: String(index + 1), title: `Recent ${index}` })))
    const recent = chat.sessions
    const snapshot = JSON.stringify(recent)
    chat.cacheSessionMetadata([
      { id: '7', title: 'Metadata title', workspace_id: 2 },
      { id: '42', title: 'Bot session', workspace_id: 2, owner_bot_id: 'bot-fixture' },
      ...Array.from({ length: 80 }, (_, index) => ({ id: String(index + 100), title: `Older ${index}` })),
    ])

    expect(chat.sessions).toBe(recent)
    expect(JSON.stringify(chat.sessions)).toBe(snapshot)
    expect(chat.getSessionById('7')?.title).toBe('Metadata title')
    expect(chat.getSessionById('42')?.owner_bot_id).toBe('bot-fixture')
    expect(chat.getSessionById('164')?.title).toBe('Older 64')
    expect(chat.selectedSessionId).toBe('7')
    expect(readLastSessionId()).toBe('7')
    expect(chat.getMessagesForSession('42')).toEqual([])
    expect(requests).toEqual([])

    chat.cacheSessions([{ id: '900', title: 'Opened through existing cache API' }])
    expect(chat.sessions[0]?.id).toBe('900')
    expect(chat.sessions).toHaveLength(30)
  })

  test('metadata lookups retain execution snapshots and reject stale versions or deleted sessions', () => {
    const chat = useChatStoreUnscoped(pinia)
    chat.cacheSessions([{ id: '7', title: 'Current', version: 3, state: { kind: 'ready', data: {} } }])
    chat.cacheSessionMetadata([
      { id: '7', title: 'Renamed', version: 4, state: { kind: 'running', data: { workflow: 'quiescent' } } },
      { id: '42', title: 'Before deletion', version: 1 },
    ])
    expect(chat.getSessionById('7')?.title).toBe('Renamed')
    expect(chat.getSessionState('7').kind).toBe('ready')
    chat.cacheSessionMetadata([{ id: '7', title: 'Stale', version: 2 }])
    expect(chat.getSessionById('7')?.title).toBe('Renamed')
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'session_deleted', session_id: 42 } })
    chat.cacheSessionMetadata([{ id: '42', title: 'Late response', version: 99 }])
    expect(chat.getSessionById('42')).toBeNull()
    expect(requests).toEqual([])
  })

  test('saving policies for older and Bot targets preserves recent order and does not load their messages', async () => {
    const chat = useChatStoreUnscoped(pinia)
    chat.selectedSessionId = '7'
    chat.cacheSessions([
      { id: '7', title: 'Current', version: 1 },
      { id: '8', title: 'Another recent', version: 1 },
    ])
    const recent = chat.sessions.map((session) => session.id)
    const save = spyOn(chatApi, 'setSessionPermission').mockImplementation(async (id) => permissionResponse(id))
    const messages = spyOn(chatApi, 'listMessages')
    const policy = { tools: { default: 'deny' } }
    await chat.setSessionPermission('164', policy)
    await chat.setSessionPermission('42', policy)
    await new Promise((resolve) => setTimeout(resolve, 5))

    expect(save.mock.calls).toEqual([
      ['164', policy],
      ['42', policy],
    ])
    expect(chat.sessions.map((session) => session.id)).toEqual(recent)
    expect(chat.getSessionById('164')?.title).toBe('Updated 164')
    expect(chat.getSessionById('42')?.owner_bot_id).toBe('bot-fixture')
    expect(chat.getSessionExecution('42')?.selected_permission).toEqual(policy)
    expect(chat.selectedSessionId).toBe('7')
    expect(readLastSessionId()).toBe('7')
    expect(messages).not.toHaveBeenCalled()
    expect(requests).toEqual([])
  })

  test('saving an existing recent target updates its state without inserting or moving a row', async () => {
    const chat = useChatStoreUnscoped(pinia)
    chat.selectedSessionId = '7'
    chat.cacheSessions([
      { id: '7', title: 'Current', version: 1 },
      { id: '8', title: 'Older', version: 1, state: { kind: 'running', data: { workflow: 'quiescent' } } },
      { id: '9', title: 'Newest', version: 1 },
    ])
    const recent = chat.sessions.map((session) => session.id)
    spyOn(chatApi, 'setSessionPermission').mockResolvedValue(permissionResponse('8'))
    await chat.setSessionPermission('8', {})

    expect(chat.sessions.map((session) => session.id)).toEqual(recent)
    expect(chat.sessions.find((session) => session.id === '8')?.title).toBe('Updated 8')
    expect(chat.sessions.find((session) => session.id === '8')?.state?.kind).toBe('ready')
    expect(chat.getSessionState('8').kind).toBe('ready')
    expect(chat.selectedSessionId).toBe('7')
    expect(readLastSessionId()).toBe('7')
  })
})
