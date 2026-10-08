import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { createServer } from 'vite'
import { effectScope, nextTick, ref } from 'vue'
import { fileURLToPath } from 'node:url'
import { createPinia, setActivePinia } from 'pinia'
import { createMermaidRenderer } from '../src/lib/mermaidRenderer'
import { ensureBrowserTestRuntime } from './testRuntime'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
after(() => vite.close())
const { renderMarkdown } = (await vite.ssrLoadModule('/src/lib/markdown.ts')) as typeof import('../src/lib/markdown')
const { createRequestLimiter, loadSidebarSessionPage } = (await vite.ssrLoadModule(
  '/src/stores/chat/sidebarPaging.ts',
)) as typeof import('../src/stores/chat/sidebarPaging')
const { useSessionActivityStore } = (await vite.ssrLoadModule(
  '/src/stores/sessionActivity.ts',
)) as typeof import('../src/stores/sessionActivity')
const { useWorkspacePreviewStore } = (await vite.ssrLoadModule(
  '/src/stores/workspacePreview.ts',
)) as typeof import('../src/stores/workspacePreview')

const { useDirectorySessionStore } = (await vite.ssrLoadModule(
  '/src/stores/directorySessionStore.ts',
)) as typeof import('../src/stores/directorySessionStore')
const resourceSync = (await vite.ssrLoadModule('/src/lib/resourceSync.ts')) as typeof import('../src/lib/resourceSync')
const { renderMarkdownAsync } = (await vite.ssrLoadModule(
  '/src/lib/markdownAsync.ts',
)) as typeof import('../src/lib/markdownAsync')
const { useChatAttachments } = (await vite.ssrLoadModule(
  '/src/pages/chat/useChatAttachments.ts',
)) as typeof import('../src/pages/chat/useChatAttachments')

test('Markdown worker serializes parses, cancels queued bodies, and recovers after failure or an over-budget parse', async () => {
  const original = globalThis.Worker
  const workers: Array<{
    onmessage?: (event: unknown) => void
    onerror?: () => void
    sent: Array<{ id: number; content: string }>
    terminated: boolean
  }> = []
  globalThis.Worker = class {
    onmessage?: (event: unknown) => void
    onerror?: () => void
    sent: Array<{ id: number; content: string }> = []
    terminated = false
    constructor() {
      workers.push(this)
    }
    postMessage(value: { id: number; content: string }) {
      this.sent.push(value)
    }
    terminate() {
      this.terminated = true
    }
  } as unknown as typeof Worker
  try {
    const signal = new AbortController()
    const cancelled = new AbortController()
    const first = renderMarkdownAsync('first', {}, signal.signal)
    const ignored = renderMarkdownAsync('cancelled', {}, cancelled.signal)
    const rejection = assert.rejects(ignored, { name: 'AbortError' })
    const last = renderMarkdownAsync('last', {}, signal.signal)
    cancelled.abort()
    await rejection
    const worker = workers[0]!
    assert.deepEqual(
      worker.sent.map((work) => work.content),
      ['first'],
    )
    worker.onmessage?.({ data: { id: worker.sent[0]!.id, parts: ['<p>first</p>'] } })
    assert.deepEqual(await first, ['<p>first</p>'])
    assert.deepEqual(
      worker.sent.map((work) => work.content),
      ['first', 'last'],
    )
    worker.onmessage?.({ data: { id: worker.sent[1]!.id, parts: ['<p>last</p>'] } })
    assert.deepEqual(await last, ['<p>last</p>'])
    const failure = renderMarkdownAsync('failure', {}, signal.signal)
    const failed = assert.rejects(failure, /worker failed/)
    worker.onerror?.()
    await failed
    const retry = renderMarkdownAsync('retry', {}, signal.signal)
    const replacement = workers[1]!
    replacement.onmessage?.({ data: { id: replacement.sent[0]!.id, parts: ['<p>retry</p>'] } })
    assert.deepEqual(await retry, ['<p>retry</p>'])
    const slow = renderMarkdownAsync('  exact <source>\n\n', {}, signal.signal)
    const nextPart = renderMarkdownAsync('another Part', {}, signal.signal)
    assert.match((await slow).join(''), /  exact &lt;source&gt;\n\n/)
    assert.equal(replacement.terminated, true)
    const fresh = workers[2]!
    // A late result/error from the retired worker cannot touch its successor.
    replacement.onmessage?.({ data: { id: fresh.sent[0]!.id, parts: ['stale'] } })
    replacement.onerror?.()
    assert.equal(fresh.terminated, false)
    fresh.onmessage?.({ data: { id: fresh.sent[0]!.id, parts: ['<p>another Part</p>'] } })
    assert.deepEqual(await nextPart, ['<p>another Part</p>'])
    // Leave no fake worker in the shared renderer after this test.
    fresh.onerror?.()
  } finally {
    globalThis.Worker = original
  }
})

test('attachment URLs survive switching drafts and failed-send recovery, then release on discard and unmount', async () => {
  const create = URL.createObjectURL
  const revoke = URL.revokeObjectURL
  const released: string[] = []
  let sequence = 0
  let retained: string[] = []
  URL.createObjectURL = () => `blob:lifecycle-${++sequence}`
  URL.revokeObjectURL = (url: string) => {
    released.push(url)
  }
  const scope = effectScope()
  try {
    const attachments = scope.run(() =>
      useChatAttachments({ toasts: { push() {} }, composerRef: ref(null), retainedUrls: () => retained }),
    )!
    await attachments.handleDrop({
      preventDefault() {},
      dataTransfer: { files: [new File(['image'], 'draft.png')] },
    } as unknown as DragEvent)
    const saved = attachments.attachedFiles.value.slice()
    retained = saved.map((file) => file.url!)
    attachments.clearAttachments({ preserve: true })
    await nextTick()
    assert.deepEqual(released, [])
    attachments.attachedFiles.value = saved
    retained = []
    await nextTick()
    assert.deepEqual(released, [])
    // A submitted/failed snapshot owns the URLs even with an empty composer.
    retained = saved.map((file) => file.url!)
    attachments.attachedFiles.value = []
    await nextTick()
    assert.deepEqual(released, [])
    retained = []
    attachments.releaseUnusedAttachmentUrls()
    assert.deepEqual(released, ['blob:lifecycle-1'])
    await attachments.handleDrop({
      preventDefault() {},
      dataTransfer: { files: [new File(['new'], 'second.png')] },
    } as unknown as DragEvent)
    scope.stop()
    assert.deepEqual(released, ['blob:lifecycle-1', 'blob:lifecycle-2'])
  } finally {
    scope.stop()
    URL.createObjectURL = create
    URL.revokeObjectURL = revoke
  }
})

const deferred = <T>() => {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
const flush = () => new Promise<void>((resolve) => setImmediate(resolve))

test('Markdown keeps real image URLs inert until hydration and does not preload media', () => {
  const html = renderMarkdown('![plot](https://example.com/plot.png)\n\n![clip](https://example.com/clip.mp4)')
  assert.match(html, /data-oc-md-raw-src="https:\/\/example.com\/plot.png"/)
  assert.match(html, /src="data:image\/gif;base64,/)
  assert.doesNotMatch(html, /\ssrc="https:\/\/example.com\/plot.png"|preload="metadata"/)
  assert.match(html, /loading="lazy"/)
})

test('Mermaid shares initialization, serializes renderers, and skips cancelled queued work', async () => {
  let imports = 0
  const calls: string[] = []
  const themes: string[] = []
  const first = deferred<{ svg: string }>()
  const render = createMermaidRenderer(async () => {
    imports++
    return {
      initialize: ({ theme }) => {
        themes.push(theme)
      },
      render: async (id, source) => {
        calls.push(`${id}:${source}`)
        return source === 'first' ? first.promise : { svg: source }
      },
    }
  })
  const live = new AbortController()
  const cancelled = new AbortController()
  const a = render('first', 'dark', live.signal)
  const b = render('stale', 'neutral', cancelled.signal)
  const c = render('last', 'dark', live.signal)
  cancelled.abort()
  await flush()
  assert.deepEqual(calls, ['oc-mermaid-1:first'])
  first.resolve({ svg: 'first' })
  await Promise.all([a, b, c])
  assert.deepEqual(calls, ['oc-mermaid-1:first', 'oc-mermaid-2:last'])
  assert.deepEqual(themes, ['dark'])
  assert.equal(imports, 1)
})

test('a failed Mermaid import is retryable and does not poison the render queue', async () => {
  let attempts = 0
  const render = createMermaidRenderer(async () => {
    if (++attempts === 1) throw new Error('offline')
    return { initialize() {}, render: async () => 'svg' }
  })
  await assert.rejects(render('one', 'neutral', new AbortController().signal), /offline/)
  assert.equal(await render('two', 'neutral', new AbortController().signal), 'svg')
})

test('sidebar retrieves exactly the requested filtered page and recovers an empty last page', async () => {
  const calls: unknown[] = []
  const result = await loadSidebarSessionPage({ workspaceId: '7', bucket: 'favorite' }, 20, 10, async (options) => {
    calls.push(options)
    return { sessions: [], total: 35, hasMore: false, nextCursor: null }
  })
  assert.equal(result.page, 3)
  assert.equal(result.pageCount, 4)
  assert.deepEqual(
    calls,
    [200, 30].map((offset) => ({
      workspaceId: '7',
      bucket: 'favorite',
      limit: 10,
      offset,
      excludeSubagents: true,
      includeTotal: true,
    })),
  )
})

test('request limiter bounds concurrency and never starts cancelled queued requests', async () => {
  const limit = createRequestLimiter(2)
  const pending = deferred<void>()
  let active = 0
  let peak = 0
  let started = 0
  const work = () =>
    limit(async () => {
      peak = Math.max(peak, ++active)
      started++
      await pending.promise
      active--
    })
  const jobs = [work(), work(), work()]
  const cancelled = new AbortController()
  const ignored = limit(async () => {
    throw new Error('cancelled job started')
  }, cancelled.signal)
  cancelled.abort()
  const rejection = assert.rejects(ignored, { name: 'AbortError' })
  assert.equal(started, 2)
  pending.resolve()
  await Promise.all([...jobs, rejection])
  assert.equal(peak, 2)
  assert.equal(started, 3)
})

test('activity requests coalesce and a late snapshot cannot resurrect an idle session', async () => {
  ensureBrowserTestRuntime()
  setActivePinia(createPinia())
  const original = globalThis.fetch
  const pending = deferred<Response>()
  let requests = 0
  globalThis.fetch = (() => {
    requests++
    return pending.promise
  }) as typeof fetch
  const store = useSessionActivityStore()
  try {
    const first = store.refresh()
    const second = store.refresh()
    await flush()
    assert.equal(requests, 1)
    store.applyEvent({
      type: 'runtime_signal',
      properties: { kind: 'activity', session_id: 7, payload: { session_id: 7, kind: 'shell', status: 'completed' } },
    })
    pending.resolve(Response.json([{ session_id: 7, kind: 'shell', status: 'running' }]))
    await Promise.all([first, second])
    assert.equal(store.snapshot['7'], undefined)
  } finally {
    store.$dispose()
    globalThis.fetch = original
  }
})

test('preview polling coalesces and a mutation schedules one fresh read after the old request', async () => {
  ensureBrowserTestRuntime()
  setActivePinia(createPinia())
  const original = globalThis.fetch
  const requests: ReturnType<typeof deferred<Response>>[] = []
  globalThis.fetch = (() => {
    const request = deferred<Response>()
    requests.push(request)
    return request.promise
  }) as typeof fetch
  const store = useWorkspacePreviewStore()
  const session = {
    id: 'new',
    state: 'running',
    directory: '/repo',
    runDirectory: '/repo',
    proxyBasePath: '/preview/new',
    targetUrl: 'http://localhost:3000',
    command: 'bun',
    args: ['dev'],
    logsPath: '/tmp/preview.log',
  }
  try {
    const first = store.refreshSessions()
    const second = store.refreshSessions()
    const afterMutation = store.refreshSessions({ force: true })
    await flush()
    assert.equal(requests.length, 1)
    requests[0]!.resolve(Response.json({ sessions: [{ ...session, id: 'stale' }] }))
    await flush()
    assert.equal(requests.length, 2)
    assert.equal(store.sessions.length, 0, 'obsolete response must not flash stale data')
    requests[1]!.resolve(Response.json({ sessions: [session] }, { headers: { etag: 'W/"preview-test:2"' } }))
    await Promise.all([first, second, afterMutation])
    assert.deepEqual(
      store.sessions.map((item) => item.id),
      ['new'],
    )
    await store.refreshSessions()
    assert.equal(requests.length, 2, 'polls in the same interval reuse the last result')
  } finally {
    store.$dispose()
    globalThis.fetch = original
  }
})

test('preview consumers share one event stream, perform no idle list reads and pause while hidden', async () => {
  ensureBrowserTestRuntime()
  const original = {
    document: globalThis.document,
    fetch: globalThis.fetch,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    windowSetTimeout: window.setTimeout,
    windowClearTimeout: window.clearTimeout,
    now: Date.now,
  }
  let now = 100_000,
    serial = 0,
    listReads = 0,
    revision = 1
  const timers = new Map<number, { at: number; callback: () => void }>()
  const document = Object.assign(new EventTarget(), { hidden: false })
  const timeout = (callback: () => void, delay = 0) => {
    const id = ++serial
    timers.set(id, { at: now + delay, callback })
    return id
  }
  const clear = (id: number) => {
    timers.delete(id)
  }
  Object.assign(globalThis, { document, setTimeout: timeout, clearTimeout: clear })
  Object.assign(window, { setTimeout: timeout, clearTimeout: clear })
  Date.now = () => now
  const settle = async () => {
    for (let i = 0; i < 40; i++) await Promise.resolve()
  }
  const advance = async (ms: number) => {
    const end = now + ms
    while (true) {
      const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
      if (!next) break
      const [id, timer] = next
      timers.delete(id)
      now = timer.at
      timer.callback()
      await settle()
    }
    now = end
    await settle()
  }
  const streams: Array<{ signal: AbortSignal; announce(): void }> = []
  const session = {
    id: 'shared-live',
    state: 'running',
    directory: '/repo',
    runDirectory: '/repo',
    proxyBasePath: '/preview/shared-live',
    targetUrl: 'http://localhost:3000',
    command: 'bun',
    args: ['dev'],
    logsPath: '/tmp/preview.log',
  }
  const unrelated = { ...session, id: 'preview-in-B', directory: '/repo-B', runDirectory: '/repo-B' }
  globalThis.fetch = (async (input, init) => {
    if (!String(input).endsWith('/events')) {
      listReads++
      return Response.json({ sessions: [session] }, { headers: { etag: `W/"preview-live-test:${revision}"` } })
    }
    const signal = init!.signal as AbortSignal
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        let initial = true
        const announce = () => {
          controller.enqueue(
            new TextEncoder().encode(
              `data: ${JSON.stringify({
                type: 'preview.sessions.changed',
                revision: `preview-live-test:${revision}`,
                delta: {
                  reset: initial,
                  upsert: initial ? [session, unrelated] : [{ ...session, state: 'stopped' }],
                  removed: [],
                },
              })}\n\n`,
            ),
          )
          initial = false
        }
        streams.push({ signal, announce })
        let keepAlive: number
        const heartbeat = () => {
          controller.enqueue(new TextEncoder().encode(': keep-alive\n\n'))
          keepAlive = timeout(heartbeat, 25_000)
        }
        keepAlive = timeout(heartbeat, 25_000)
        signal.addEventListener(
          'abort',
          () => {
            clear(keepAlive)
            controller.error(signal.reason)
          },
          { once: true },
        )
        announce()
      },
    })
    return new Response(body, { headers: { 'content-type': 'text/event-stream' } })
  }) as typeof fetch
  setActivePinia(createPinia())
  const store = useWorkspacePreviewStore()
  try {
    const releaseFirst = store.retainLiveSessions()
    const releaseSecond = store.retainLiveSessions()
    await settle()
    await advance(0)
    assert.equal(streams.length, 1, 'sidebar and dock share their stream')
    assert.equal(listReads, 0, 'initial stream supplies the registry without a second GET')
    const bRecord = store.sessions.find((item) => item.id === unrelated.id)
    await advance(120_000)
    assert.equal(streams.length, 1)
    assert.equal(listReads, 0, 'keep-alive bytes do not trigger list reads')
    revision++
    for (let i = 0; i < 100; i++) streams[0]!.announce()
    await settle()
    await advance(1000)
    assert.equal(listReads, 0, 'changes apply only their descriptor and require no list GET')
    assert.equal(
      store.sessions.find((item) => item.id === unrelated.id),
      bRecord,
      'B keeps its object when A changes',
    )
    assert.equal(store.sessions.find((item) => item.id === session.id)!.state, 'stopped')
    releaseFirst()
    assert.equal(streams[0]!.signal.aborted, false, 'another mounted consumer retains the stream')
    document.hidden = true
    document.dispatchEvent(new Event('visibilitychange'))
    await settle()
    assert.equal(streams[0]!.signal.aborted, true)
    await advance(120_000)
    assert.equal(listReads, 0)
    revision++
    document.hidden = false
    document.dispatchEvent(new Event('visibilitychange'))
    await settle()
    await advance(1000)
    assert.equal(streams.length, 2)
    assert.equal(listReads, 0, 'resume supplies a recovery snapshot over the shared stream')
    releaseSecond()
    assert.equal(streams[1]!.signal.aborted, true)
    await advance(120_000)
    assert.equal(listReads, 0)
  } finally {
    store.$dispose()
    Date.now = original.now
    Object.assign(globalThis, {
      document: original.document,
      fetch: original.fetch,
      setTimeout: original.setTimeout,
      clearTimeout: original.clearTimeout,
    })
    Object.assign(window, { setTimeout: original.windowSetTimeout, clearTimeout: original.windowClearTimeout })
  }
})

test('activity completion and dismissal in the same millisecond both reach the busy indicator', () => {
  ensureBrowserTestRuntime()
  setActivePinia(createPinia())
  const store = useSessionActivityStore()
  const item = { id: 'proc_same_time', kind: 'shell', status: 'running', session_id: 7 }
  const event = (activity: typeof item, reason: string) => ({
    type: 'runtime_signal',
    properties: { kind: 'activity', session_id: 7, payload: { activity, reason, ts_ms: 1000 } },
  })
  try {
    store.applyEvent(event(item, 'started'))
    assert.equal(store.snapshot['7']?.type, 'busy')
    store.applyEvent(event({ ...item, status: 'succeeded' }, 'finished'))
    assert.equal(store.snapshot['7'], undefined)
    store.applyEvent(event(item, 'dismissed'))
    assert.equal(store.snapshot['7'], undefined)
    store.applyEvent({
      ...event(item, 'started'),
      properties: {
        kind: 'activity',
        session_id: 7,
        payload: { activity: item, reason: 'started', ts_ms: 999 },
      },
    })
    assert.equal(store.snapshot['7'], undefined, 'an older event cannot resurrect dismissed work')
  } finally {
    store.$dispose()
  }
})

test('a collapsed sidebar fetches counts and footer pages without enumerating directory history', async () => {
  ensureBrowserTestRuntime()
  setActivePinia(createPinia())
  const original = globalThis.fetch
  const calls: URL[] = []
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'http://agena.test')
    calls.push(url)
    if (url.pathname === '/api/v1/changes/revisions')
      return Response.json(
        Object.fromEntries(
          JSON.parse(url.searchParams.get('resources')!).map((key: string) => [key, 'sidebar-test:1']),
        ),
      )
    if (url.pathname === '/api/v1/workspaces')
      return Response.json({
        items: [
          { id: 7, path: '/repo', session_stats: { total: 10000, roots: 9000, pinned: 100, running: 1, attention: 2 } },
        ],
        page: { has_more: false },
      })
    if (url.pathname === '/api/v1/workspaces/session-stats')
      return Response.json({
        items: [
          {
            workspace_id: 7,
            revision: 'sidebar-test:1',
            stats: { total: 10000, roots: 9000, pinned: 100, running: 1, attention: 2 },
          },
        ],
      })
    assert.equal(url.pathname, '/api/v1/sessions')
    assert.equal(url.searchParams.get('limit'), '1')
    assert.equal(url.searchParams.get('include_total'), 'true')
    assert.equal(url.searchParams.get('workspace_id'), null)
    return Response.json({ items: [], total: 1000, page: { has_more: true, next_cursor: 'do-not-follow' } })
  }) as typeof fetch
  const store = useDirectorySessionStore()
  try {
    store.uiPrefs.collapsedDirectoryIds = ['7']
    store.uiPrefs.pinnedSessionsOpen = false
    store.uiPrefs.favoriteSessionsOpen = false
    store.uiPrefs.recentSessionsOpen = false
    store.uiPrefs.runningSessionsOpen = false
    assert.equal(await store.revalidateFromApi(), true)
    assert.equal(
      calls.length,
      7,
      'one revision check, one catalog page, one batch of stats and one bounded query per footer',
    )
    assert.equal(store.directorySidebarById['7']!.sessionCount, 10000)
    assert.equal(store.directorySidebarById['7']!.recentRows.length, 0)
    assert.equal(store.directorySidebarById['7']!.hasAttentionSessions, true)
    assert.equal(store.favoriteFooterView.total, 1000)
  } finally {
    store.$dispose()
    globalThis.fetch = original
  }
})

test('real sidebar refreshes only dirty directories and buckets, including after body cache eviction and reconnect', async () => {
  ensureBrowserTestRuntime()
  const pinia = createPinia()
  setActivePinia(pinia)
  const originals = {
    fetch: globalThis.fetch,
    now: Date.now,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
  }
  let now = originals.now()
  let timerId = 0
  const timers = new Map<number, { at: number; fn: () => void }>()
  Date.now = () => now
  globalThis.setTimeout = ((fn: () => void, ms = 0) => {
    const id = ++timerId
    timers.set(id, { at: now + ms, fn })
    return id
  }) as typeof setTimeout
  globalThis.clearTimeout = ((id: number) => {
    timers.delete(id)
  }) as typeof clearTimeout
  window.setTimeout = globalThis.setTimeout as typeof window.setTimeout
  window.clearTimeout = globalThis.clearTimeout as typeof window.clearTimeout
  const advance = async (ms: number) => {
    const target = now + ms
    for (let count = 0; count < 1000; count++) {
      await flush()
      const next = [...timers].filter(([, timer]) => timer.at <= target).sort((a, b) => a[1].at - b[1].at)[0]
      if (!next) {
        now = target
        await flush()
        return
      }
      now = next[1].at
      timers.delete(next[0])
      next[1].fn()
    }
    throw new Error('timer loop did not settle')
  }
  const ids = ['8101', '8102', '8103']
  const keys = [
    'workspaces:catalog',
    ...['pinned', 'favorite', 'recent', 'running'].map((kind) => `sessions:bucket:${kind}:count`),
    ...ids.flatMap((id) => [`workspace:${id}:sessions:roots`, `workspace:${id}:stats`]),
  ]
  const tokens = new Map(keys.map((key) => [key, 'sidebar-granularity:1']))
  const titles = new Map(ids.map((id) => [id, `before-${id}`]))
  const totals = new Map(ids.map((id) => [id, 1]))
  let treeEnabled = false
  const childTitles = new Map([
    ['8201', 'child A'],
    ['8401', 'child B'],
  ])
  const calls: URL[] = []
  const etag = (key: string) => ({ etag: `W/"${tokens.get(key) || 'sidebar-granularity:1'}"` })
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'http://agena.test')
    calls.push(url)
    if (url.pathname === '/api/v1/changes/revisions')
      return Response.json(
        Object.fromEntries(
          JSON.parse(url.searchParams.get('resources')!).map((key: string) => [
            key,
            tokens.get(key) || 'sidebar-granularity:1',
          ]),
        ),
      )
    if (url.pathname === '/api/v1/workspaces')
      return Response.json(
        {
          items: ids.map((id) => ({ id: Number(id), path: `/repo-${id}` })),
          page: { has_more: false },
        },
        { headers: etag('workspaces:catalog') },
      )
    if (url.pathname === '/api/v1/workspaces/session-stats')
      return Response.json({
        items: url.searchParams
          .get('ids')!
          .split(',')
          .map((id) => ({
            workspace_id: Number(id),
            revision: tokens.get(`workspace:${id}:stats`),
            stats: {
              total: totals.get(id),
              roots: totals.get(id),
              pinned: treeEnabled && id === '8101' ? 1 : 0,
              running: 0,
              attention: 0,
            },
          })),
      })
    if (url.pathname === '/api/v1/sessions') {
      const id = url.searchParams.get('workspace_id')
      const bucket = url.searchParams.get('bucket')
      const key = id
        ? `workspace:${id}:sessions:${url.searchParams.has('parent_id') ? `parent:${url.searchParams.get('parent_id')}` : bucket ? `bucket:${bucket}` : 'roots'}`
        : bucket
          ? `sessions:bucket:${bucket}${url.searchParams.get('count_only') === 'true' ? ':count' : ''}`
          : 'sessions'
      let items = id
        ? [
            {
              id: Number(id) + 100,
              workspace_id: Number(id),
              title: titles.get(id),
              state: { kind: 'ready' },
              root_id: Number(id) + 100,
              parent_id: null,
              child_session_count: 0,
              version: 1,
            },
          ]
        : []
      if (treeEnabled && id === '8101') {
        const roots = [8201, 8401].map((sid) => ({
          id: sid,
          workspace_id: 8101,
          root_id: sid,
          parent_id: null,
          title: sid === 8201 ? titles.get('8101') : 'other root',
          pinned: sid === 8201,
          state: { kind: 'ready' },
          child_session_count: 1,
          version: 2,
        }))
        const parent = url.searchParams.get('parent_id')
        items = parent
          ? [
              {
                id: Number(parent) + 300,
                workspace_id: 8101,
                root_id: Number(parent),
                parent_id: Number(parent),
                title: childTitles.get(parent),
                state: { kind: 'ready' },
                child_session_count: 0,
                version: 3,
              },
            ]
          : bucket
            ? [roots[0]!]
            : roots
      }
      return Response.json(
        {
          items: url.searchParams.get('count_only') === 'true' ? [] : items,
          total: items.length,
          page: { has_more: false },
        },
        { headers: etag(key) },
      )
    }
    // Unique inert bodies fill the real 80-entry response cache.
    if (url.pathname.startsWith('/evict-sidebar/')) return Response.json({}, { headers: etag('sessions') })
    throw new Error(`Unexpected request ${url}`)
  }) as typeof fetch
  const store = useDirectorySessionStore()
  try {
    store.uiPrefs.collapsedDirectoryIds = ['8103']
    for (const kind of ['pinned', 'favorite', 'recent', 'running'] as const)
      store.uiPrefs[`${kind}SessionsOpen`] = false
    const boot = store.revalidateFromApi()
    await advance(1000)
    assert.equal(await boot, true)
    const seed = resourceSync.checkResourceVersions(keys)
    await advance(1000)
    await seed
    const bView = store.directorySidebarById['8102']
    const cView = store.directorySidebarById['8103']
    const favoriteView = store.favoriteFooterView
    const catalogRows = store.directoryPageRows
    calls.length = 0
    titles.set('8101', 'after-A')
    tokens.set('workspace:8101:sessions:roots', 'sidebar-granularity:2')
    const event = {
      type: 'session_changed',
      properties: {
        kind: 'session_meta_updated',
        session_id: 8201,
        title: 'after-A',
        version: 2,
        resource_revisions: {
          sessions: 'sidebar-granularity:2',
          'workspace:8101:sessions:roots': 'sidebar-granularity:2',
        },
      },
    }
    resourceSync.applyResourceEvent(event)
    ;(pinia as unknown as { _s: Map<string, { applyEvent(event: typeof event): void }> })._s
      .get('chat')!
      .applyEvent(event)
    await advance(11_000)
    assert.deepEqual(
      calls.map((url) => [url.pathname, url.searchParams.get('workspace_id')]),
      [['/api/v1/sessions', '8101']],
    )
    assert.equal(store.directorySidebarById['8101']!.recentRows[0]!.session!.title, 'after-A')
    assert.equal(store.directorySidebarById['8102'], bView, 'unchanged directory retains its actual Vue object')
    assert.equal(store.directorySidebarById['8103'], cView)
    assert.equal(store.favoriteFooterView, favoriteView)
    assert.equal(store.directoryPageRows, catalogRows)

    const { conditionalJson } = (await vite.ssrLoadModule(
      '/src/lib/conditionalJson.ts',
    )) as typeof import('../src/lib/conditionalJson')
    await Promise.all(Array.from({ length: 90 }, (_, i) => conditionalJson('sessions', `/evict-sidebar/${i}`)))
    calls.length = 0
    tokens.set('workspace:8101:sessions:roots', 'sidebar-granularity:3')
    tokens.set('workspace:8101:stats', 'sidebar-granularity:3')
    tokens.set('sessions:bucket:pinned:count', 'sidebar-granularity:3')
    resourceSync.applyResourceEvent({
      type: 'session_changed',
      properties: {
        kind: 'session_meta_updated',
        resource_revisions: {
          'workspace:8101:sessions:roots': 'sidebar-granularity:3',
          'workspace:8101:stats': 'sidebar-granularity:3',
          'sessions:bucket:pinned:count': 'sidebar-granularity:3',
        },
      },
    })
    await advance(11_000)
    assert.equal(
      calls.length,
      3,
      'one stats batch, one directory page and one changed bucket even when all other bodies were evicted',
    )
    assert.equal(calls.find((url) => url.pathname.endsWith('/session-stats'))!.searchParams.get('ids'), '8101')
    assert.equal(calls.filter((url) => url.pathname === '/api/v1/workspaces').length, 0)
    assert.equal(
      calls.filter(
        (url) => url.searchParams.get('workspace_id') === '8102' || url.searchParams.get('workspace_id') === '8103',
      ).length,
      0,
    )
    assert.equal(store.directorySidebarById['8102'], bView)

    // Moving a session affects A and B, while C stays folded and untouched.
    calls.length = 0
    const moved = Object.fromEntries(
      ['8101', '8102'].flatMap((id) =>
        ['sessions:roots', 'stats'].map((section) => {
          const key = `workspace:${id}:${section}`
          tokens.set(key, 'sidebar-granularity:4')
          return [key, 'sidebar-granularity:4']
        }),
      ),
    )
    resourceSync.applyResourceEvent({ type: 'session_changed', properties: { resource_revisions: moved } })
    await advance(11_000)
    assert.deepEqual(
      calls.filter((url) => url.pathname.endsWith('/session-stats')).map((url) => url.searchParams.get('ids')),
      ['8101,8102'],
    )
    assert.deepEqual(
      calls
        .filter((url) => url.pathname === '/api/v1/sessions')
        .map((url) => url.searchParams.get('workspace_id'))
        .sort(),
      ['8101', '8102'],
    )
    assert.equal(store.directorySidebarById['8103'], cView)

    calls.length = 0
    totals.set('8103', 2)
    tokens.set('workspace:8103:stats', 'sidebar-granularity:5')
    resourceSync.applyResourceEvent({
      type: 'session_changed',
      properties: {
        resource_revisions: {
          'workspace:8103:stats': 'sidebar-granularity:5',
        },
      },
    })
    await advance(11_000)
    assert.deepEqual(
      calls.map((url) => [url.pathname, url.searchParams.get('ids')]),
      [['/api/v1/workspaces/session-stats', '8103']],
      'a folded directory updates its counts without enumerating any session page',
    )
    assert.equal(store.directorySidebarById['8103']!.sessionCount, 2)
    assert.equal(store.directorySidebarById['8103']!.recentRows.length, 0)

    calls.length = 0
    store.scheduleSidebarRecoverySync('stream-connected', 0, { force: true })
    await advance(11_000)
    assert.deepEqual(
      calls.map((url) => url.pathname),
      ['/api/v1/changes/revisions'],
      'reconnect checks tokens without reloading evicted but unchanged bodies',
    )

    treeEnabled = true
    totals.set('8101', 4)
    store.uiPrefs.expandedParentSessionIds = ['8201', '8401']
    tokens.set('workspace:8101:sessions:roots', 'sidebar-granularity:6')
    tokens.set('workspace:8101:sessions:bucket:pinned', 'sidebar-granularity:6')
    tokens.set('workspace:8101:sessions:parent:8201', 'sidebar-granularity:6')
    tokens.set('workspace:8101:sessions:parent:8401', 'sidebar-granularity:6')
    tokens.set('workspace:8101:stats', 'sidebar-granularity:6')
    resourceSync.invalidateResources(keys)
    const treeBoot = store.revalidateFromApi()
    await advance(11_000)
    assert.equal(await treeBoot, true)
    const otherRoot = store.directorySidebarById['8101']!.recentRows.find((row) => row.id === '8401')
    const otherChild = store.directorySidebarById['8101']!.recentRows.find((row) => row.id === '8701')
    assert.ok(otherChild, 'second expanded branch is loaded')
    await Promise.all(Array.from({ length: 90 }, (_, i) => conditionalJson('sessions', `/evict-sidebar/tree-${i}`)))
    calls.length = 0
    childTitles.set('8201', 'changed child A')
    tokens.set('workspace:8101:sessions:parent:8201', 'sidebar-granularity:7')
    resourceSync.applyResourceEvent({
      type: 'session_changed',
      properties: {
        resource_revisions: { 'workspace:8101:sessions:parent:8201': 'sidebar-granularity:7' },
      },
    })
    await advance(11_000)
    assert.deepEqual(
      calls.map((url) => [url.pathname, url.searchParams.get('parent_id')]),
      [['/api/v1/sessions', '8201']],
      'only the changed child branch reads, even after body cache eviction',
    )
    assert.equal(
      store.directorySidebarById['8101']!.recentRows.find((row) => row.id === '8501')!.session!.title,
      'changed child A',
    )
    assert.equal(
      store.directorySidebarById['8101']!.recentRows.find((row) => row.id === '8401'),
      otherRoot,
    )
    assert.equal(
      store.directorySidebarById['8101']!.recentRows.find((row) => row.id === '8701'),
      otherChild,
    )

    calls.length = 0
    resourceSync.applyResourceEvent({
      type: 'session_changed',
      properties: {
        resource_revisions: { 'sessions:bucket:pinned': 'sidebar-granularity:8' },
      },
    })
    await advance(11_000)
    assert.equal(calls.length, 0, 'a closed footer ignores row changes while its count is unchanged')
    tokens.set('sessions:bucket:pinned:count', 'sidebar-granularity:9')
    resourceSync.applyResourceEvent({
      type: 'session_changed',
      properties: {
        resource_revisions: { 'sessions:bucket:pinned:count': 'sidebar-granularity:9' },
      },
    })
    await advance(11_000)
    assert.equal(calls.length, 1)
    assert.equal(calls[0]!.searchParams.get('count_only'), 'true', 'closed footer requests only a count')
  } finally {
    for (const instance of (pinia as unknown as { _s: Map<string, { $dispose(): void }> })._s.values())
      instance.$dispose()
    globalThis.fetch = originals.fetch
    Date.now = originals.now
    globalThis.setTimeout = originals.setTimeout
    globalThis.clearTimeout = originals.clearTimeout
    window.setTimeout = originals.setTimeout as typeof window.setTimeout
    window.clearTimeout = originals.clearTimeout as typeof window.clearTimeout
  }
})
