import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { computed, createApp, ref } from 'vue'
import { createRevalidator } from '../src/lib/revalidation'
import { ensureBrowserTestRuntime } from './testRuntime'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
after(() => vite.close())
const { useChatStore } = (await vite.ssrLoadModule('/src/stores/chat.ts')) as typeof import('../src/stores/chat')
const { workspacePaneContextKey } = (await vite.ssrLoadModule(
  '/src/app/workspace/workspacePaneContext.ts',
)) as typeof import('../src/app/workspace/workspacePaneContext')
const pause = (ms = 0) => new Promise<void>((resolve) => setTimeout(resolve, ms))
const deferred = <T>() => {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
async function until(check: () => boolean) {
  for (let i = 0; i < 400; i++) {
    if (check()) return
    await pause(10)
  }
  assert.ok(check(), 'condition did not converge')
}
const marker = (id = 1) => ({
  part_id: id,
  kind: 'run',
  role: 'assistant',
  state: 'in_progress',
  content: {},
  revision: 1,
  updated_at_ms: 1,
})
const part = (revision = 1, text = 'old', id = 2) => ({
  part_id: id,
  run_id: 1,
  kind: 'text',
  role: 'assistant',
  state: 'in_progress',
  content: { text },
  revision,
  updated_at_ms: revision,
})
const page = (parts: unknown[], version = 1) => ({ parts, version, user_message_count: 0, page: { has_more: false } })
const state = (id = 7, version = 1) => ({
  session: { id, version, title: 'Session', state: { kind: 'ready', data: {} } },
  execution: {},
  parts: [],
})

async function withChat(run: (chat: ReturnType<typeof useChatStore>) => Promise<void>) {
  ensureBrowserTestRuntime()
  const pinia = createPinia()
  setActivePinia(pinia)
  const original = globalThis.fetch
  globalThis.fetch = (async (url) =>
    String(url).includes('/state') ? Response.json(state()) : Response.json({ items: [], page: {} })) as typeof fetch
  const chat = useChatStore()
  chat.retainSession('7')
  try {
    await run(chat)
  } finally {
    disposePinia(pinia)
    globalThis.fetch = original
  }
}

test('execution state follows its own workspace pane instead of the globally selected session', async () =>
  withChat(async (chat) => {
    chat.cacheSessions([
      { id: '7', version: 10, state: { kind: 'ready', data: {} } },
      { id: '8', version: 10, state: { kind: 'running', data: { workflow: 'tool_pending' } } },
    ])
    chat.selectedSessionId = '7'
    const sessionId = ref('8')
    const app = createApp({ render: () => null })
    app.provide(workspacePaneContextKey, {
      windowId: computed(() => 'pane-test'),
      isFocused: computed(() => false),
      route: computed(() => ({
        query: { sessionId: sessionId.value },
      })) as import('../src/app/workspace/workspacePaneContext').WorkspacePaneContext['route'],
      navigate: async () => {},
    })
    const pane = app.runWithContext(() => useChatStore())
    assert.equal(chat.selectedSessionState.kind, 'ready')
    assert.equal(pane.selectedSessionState.kind, 'running')
    sessionId.value = '7'
    assert.equal(pane.selectedSessionState.kind, 'ready')
    sessionId.value = '9'
    assert.equal(pane.selectedSessionState.kind, 'ready')
  }))

test('invalidation during a read produces one trailing read, without a sliding debounce', async () => {
  let calls = 0
  const first = deferred<void>()
  const queue = createRevalidator(
    async () => {
      calls++
      if (calls === 1) await first.promise
    },
    { intervalMs: 10 },
  )
  try {
    const reading = queue.refresh()
    await pause()
    for (let i = 0; i < 100; i++) queue.invalidate()
    assert.equal(calls, 1)
    first.resolve()
    await reading
    await until(() => calls === 2)
    await pause(20)
    assert.equal(calls, 2)
  } finally {
    queue.dispose()
  }
})

test('an older HTTP snapshot or live patch cannot roll text back, including buffered updates', async () =>
  withChat(async (chat) => {
    const pending = deferred<Response>()
    globalThis.fetch = (async (url) =>
      String(url).includes('/transcript') ? pending.promise : Response.json(state())) as typeof fetch
    const reading = chat.refreshMessages('7')
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: 7, part: marker() } })
    chat.applyEvent({
      type: 'session_changed',
      properties: { kind: 'part_updated', session_id: 7, part: part(4, 'new') },
    })
    pending.resolve(Response.json(page([marker(), part(1)])))
    await reading
    assert.equal(chat.getMessagesForSession('7')[0]?.parts[0]?.text, 'new')
    chat.applyEvent({
      type: 'session_changed',
      properties: { kind: 'part_updated', session_id: 7, part: part(2, 'stale') },
    })
    assert.equal(chat.getMessagesForSession('7')[0]?.parts[0]?.text, 'new')
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_updated', session_id: 7, part: part(5, '') } })
    assert.equal(
      chat.getMessagesForSession('7')[0]?.parts[0]?.text,
      '',
      'empty authoritative text clears the previous body',
    )
  }))

test('background activity events update the parent immediately and a pending state read cannot undo progress or dismissal', async () =>
  withChat(async (chat) => {
    const pending = deferred<Response>()
    globalThis.fetch = (async () => pending.promise) as typeof fetch
    const reading = chat.refreshExecutionStatus('7')
    const activity = {
      id: 'task_a', kind: 'task', status: 'running', title: 'Child task', description: '',
      session_id: 8, parent_session_id: 7, last_seq: 1, controls: ['stop'],
    }
    const event = (value: typeof activity, reason = 'updated') => chat.applyEvent({
      type: 'runtime_signal', properties: {
        kind: 'activity', session_id: 7, payload: { activity_id: value.id, reason, activity: value },
      },
    })
    event(activity)
    assert.equal(chat.sessionBackgroundActivities('7')[0]?.last_seq, 1)
    assert.equal(chat.sessionBackgroundActivityKinds('7').join(','), 'task')
    assert.equal(chat.sessionBackgroundActivities('8').length, 0)
    event({ ...activity, last_seq: 4 })
    assert.equal(chat.sessionBackgroundActivities('7')[0]?.last_seq, 4)
    pending.resolve(Response.json({ ...state(), background_activities: [activity] }))
    await reading
    assert.equal(chat.sessionBackgroundActivities('7')[0]?.last_seq, 4)

    const dismissedRead = deferred<Response>()
    globalThis.fetch = (async () => dismissedRead.promise) as typeof fetch
    const refreshing = chat.refreshExecutionStatus('7')
    event({ ...activity, status: 'succeeded', last_seq: 5, controls: ['dismiss'] })
    event({ ...activity, status: 'succeeded', last_seq: 5, controls: ['dismiss'] }, 'dismissed')
    assert.equal(chat.sessionBackgroundActivities('7').length, 0)
    assert.equal(chat.sessionBackgroundActivityKinds('7').length, 0)
    dismissedRead.resolve(Response.json({ ...state(), background_activities: [activity] }))
    await refreshing
    assert.equal(chat.sessionBackgroundActivities('7').length, 0)
  }))

test('removed memberships and deleted sessions cannot be resurrected by pending reads', async () =>
  withChat(async (chat) => {
    const pending = deferred<Response>()
    globalThis.fetch = (async (url) =>
      String(url).includes('/transcript') ? pending.promise : Response.json(state())) as typeof fetch
    const reading = chat.refreshMessages('7')
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_removed', session_id: 7, part_id: 2 } })
    pending.resolve(Response.json(page([marker(), part()])))
    await reading
    assert.equal(chat.getMessagesForSession('7')[0]?.parts.length, 0)
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'session_deleted', session_id: 7 } })
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: 7, part: marker() } })
    assert.equal(chat.getMessagesForSession('7').length, 0)
    assert.equal(chat.getSessionById('7'), null)
  }))

test('a send acknowledgement forces a fresh transcript even when a pre-send read is running and SSE is lost', async () =>
  withChat(async (chat) => {
    const pending = deferred<Response>()
    let reads = 0
    globalThis.fetch = (async (url) => {
      if (String(url).includes('/transcript'))
        return ++reads === 1 ? pending.promise : Response.json(page([marker(), part(2, 'accepted')]))
      return Response.json(state())
    }) as typeof fetch
    const reading = chat.refreshMessages('7')
    await chat.sendMessage('7', { text: 'hello' })
    await pause(20)
    assert.equal(reads, 1)
    pending.resolve(Response.json(page([])))
    await reading
    await until(() => chat.getMessagesForSession('7')[0]?.parts[0]?.text === 'accepted')
    assert.equal(reads, 2)
  }))

test('reconnect reconciles every mounted conversation and removes missed deletions using only loaded ids', async () =>
  withChat(async (chat) => {
    for (const sid of [7, 8]) {
      chat.retainSession(String(sid))
      chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: sid, part: marker() } })
      chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: sid, part: part() } })
    }
    const reconciled: string[] = []
    globalThis.fetch = (async (url) => {
      const parsed = new URL(String(url), 'http://agena.test')
      const sid = Number(parsed.pathname.match(/sessions\/(\d+)/)?.[1] || 7)
      if (parsed.searchParams.has('ids')) {
        reconciled.push(String(sid))
        assert.equal(parsed.searchParams.get('ids'), '1,2')
        return Response.json(page([marker()]))
      }
      if (parsed.pathname.endsWith('/transcript')) return Response.json(page([marker()]))
      return Response.json(state(sid))
    }) as typeof fetch
    chat.reconcileLiveState()
    await until(() => reconciled.length === 2 && chat.getMessagesForSession('8')[0]?.parts.length === 0)
    assert.deepEqual(reconciled.sort(), ['7', '8'])
    assert.equal(chat.getMessagesForSession('7')[0]?.parts.length, 0)
  }))

test('session caches reject an older list or execution version', async () =>
  withChat(async (chat) => {
    chat.cacheSessions([{ id: '7', version: 9, title: 'latest', state: { kind: 'ready', data: {} } }])
    globalThis.fetch = (async (url) =>
      String(url).includes('/transcript')
        ? Response.json(page([]))
        : String(url).includes('/state')
          ? Response.json(state(7, 1))
          : Response.json({ items: [state(7, 1).session], page: {} })) as typeof fetch
    await chat.refreshSessions()
    await chat.refreshMessages('7')
    await pause()
    assert.equal(chat.getSessionById('7')?.version, 9)
    assert.equal(chat.getSessionById('7')?.title, 'latest')
  }))

test('session-list invalidation during a foreground read gets a trailing current list', async () =>
  withChat(async (chat) => {
    const pending = deferred<Response>()
    let lists = 0
    globalThis.fetch = (async (url) => {
      const path = new URL(String(url), 'http://agena.test').pathname
      if (path.endsWith('/sessions')) {
        lists++
        return lists === 1 ? pending.promise : Response.json({ items: [state(8, 2).session], page: {} })
      }
      return Response.json(state())
    }) as typeof fetch
    const reading = chat.refreshSessions()
    chat.reconcileLiveState()
    await pause(20)
    pending.resolve(Response.json({ items: [], page: {} }))
    await reading
    await until(() => chat.getSessionById('8') !== null)
    assert.equal(lists, 2)
  }))

test('shared part updates reach loaded fork memberships and complete run snapshots clear obsolete fields', async () =>
  withChat(async (chat) => {
    for (const sid of [7, 8]) {
      chat.retainSession(String(sid))
      chat.applyEvent({
        type: 'session_changed',
        properties: { kind: 'part_added', session_id: sid, part: { ...marker(), state: 'completed' } },
      })
      chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: sid, part: part() } })
    }
    chat.applyEvent({
      type: 'session_changed',
      properties: { kind: 'part_updated', session_id: 7, part: part(4, 'shared') },
    })
    assert.equal(chat.getMessagesForSession('8')[0]?.parts[0]?.text, 'shared')
    chat.applyEvent({
      type: 'session_changed',
      properties: { kind: 'part_updated', session_id: 7, part: { ...marker(), revision: 5 } },
    })
    assert.equal(chat.getMessagesForSession('8')[0]?.info.finish, undefined)
  }))

test('a missed session deletion is reconciled by a 404 state read and stops status retrying', async () =>
  withChat(async (chat) => {
    chat.cacheSessions([{ id: '7', version: 1, title: 'deleted', state: { kind: 'ready', data: {} } }])
    globalThis.fetch = (async () => new Response('{}', { status: 404 })) as typeof fetch
    chat.retainSession('7')
    chat.reconcileLiveState()
    await until(() => chat.getSessionById('7') === null)
  }))

test('activity invalidation waits for a pre-reconnect read and then obtains a current snapshot', async () =>
  withChat(async () => {
    const { useSessionActivityStore } = (await vite.ssrLoadModule(
      '/src/stores/sessionActivity.ts',
    )) as typeof import('../src/stores/sessionActivity')
    const activity = useSessionActivityStore()
    const pending = deferred<Response>()
    let reads = 0
    globalThis.fetch = (async () => (++reads === 1 ? pending.promise : Response.json([]))) as typeof fetch
    const reading = activity.refresh()
    activity.invalidate()
    pending.resolve(Response.json([{ id: 'a', session_id: 7, status: 'running', kind: 'shell' }]))
    await reading
    await until(() => reads === 2 && Object.keys(activity.snapshot).length === 0)
    assert.equal(reads, 2)
  }))

test('global live events do not materialize unopened session histories', async () =>
  withChat(async (chat) => {
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: 99, part: marker() } })
    chat.applyEvent({
      type: 'session_changed',
      properties: { kind: 'part_updated', session_id: 99, part: part(4, 'background') },
    })
    assert.equal(chat.getMessagesForSession('99').length, 0)
  }))
