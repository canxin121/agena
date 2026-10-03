import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { createServer } from 'vite'
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
    assert.equal(requests.length, 1)
    requests[0]!.resolve(Response.json({ sessions: [{ ...session, id: 'stale' }] }))
    await flush()
    assert.equal(requests.length, 2)
    assert.equal(store.sessions.length, 0, 'obsolete response must not flash stale data')
    requests[1]!.resolve(Response.json({ sessions: [session] }))
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

test('a collapsed sidebar fetches counts and footer pages without enumerating directory history', async () => {
  ensureBrowserTestRuntime()
  setActivePinia(createPinia())
  const original = globalThis.fetch
  const calls: URL[] = []
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'http://agena.test')
    calls.push(url)
    if (url.pathname === '/api/v1/workspaces')
      return Response.json({
        items: [
          { id: 7, path: '/repo', session_stats: { total: 10000, roots: 9000, pinned: 100, running: 1, attention: 2 } },
        ],
        page: { has_more: false },
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
    assert.equal(calls.length, 5, 'one workspace page plus one bounded query per footer')
    assert.equal(store.directorySidebarById['7']!.sessionCount, 10000)
    assert.equal(store.directorySidebarById['7']!.recentRows.length, 0)
    assert.equal(store.directorySidebarById['7']!.hasAttentionSessions, true)
    assert.equal(store.favoriteFooterView.total, 1000)
  } finally {
    store.$dispose()
    globalThis.fetch = original
  }
})
