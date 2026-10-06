import { afterAll, expect, test } from 'bun:test'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { createRenderer, defineComponent, nextTick, ref } from 'vue'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { ensureBrowserTestRuntime } from './testRuntime'
import type { SessionActivity } from '../src/types/activity'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())
const { useWorkspaceChanges } = (await vite.ssrLoadModule(
  '/src/pages/chat/useWorkspaceChanges.ts',
)) as typeof import('../src/pages/chat/useWorkspaceChanges')
const { useActivityLogs } = (await vite.ssrLoadModule(
  '/src/composables/useActivityLogs.ts',
)) as typeof import('../src/composables/useActivityLogs')
const { useChatStore } = (await vite.ssrLoadModule('/src/stores/chat.ts')) as typeof import('../src/stores/chat')
const { useSessionActivityStore } = (await vite.ssrLoadModule(
  '/src/stores/sessionActivity.ts',
)) as typeof import('../src/stores/sessionActivity')
const { useSessionPlan } = (await vite.ssrLoadModule(
  '/src/pages/chat/useSessionPlan.ts',
)) as typeof import('../src/pages/chat/useSessionPlan')
const sync = (await vite.ssrLoadModule('/src/lib/resourceSync.ts')) as typeof import('../src/lib/resourceSync')
const { conditionalJson, conditionalJsonObserved } = (await vite.ssrLoadModule(
  '/src/lib/conditionalJson.ts',
)) as typeof import('../src/lib/conditionalJson')

async function withRuntime(
  run: (context: {
    mount: (setup: () => void) => void
    advance: (ms: number) => Promise<void>
    doc: EventTarget & { hidden: boolean; visibilityState: string }
    tokens: Map<string, string>
    calls: URL[]
    reply: (key: string, body: unknown) => Response
    evict: () => Promise<void>
    setFetch: (handler: (url: URL) => Response) => void
  }) => Promise<void>,
) {
  ensureBrowserTestRuntime()
  const original = {
    document: globalThis.document,
    fetch: globalThis.fetch,
    now: Date.now,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    windowSetTimeout: window.setTimeout,
    windowClearTimeout: window.clearTimeout,
  }
  let now = 100_000,
    sequence = 0
  const timers = new Map<number, { at: number; callback: () => void }>()
  const timeout = ((callback: () => void, ms = 0) => {
    const id = ++sequence
    timers.set(id, { at: now + ms, callback })
    return id
  }) as unknown as typeof setTimeout
  const clear = ((id: number) => timers.delete(id)) as unknown as typeof clearTimeout
  const doc = Object.assign(new EventTarget(), { hidden: false, visibilityState: 'visible' })
  Object.assign(globalThis, { document: doc, setTimeout: timeout, clearTimeout: clear })
  window.setTimeout = timeout as typeof window.setTimeout
  window.clearTimeout = clear as typeof window.clearTimeout
  Date.now = () => now
  const pinia = createPinia()
  setActivePinia(pinia)
  const renderer = createRenderer<Record<string, never>, Record<string, never>>({
    createElement: () => ({}),
    createText: () => ({}),
    createComment: () => ({}),
    insert() {},
    remove() {},
    setText() {},
    setElementText() {},
    patchProp() {},
    parentNode: () => null,
    nextSibling: () => null,
  })
  const apps: Array<ReturnType<typeof renderer.createApp>> = []
  const mount = (setup: () => void) => {
    const app = renderer.createApp(
      defineComponent({
        setup() {
          setup()
          return () => null
        },
      }),
    )
    apps.push(app)
    app.mount({})
  }
  const settle = async () => {
    for (let i = 0; i < 6; i++) {
      await new Promise<void>((resolve) => setImmediate(resolve))
      await nextTick()
    }
  }
  const advance = async (ms: number) => {
    const end = now + ms
    for (let count = 0; count < 300; count++) {
      await settle()
      const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
      if (!next) {
        now = end
        await settle()
        return
      }
      now = next[1].at
      timers.delete(next[0])
      next[1].callback()
    }
    throw new Error('timers did not settle')
  }
  const tokens = new Map<string, string>(),
    calls: URL[] = []
  const reply = (key: string, body: unknown) =>
    Response.json(body, {
      headers: { etag: `W/"${tokens.get(key) ?? 'granularity-http:1'}"` },
    })
  const setFetch = (handler: (url: URL) => Response) => {
    globalThis.fetch = (async (input) => {
      const url = new URL(String(input), 'http://agena.test')
      calls.push(url)
      if (url.pathname === '/api/v1/changes/revisions')
        return Response.json(
          Object.fromEntries(
            (JSON.parse(url.searchParams.get('resources')!) as string[]).map((key) => [
              key,
              tokens.get(key) ?? 'granularity-http:1',
            ]),
          ),
        )
      if (url.pathname.startsWith('/evict-granularity/')) return reply('sessions', {})
      return handler(url)
    }) as typeof fetch
  }
  let eviction = 0
  const evict = () =>
    Promise.all(
      Array.from({ length: 90 }, (_, i) => conditionalJson('sessions', `/evict-granularity/${eviction++}-${i}`)),
    ).then(() => {})
  try {
    await run({ mount, advance, doc, tokens, calls, reply, evict, setFetch })
  } finally {
    for (const app of apps) app.unmount()
    disposePinia(pinia)
    Date.now = original.now
    Object.assign(globalThis, {
      document: original.document,
      fetch: original.fetch,
      setTimeout: original.setTimeout,
      clearTimeout: original.clearTimeout,
    })
    window.setTimeout = original.windowSetTimeout
    window.clearTimeout = original.windowClearTimeout
  }
}

test('a selected file keeps its diff through other-file changes, eviction, hide/show and reopening', () =>
  withRuntime(async ({ mount, advance, doc, tokens, calls, reply, evict, setFetch }) => {
    const resource = 'session:9951:files'
    tokens.set(resource, 'granularity-http:10')
    let a = 'a1',
      b = 'b1'
    setFetch((url) => {
      expect(url.pathname).toBe('/api/v1/sessions/9951/file-changes')
      const path = url.searchParams.get('path')
      const files = ['a.ts', 'b.ts']
        .filter((p) => !path || p === path)
        .map((path) => ({
          path,
          revision: path === 'a.ts' ? a : b,
          operation_count: 1,
          operation_history: false,
          operations: path
            ? [{ part_id: 1, tool: 'fs.write', kind: 'created', diff: `+${a}`, diff_scope: 'file' }]
            : [],
        }))
      return reply(resource, {
        files: url.searchParams.get('summary') === 'true' ? [] : files,
        total_files: 2,
        offset: 0,
        has_more: false,
        recording_incomplete: false,
      })
    })
    let state!: ReturnType<typeof useWorkspaceChanges>
    mount(() => {
      state = useWorkspaceChanges({ sessionId: ref('9951'), busy: ref(false) })
    })
    await advance(1000)
    state.expanded.value = true
    await advance(1000)
    state.select(state.files.value[0]!)
    await advance(1000)
    expect(state.visibleDiff.value?.files[0]?.operations[0]?.diff).toBe('+a1')
    await evict()
    calls.length = 0
    b = 'b2'
    tokens.set(resource, 'granularity-http:11')
    sync.applyResourceEvent({
      type: 'session_changed',
      properties: { resource_revisions: { [resource]: tokens.get(resource)! } },
    })
    await advance(1000)
    expect(calls.filter((url) => url.searchParams.has('path')).length).toBe(0)
    expect(calls.filter((url) => url.pathname.endsWith('/file-changes')).length).toBe(1)
    doc.hidden = true
    doc.visibilityState = 'hidden'
    doc.dispatchEvent(new Event('visibilitychange'))
    doc.hidden = false
    doc.visibilityState = 'visible'
    doc.dispatchEvent(new Event('visibilitychange'))
    await advance(1000)
    state.select(state.files.value[0]!)
    state.select(state.files.value[0]!)
    await advance(1000)
    expect(calls.filter((url) => url.searchParams.has('path')).length).toBe(0)
    calls.length = 0
    a = 'a2'
    tokens.set(resource, 'granularity-http:12')
    sync.applyResourceEvent({
      type: 'session_changed',
      properties: { resource_revisions: { [resource]: tokens.get(resource)! } },
    })
    await advance(1000)
    expect(calls.filter((url) => url.pathname.endsWith('/file-changes')).length).toBe(2)
    expect(calls.filter((url) => url.searchParams.get('path') === 'a.ts').length).toBe(1)
    expect(state.visibleDiff.value?.files[0]?.operations[0]?.diff).toBe('+a2')
  }))

test('activity descriptors never read log bodies; same-cursor log updates replace only that line', () =>
  withRuntime(async ({ mount, advance, tokens, calls, reply, evict, setFetch }) => {
    const id = 'task_granularity',
      resource = `activity:${id}:logs`
    tokens.set(resource, 'granularity-http:20')
    let text = 'first fragment'
    setFetch((url) => {
      expect(url.pathname).toBe(`/api/v1/activities/${id}/logs`)
      return reply(resource, {
        activity_id: id,
        status: 'running',
        last_seq: 1,
        has_more: false,
        dropped_lines: 0,
        lines: [{ seq: 1, stream: 'message', text }],
      })
    })
    let state!: ReturnType<typeof useActivityLogs>
    mount(() => {
      state = useActivityLogs(ref(`7/${id}`), () => ({ id, last_seq: 1 }) as SessionActivity)
    })
    await advance(1000)
    await evict()
    calls.length = 0
    sync.applyResourceEvent({
      type: 'runtime_signal',
      properties: {
        kind: 'activity',
        resource_revisions: {
          [`activity:${id}`]: 'granularity-http:21',
          [resource]: 'granularity-http:20',
        },
      },
    })
    await advance(1000)
    expect(calls.length).toBe(0)
    text = 'full streamed answer'
    tokens.set(resource, 'granularity-http:22')
    sync.applyResourceEvent({
      type: 'runtime_signal',
      properties: {
        kind: 'activity',
        payload: { reason: 'logs_changed' },
        resource_revisions: { [resource]: tokens.get(resource)! },
      },
    })
    await advance(1000)
    expect(calls.length).toBe(1)
    expect(calls[0]!.searchParams.get('since_seq')).toBe('1')
    expect(state.data.value!.lines).toHaveLength(1)
    expect(state.data.value!.lines[0]!.text).toBe('full streamed answer')
  }))

test('unchanged mounted conversations and activity list recover with revisions and zero bodies after eviction', () =>
  withRuntime(async ({ advance, tokens, calls, reply, evict, setFetch }) => {
    for (const id of [9961, 9962])
      for (const suffix of ['state', 'transcript']) tokens.set(`session:${id}:${suffix}`, 'granularity-http:30')
    tokens.set('activities', 'granularity-http:30')
    let activityRows: Array<{ id: string; kind: string; status: string; session_id: number }> = []
    setFetch((url) => {
      const sid = Number(url.pathname.match(/sessions\/(\d+)/)?.[1])
      if (url.pathname.endsWith('/transcript'))
        return reply(`session:${sid}:transcript`, {
          session_id: sid,
          version: 1,
          parts: [],
          user_message_count: 0,
          page: { has_more: false },
        })
      if (url.pathname.endsWith('/state'))
        return reply(`session:${sid}:state`, {
          session: { id: sid, version: 1, workspace_id: 1, title: 'quiet', state: { kind: 'ready' } },
          parts: [],
        })
      if (url.pathname === '/api/v1/activities') return reply('activities', activityRows)
      throw new Error(`Unexpected request ${url}`)
    })
    const chat = useChatStore(),
      activity = useSessionActivityStore()
    const releaseA = chat.retainSession('9961'),
      releaseB = chat.retainSession('9962')
    try {
      await chat.refreshMessages('9961')
      await chat.refreshMessages('9962')
      await activity.refresh()
      await advance(3000)
      await evict()
      calls.length = 0
      chat.reconcileLiveState()
      sync.invalidateResources(['activities'])
      activity.invalidate()
      await advance(3000)
      expect(calls.length).toBeGreaterThan(0)
      expect(calls.every((url) => url.pathname === '/api/v1/changes/revisions')).toBe(true)
      calls.length = 0
      tokens.set('session:9961:transcript', 'granularity-http:31')
      chat.reconcileLiveState()
      await advance(3000)
      expect(calls.filter((url) => url.pathname.endsWith('/transcript')).map((url) => url.pathname)).toEqual([
        '/api/v1/sessions/9961/transcript',
      ])
      expect(calls.some((url) => url.pathname.includes('/9962/') || url.pathname.endsWith('/state'))).toBe(false)
      calls.length = 0
      // A metadata probe discovers a missed record, then a newer unrelated
      // descriptor arrives before the scheduled list read. That one record
      // must not certify the whole retained list as synchronized.
      activityRows = [
        { id: 'task_A', kind: 'task', status: 'running', session_id: 9961 },
        { id: 'task_B', kind: 'task', status: 'running', session_id: 9962 },
      ]
      tokens.set('activities', 'granularity-http:32')
      sync.noteResourceVersion('activities', 'granularity-http:31', sync.captureResourceObservation('activities'))
      const event = {
        type: 'runtime_signal',
        properties: {
          kind: 'activity',
          resource_revisions: {
            activities: 'granularity-http:32',
          },
          payload: { activity: activityRows[0]!, reason: 'started', ts_ms: 1 },
        },
      }
      sync.applyResourceEvent(event)
      activity.applyEvent(event)
      await advance(3000)
      expect(calls.filter((url) => url.pathname === '/api/v1/activities')).toHaveLength(1)
      expect(activity.snapshot['9962']?.type).toBe('busy')
    } finally {
      releaseA()
      releaseB()
    }
  }))

test('a displayed plan survives body eviction and foreground recovery without rereading an unchanged document', () =>
  withRuntime(async ({ mount, advance, doc, tokens, calls, reply, evict, setFetch }) => {
    const resource = 'session:9975:plan'
    tokens.set(resource, 'granularity-http:40')
    setFetch((url) => {
      expect(url.pathname).toBe('/api/v1/sessions/9975/plan')
      return reply(resource, {
        output_text: `Revision: v1\n# ${tokens.get(resource)}`,
        payload: { plan: { title: tokens.get(resource), autorun: false } },
      })
    })
    let state!: ReturnType<typeof useSessionPlan>
    mount(() => {
      state = useSessionPlan(
        () => '9975',
        () => false,
        () => 0,
        async (_id, _tool, _input, signal, read) => {
          const { value, observation } = await conditionalJsonObserved<
            Record<string, import('../src/types/json').JsonValue>
          >(resource, '/api/v1/sessions/9975/plan', { signal }, read?.force)
          read?.observe(observation)
          return value
        },
      )
    })
    await advance(1500)
    expect(calls.map((url) => url.pathname)).toEqual(['/api/v1/sessions/9975/plan'])
    await evict()
    calls.length = 0
    doc.hidden = true
    doc.visibilityState = 'hidden'
    doc.dispatchEvent(new Event('visibilitychange'))
    await advance(1000)
    doc.hidden = false
    doc.visibilityState = 'visible'
    doc.dispatchEvent(new Event('visibilitychange'))
    await advance(2000)
    expect(calls.length).toBe(0)
    expect(state.viewer.markdown.value).toBe('# granularity-http:40')
    tokens.set(resource, 'granularity-http:41')
    sync.applyResourceEvent({
      type: 'runtime_signal',
      properties: {
        kind: 'plugin',
        payload: { kind: 'plan.changed' },
        resource_revisions: { [resource]: tokens.get(resource)! },
      },
    })
    await advance(2000)
    expect(calls.map((url) => url.pathname)).toEqual(['/api/v1/sessions/9975/plan'])
    expect(state.viewer.markdown.value).toBe('# granularity-http:41')
    calls.length = 0
    await state.viewer.refresh()
    expect(calls.map((url) => url.pathname)).toEqual(['/api/v1/sessions/9975/plan'])
  }))

test('preview registry identities cannot invalidate or retire core API versions', () =>
  withRuntime(async () => {
    const apiKey = 'session:9971:state'
    sync.noteResourceVersion(apiKey, 'independent-api:1')
    const apiObservation = sync.captureResourceObservation(apiKey)
    expect(sync.noteResourceVersion('preview', 'independent-preview:1')).toBe(true)
    expect(sync.canReuseResource(apiKey, 'independent-api:1')).toBe(true)
    expect(sync.noteResourceVersion(apiKey, 'independent-api:2', apiObservation)).toBe(true)
    const previewObservation = sync.captureResourceObservation('preview')
    expect(sync.noteResourceVersion('preview', 'restarted-preview:1', previewObservation)).toBe(true)
    expect(sync.noteResourceVersion('preview', 'restarted-preview:1', sync.captureResourceObservation('preview'))).toBe(
      true,
    )
    expect(sync.canReuseResource(apiKey, 'independent-api:2')).toBe(true)
    expect(sync.noteResourceVersion(apiKey, 'independent-api:3')).toBe(true)
    expect(sync.noteResourceVersion('preview', 'independent-preview:2')).toBe(false)
    expect(sync.canReuseResource(apiKey, 'independent-api:3')).toBe(true)
    const beforeRestart = sync.captureResourceObservation(apiKey)
    expect(sync.noteResourceVersion(apiKey, 'restarted-api:1', beforeRestart)).toBe(true)
    expect(sync.noteResourceVersion(apiKey, 'independent-api:4')).toBe(false)
    expect(sync.canReuseResource('preview', 'restarted-preview:1')).toBe(true)
  }))
