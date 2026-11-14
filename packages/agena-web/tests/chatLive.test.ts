import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia'
import { useChatStoreUnscoped } from '../src/stores/chat'
import * as chatApi from '../src/stores/chat/api'
import * as sse from '../src/lib/sse'

const names = ['window', 'document', 'localStorage', 'sessionStorage'] as const
const originals = new Map(names.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]))
let pinia: Pinia
let doc: EventTarget & { visibilityState: string }
const connections: Array<{ options: sse.SseClientOptions; closed: number }> = []

beforeEach(() => {
  const storage = () => {
    const values = new Map<string, string>()
    return {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        values.set(key, value)
      },
      removeItem: (key: string) => {
        values.delete(key)
      },
    }
  }
  doc = Object.assign(new EventTarget(), { visibilityState: 'visible' })
  const values = {
    window: Object.assign(new EventTarget(), {
      location: {
        origin: 'https://agena.test',
        href: 'https://agena.test/bot',
        pathname: '/bot',
        search: '',
        hash: '',
      },
      setTimeout,
      clearTimeout,
    }),
    document: doc,
    localStorage: storage(),
    sessionStorage: storage(),
  }
  for (const name of names) Object.defineProperty(globalThis, name, { configurable: true, value: values[name] })
  pinia = createPinia()
  setActivePinia(pinia)
  connections.length = 0
  spyOn(sse, 'connectSse').mockImplementation((options) => {
    const connection = { options, closed: 0 }
    connections.push(connection)
    return {
      close: () => {
        connection.closed++
      },
      getStats: () => ({}) as sse.SseClientStats,
    }
  })
  // These tests exercise live delivery independently of snapshot success.
  spyOn(chatApi, 'getSessionExecutionStatus').mockImplementation(() => new Promise(() => {}))
  spyOn(chatApi, 'listMessages').mockImplementation(() => new Promise(() => {}))
})
afterEach(() => {
  disposePinia(pinia)
  setActivePinia(undefined)
  mock.restore()
  for (const name of names) {
    const previous = originals.get(name)
    if (previous) Object.defineProperty(globalThis, name, previous)
    else Reflect.deleteProperty(globalThis, name)
  }
})

test('visible Bot conversation owns a session-scoped stream, shared across panes', () => {
  const chat = useChatStoreUnscoped(pinia)
  const first = chat.retainSession('6')
  const second = chat.retainSession('6')
  expect(connections).toHaveLength(1)
  const live = connections[0]!
  const url = new URL(live.options.endpoint!, 'https://agena.test')
  expect(url.searchParams.get('scope_kind')).toBe('session')
  expect(url.searchParams.get('session_id')).toBe('6')
  live.options.onEvent({
    type: 'session_changed',
    properties: {
      kind: 'part_updated',
      session_id: 6,
      part: {
        part_id: 8,
        run_id: 8,
        message_boundary: true,
        role: 'assistant',
        kind: 'text',
        state: 'completed',
        run_state: 'completed',
        content: { document: [] },
        sections: [],
        revision: 1,
        created_at_ms: 1,
        updated_at_ms: 2,
      },
    },
  })
  expect(chat.getMessagesForSession('6').map((message) => message.info.id)).toEqual(['8'])
  first()
  expect(live.closed).toBe(0)
  second()
  expect(live.closed).toBe(1)
})

test('hidden sessions stop their streams and resume with a new subscription', () => {
  const chat = useChatStoreUnscoped(pinia)
  const release = chat.retainSession('6')
  expect(connections).toHaveLength(1)
  doc.visibilityState = 'hidden'
  doc.dispatchEvent(new Event('visibilitychange'))
  expect(connections[0]!.closed).toBe(1)
  doc.visibilityState = 'visible'
  doc.dispatchEvent(new Event('visibilitychange'))
  expect(connections).toHaveLength(2)
  release()
  expect(connections[1]!.closed).toBe(1)
})

test('late events from a closed pane cannot repopulate its conversation', () => {
  const chat = useChatStoreUnscoped(pinia)
  const release = chat.retainSession('6')
  const old = connections[0]!
  release()
  old.options.onEvent({
    type: 'session_changed',
    properties: {
      kind: 'part_updated',
      session_id: 6,
      part: {
        part_id: 80,
        run_id: 80,
        message_boundary: true,
        role: 'assistant',
        kind: 'text',
        state: 'completed',
        content: { document: [] },
        revision: 1,
      },
    },
  })
  expect(chat.getMessagesForSession('6')).toEqual([])
})

test("releasing one owner twice does not close another pane's live tail", () => {
  const chat = useChatStoreUnscoped(pinia)
  const first = chat.retainSession('6')
  const second = chat.retainSession('6')
  first()
  first()
  expect(connections[0]!.closed).toBe(0)
  second()
  expect(connections[0]!.closed).toBe(1)
})

test('opening a live tail immediately reconciles a snapshot without waiting for the global poll', async () => {
  const chat = useChatStoreUnscoped(pinia)
  const release = chat.retainSession('6')
  const list = spyOn(chatApi, 'listMessages').mockImplementation(() => new Promise(() => {}))
  connections[0]!.options.onOpen?.()
  await new Promise((resolve) => setTimeout(resolve, 30))
  expect(list).toHaveBeenCalledTimes(1)
  expect(list.mock.calls[0]![0]).toBe('6')
  release()
})
