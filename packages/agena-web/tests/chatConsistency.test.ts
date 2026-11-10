import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia'
import { computed, createApp, shallowRef } from 'vue'
import type { RouteLocationNormalizedLoaded, RouteLocationRaw } from 'vue-router'
import { workspacePaneContextKey, useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useChatStore, useChatStoreUnscoped } from '../src/stores/chat'
import { useDirectoryStoreUnscoped } from '../src/stores/directory'
import * as chatApi from '../src/stores/chat/api'
import type { AgenaPart } from '../src/stores/chat/api'

const browserGlobals = ['window', 'document', 'localStorage', 'sessionStorage'] as const
const originalGlobals = new Map(browserGlobals.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]))
const originalFetch = globalThis.fetch
let pinia: Pinia

function memoryStorage() {
  const values = new Map<string, string>()
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  }
}

beforeEach(() => {
  globalThis.fetch = (async (url) => {
    throw new Error(`Unexpected fixture request: ${String(url)}`)
  }) as typeof fetch
  const values = {
    window: Object.assign(new EventTarget(), {
      location: {
        origin: 'https://agena.test',
        href: 'https://agena.test/chat',
        pathname: '/chat',
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
})

afterEach(() => {
  disposePinia(pinia)
  setActivePinia(undefined)
  mock.restore()
  globalThis.fetch = originalFetch
  for (const name of browserGlobals) {
    const descriptor = originalGlobals.get(name)
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

function part(overrides: Partial<AgenaPart> = {}): AgenaPart {
  return {
    part_id: 10,
    run_id: 10,
    message_boundary: true,
    role: 'assistant',
    kind: 'text',
    state: 'running',
    content: { document: [], execution_phase: 'running' },
    sections: [],
    revision: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

function execution(controlVersion: number, active = false): chatApi.AgenaExecutionState {
  return {
    session: {
      id: 1,
      version: 7,
      workspace_id: 1,
      state: active
        ? {
            kind: 'awaiting_interaction',
            data: { requests: [{ kind: 'permission', request_id: 'old-permission', session_id: 1 }] },
          }
        : { kind: 'ready', data: {} },
    },
    latest_event_seq: controlVersion,
    parts: [],
    part_page: null,
    execution: { agent_status: active ? 'running' : 'completed' },
    queued_messages: active ? [{ execution_id: 'old-queued-execution', message_id: 99 }] : [],
    queue_paused: active,
    usage: { current_tokens: controlVersion },
    background_activities: [
      {
        id: 'background-child',
        kind: 'agent',
        status: active ? 'running' : 'completed',
        title: 'Child',
        description: '',
        last_seq: controlVersion,
        controls: [],
      },
    ],
  }
}

function status(resource: chatApi.AgenaExecutionState): chatApi.SessionExecutionStatus {
  const result = {
    controlVersion: resource.latest_event_seq ?? undefined,
    session: chatApi.toSession(resource.session)!,
    state: resource.session.state!,
    parts: resource.parts,
    execution: resource.execution,
    usage: resource.usage,
    queuedMessages: resource.queued_messages,
    queuePaused: resource.queue_paused,
    backgroundActivities: resource.background_activities ?? [],
    backgroundActivityKinds: (resource.background_activities ?? []).map((item) => item.kind),
  }
  return result
}

function paneStore() {
  const raw = useChatStoreUnscoped(pinia)
  const route = shallowRef({ path: '/chat', query: { sessionId: '99' }, hash: '' } as RouteLocationNormalizedLoaded)
  const navigations: RouteLocationRaw[] = []
  const app = createApp({})
  app.use(pinia)
  const pane = {
    windowId: computed(() => 'chat-pane'),
    isFocused: computed(() => true),
    isVisible: computed(() => true),
    route: computed(() => route.value),
    navigate: async (target: RouteLocationRaw) => {
      navigations.push(target)
      route.value = { ...route.value, ...(target as object) } as RouteLocationNormalizedLoaded
    },
  }
  app.provide(workspacePaneContextKey, pane)
  const chat = app.runWithContext(() => {
    expect(useWorkspacePaneContext()).toBe(pane)
    return useChatStore(pinia)
  })
  expect(chat.$state).toBe(raw.$state)
  expect(chat.selectedSessionId).toBe('99')
  return { chat, route, navigations }
}

async function until(check: () => boolean) {
  const deadline = Date.now() + 1500
  while (!check() && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 5))
  expect(check()).toBe(true)
}

describe('chat session consistency', () => {
  test('parent part updates do not alter loaded fork or nested-fork snapshots', () => {
    const chat = useChatStoreUnscoped(pinia)
    const tool = part({
      part_id: 11,
      message_boundary: false,
      kind: 'tool_call',
      content: { name: 'fs.read', input: {} },
    })
    const seed = (sid: string) => {
      chat.selectedSessionId = sid
      for (const snapshot of [part(), tool]) {
        chat.applyEvent({
          type: 'session_changed',
          properties: { session_id: Number(sid), kind: 'part_added', part: snapshot, resource_revisions: {} },
        })
      }
    }
    for (const sid of ['1', '2', '3']) seed(sid)
    const branches = ['2', '3'].map((sid) => JSON.stringify(chat.getMessagesForSession(sid)))

    for (const snapshot of [
      part({ state: 'completed', revision: 2, updated_at_ms: 2, content: { execution_phase: 'succeeded' } }),
      {
        ...tool,
        state: 'completed',
        revision: 2,
        updated_at_ms: 2,
        content: { name: 'fs.read', output: 'PARENT_ONLY_RESULT' },
      },
    ]) {
      chat.applyEvent({
        type: 'session_changed',
        properties: { session_id: 1, kind: 'part_updated', part: snapshot, resource_revisions: {} },
      })
    }

    const parent = chat.getMessagesForSession('1')[0]!
    expect(parent.info.runState).toBe('completed')
    expect(parent.parts.find((entry) => entry.id === '11')?.partState).toBe('completed')
    expect(JSON.stringify(parent)).toContain('PARENT_ONLY_RESULT')
    for (const [index, sid] of ['2', '3'].entries()) {
      expect(JSON.stringify(chat.getMessagesForSession(sid))).toBe(branches[index])
      expect(chat.getMessagesForSession(sid)[0]!.info.runState).toBe('running')
    }
  })

  test('a delayed send acknowledgement cannot restore an older execution, queue, or interaction', async () => {
    const chat = useChatStoreUnscoped(pinia)
    const acknowledged = deferred<chatApi.AgenaExecutionState>()
    spyOn(chatApi, 'sendMessage').mockReturnValue(acknowledged.promise)
    spyOn(chatApi, 'getSessionExecutionStatus').mockResolvedValue(status(execution(12)))
    const sent = chat.sendMessage('1', { text: 'Work that completed before its HTTP acknowledgement arrived' })
    await chat.refreshExecutionStatus('1')
    acknowledged.resolve(execution(10, true))
    await sent

    expect(chat.getSessionState('1').kind).toBe('ready')
    const current = chat.getSessionExecution('1')
    expect(current?.agent_status).toBe('completed')
    expect(Array.isArray(current?.queued_messages)).toBe(true)
    expect(current?.queued_messages).toHaveLength(0)
    expect(current?.queue_paused).toBe(false)
    expect(chat.getPendingInteraction('1', 'old-permission', 'permission')).toBeNull()
    expect(chat.getSessionAttention('1')).toBeNull()
    expect(chat.getSessionUsage('1')?.current_tokens).toBe(12)
    expect(chat.sessionBackgroundActivities('1')[0]?.status).toBe('completed')
  })

  test('an old session list with an unchanged metadata version cannot undo a newer execution snapshot', async () => {
    const chat = useChatStoreUnscoped(pinia)
    const listed = deferred<chatApi.SessionListResponse>()
    spyOn(chatApi, 'listSessions').mockReturnValue(listed.promise)
    spyOn(chatApi, 'getSessionExecutionStatus').mockResolvedValue(status(execution(12)))
    const loading = chat.refreshSessions()
    await chat.refreshExecutionStatus('1')
    listed.resolve({ sessions: [chatApi.toSession(execution(10, true).session)!] })
    await loading
    expect(chat.getSessionState('1').kind).toBe('ready')
    expect(chat.getSessionExecution('1')?.queue_paused).toBe(false)
  })

  test('reopening a cached session reconciles a newer background-state hint', async () => {
    const chat = useChatStoreUnscoped(pinia)
    const read = spyOn(chatApi, 'getSessionExecutionStatus').mockResolvedValue(status(execution(12)))
    await chat.refreshExecutionStatus('1')
    const started = execution(13, true)
    started.session.state = { kind: 'running', data: { workflow: 'quiescent' } }
    started.queued_messages = []
    read.mockResolvedValue(status(started))
    chat.cacheSessions([chatApi.toSession(started.session)!])
    const release = chat.retainSession('1')
    try {
      await until(() => chat.getSessionState('1').kind === 'running')
      expect(chat.getSessionExecution('1')?.agent_status).toBe('running')
      expect(read.mock.calls.length).toBeGreaterThanOrEqual(2)
    } finally {
      release()
    }
  })

  test('a slower pane selection cannot navigate over a newer selection', async () => {
    const raw = useChatStoreUnscoped(pinia)
    raw.cacheSessions([
      { id: '1', workspace_id: 1 },
      { id: '2', workspace_id: 2 },
    ])
    const older = deferred<chatApi.TranscriptMessageListResponse>()
    const page = { entries: [], userMessageCount: 0, hasMore: false }
    const reads = spyOn(chatApi, 'listMessages').mockImplementation(async (sid) => (sid === '1' ? older.promise : page))
    spyOn(chatApi, 'getWorkspace').mockImplementation(async (id) => ({ id: Number(id), path: '/workspace' }))
    spyOn(chatApi, 'getSessionExecutionStatus').mockImplementation(async (sid) => {
      const value = execution(12)
      value.session.id = Number(sid)
      return status(value)
    })
    const { chat, route, navigations } = paneStore()
    const first = chat.selectSession('1')
    await until(() => reads.mock.calls.some(([sid]) => sid === '1'))
    await chat.selectSession('2')
    older.resolve(page)
    await first
    expect(route.value.query.sessionId).toBe('2')
    expect(navigations).toHaveLength(1)
  })

  test('an in-place route change from chat to Bot invalidates a pending pane selection', async () => {
    const raw = useChatStoreUnscoped(pinia)
    raw.cacheSessions([{ id: '1' }])
    const older = deferred<chatApi.TranscriptMessageListResponse>()
    const page = { entries: [], userMessageCount: 0, hasMore: false }
    const reads = spyOn(chatApi, 'listMessages').mockImplementation(() => older.promise)
    spyOn(chatApi, 'getSessionExecutionStatus').mockImplementation(async () => status(execution(12)))
    const { chat, route, navigations } = paneStore()
    const pending = chat.selectSession('1')
    await until(() => reads.mock.calls.length > 0)
    // WorkspacePaneView uses Object.assign on a stable shallowReactive route.
    Object.assign(route.value, { path: '/bot', query: { sessionId: '42' } })
    older.resolve(page)
    await pending
    expect(route.value.path).toBe('/bot')
    expect(route.value.query.sessionId).toBe('42')
    expect(navigations).toEqual([])
  })

  test('a slower workspace lookup cannot change the directory of the newly selected pane', async () => {
    const raw = useChatStoreUnscoped(pinia)
    raw.cacheSessions([
      { id: '1', workspace_id: 1 },
      { id: '2', workspace_id: 2 },
    ])
    const older = deferred<{ id: number; path: string }>()
    spyOn(chatApi, 'getWorkspace').mockImplementation(async (id) =>
      Number(id) === 1 ? older.promise : { id: 2, path: '/second-workspace' },
    )
    spyOn(chatApi, 'listMessages').mockResolvedValue({ entries: [], userMessageCount: 0, hasMore: false })
    spyOn(chatApi, 'getSessionExecutionStatus').mockImplementation(async (sid) => {
      const value = execution(12)
      value.session.id = Number(sid)
      return status(value)
    })
    const { chat, route } = paneStore()
    const directory = useDirectoryStoreUnscoped(pinia)
    const first = chat.selectSession('1')
    const second = chat.selectSession('2')
    await Promise.all([first, second])
    await until(() => directory.getDirectoryForWindow('chat-pane') === '/second-workspace')
    older.resolve({ id: 1, path: '/first-workspace' })
    await until(() => raw.getSessionDirectory('1') === '/first-workspace')
    expect(route.value.query.sessionId).toBe('2')
    expect(directory.getDirectoryForWindow('chat-pane')).toBe('/second-workspace')
  })

  test('global hydration preserves remembered project directories', async () => {
    const chat = useChatStoreUnscoped(pinia)
    const directory = useDirectoryStoreUnscoped(pinia)
    directory.setDirectory('/remembered-project')
    directory.setDirectoryForWindow('project-pane', '/project-pane')
    chat.cacheSessions([{ id: '42', workspace_id: null }])
    const workspace = spyOn(chatApi, 'getWorkspace')
    spyOn(chatApi, 'listMessages').mockResolvedValue({ entries: [], userMessageCount: 0, hasMore: false })
    await chat.hydrateSession('42', { windowId: 'bot-pane' })
    expect(workspace).not.toHaveBeenCalled()
    expect(directory.currentDirectory).toBe('/remembered-project')
    expect(directory.getDirectoryForWindow('project-pane')).toBe('/project-pane')
    expect(chat.getSessionDirectory('42')).toBeNull()
  })

  test('an older session load cannot clear the new selected session loading indicator', async () => {
    const chat = useChatStoreUnscoped(pinia)
    chat.cacheSessions([
      { id: '1', workspace_id: 1 },
      { id: '2', workspace_id: 2 },
    ])
    const firstPage = deferred<chatApi.TranscriptMessageListResponse>()
    const secondPage = deferred<chatApi.TranscriptMessageListResponse>()
    const pages = spyOn(chatApi, 'listMessages').mockImplementation(async (sid) =>
      sid === '1' ? firstPage.promise : secondPage.promise,
    )
    spyOn(chatApi, 'getWorkspace').mockImplementation(async (id) => ({ id: Number(id), path: '/workspace' }))
    spyOn(chatApi, 'getSessionExecutionStatus').mockImplementation(async (sid) => {
      const value = execution(12)
      value.session.id = Number(sid)
      return status(value)
    })
    const first = chat.selectSession('1')
    await until(() => pages.mock.calls.some(([sid]) => sid === '1'))
    const second = chat.selectSession('2')
    await until(() => pages.mock.calls.some(([sid]) => sid === '2'))
    expect(chat.messagesLoading).toBe(true)
    firstPage.resolve({ entries: [], userMessageCount: 0, hasMore: false })
    await first
    expect(chat.selectedSessionId).toBe('2')
    expect(chat.messagesLoading).toBe(true)
    secondPage.resolve({ entries: [], userMessageCount: 0, hasMore: false })
    await second
    expect(chat.messagesLoading).toBe(false)
  })

  test('in-place rewind reloads the transcript with exact replacement and drops retracted parts', async () => {
    const chat = useChatStoreUnscoped(pinia)
    chat.selectedSessionId = '1'
    const retracted = part({
      part_id: 22,
      run_id: 22,
      message_id: 22,
      role: 'user',
      kind: 'text',
      state: 'completed',
      run_state: 'completed',
      content: { document: [], execution_phase: 'succeeded' },
    })
    const surviving = part({ part_id: 10, run_id: 10, role: 'assistant' })
    for (const snapshot of [surviving, retracted]) {
      chat.applyEvent({
        type: 'session_changed',
        properties: { session_id: 1, kind: 'part_added', part: snapshot, resource_revisions: {} },
      })
    }
    expect(chat.getMessagesForSession('1').map((entry) => entry.info.id)).toEqual(['10', '22'])

    const rewound = execution(13)
    rewound.session.id = 1
    spyOn(chatApi, 'rewindSession').mockResolvedValue(rewound)
    spyOn(chatApi, 'listSessionMessages').mockResolvedValue([
      {
        info: { id: '22', sessionID: '1', role: 'user', runId: 22, runState: 'completed' },
        parts: [],
      },
    ])
    const listed = spyOn(chatApi, 'listMessages').mockResolvedValue({
      entries: [
        {
          info: { id: '10', sessionID: '1', role: 'assistant', runId: 10, runState: 'completed' },
          parts: [],
        },
      ],
      hasMore: false,
      nextCursor: null,
      userMessageCount: 1,
    })

    const result = await chat.revertToMessage('1', '22', 'in_place')
    expect(result?.session.id).toBe('1')
    expect(listed).toHaveBeenCalled()
    // The retracted turn never comes back from the old cache.
    expect(chat.getMessagesForSession('1').map((entry) => entry.info.id)).toEqual(['10'])
  })

  test('fork rewind creates a branch session without rewriting the source', async () => {
    const chat = useChatStoreUnscoped(pinia)
    chat.selectedSessionId = '1'
    const source = part({ part_id: 10, run_id: 10, role: 'assistant' })
    chat.applyEvent({
      type: 'session_changed',
      properties: { session_id: 1, kind: 'part_added', part: source, resource_revisions: {} },
    })
    const created = execution(14)
    created.session.id = 2
    created.session.parent_id = 1
    created.session.relation_kind = 'rewind'
    const rewind = spyOn(chatApi, 'rewindSession').mockResolvedValue(created)
    spyOn(chatApi, 'listSessionMessages').mockResolvedValue([
      {
        info: { id: '22', sessionID: '1', role: 'user', runId: 22, runState: 'completed' },
        parts: [],
      },
    ])
    const listed = spyOn(chatApi, 'listMessages')

    const result = await chat.revertToMessage('1', '22', 'fork')
    expect(rewind).toHaveBeenCalledWith('1', 22, 'fork')
    expect(result?.session.id).toBe('2')
    expect(listed).not.toHaveBeenCalled()
    expect(chat.getMessagesForSession('1').map((entry) => entry.info.id)).toEqual(['10'])
  })
})
