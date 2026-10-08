import { afterAll, expect, test } from 'bun:test'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { createRenderer, defineComponent, nextTick, ref, ssrContextKey } from 'vue'
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
const { useDirectorySessionStore } = (await vite.ssrLoadModule(
  '/src/stores/directorySessionStore.ts',
)) as typeof import('../src/stores/directorySessionStore')
const chatApi = (await vite.ssrLoadModule('/src/stores/chat/api.ts')) as typeof import('../src/stores/chat/api')
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

let runtimeClock = 100_000

async function withRuntime(
  run: (context: {
    mount: (setup: () => void) => void
    advance: (ms: number) => Promise<void>
    doc: EventTarget & { hidden: boolean; visibilityState: string }
    tokens: Map<string, string>
    calls: URL[]
    reply: (key: string, body: unknown) => Response
    evict: () => Promise<void>
    setFetch: (handler: (url: URL, init?: RequestInit) => Response | Promise<Response>) => void
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
    location: window.location,
  }
  let now = runtimeClock,
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
  Object.assign(window, { location: new URL('http://agena.test/chat') })
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
    app.provide(ssrContextKey, { modules: new Set() })
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
  const setFetch = (handler: (url: URL, init?: RequestInit) => Response | Promise<Response>) => {
    globalThis.fetch = (async (input, init) => {
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
      return handler(url, init)
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
    runtimeClock = now + 1000
    Date.now = original.now
    Object.assign(globalThis, {
      document: original.document,
      fetch: original.fetch,
      setTimeout: original.setTimeout,
      clearTimeout: original.clearTimeout,
    })
    window.setTimeout = original.windowSetTimeout
    window.clearTimeout = original.windowClearTimeout
    Object.assign(window, { location: original.location })
  }
}

test('creating a session refreshes its sidebar within one second after a previous refresh, without SSE', () =>
  withRuntime(async ({ mount, advance, tokens, calls, reply, setFetch }) => {
    const workspaceIds = ['99601', '99602']
    const resourceKeys = [
      'sessions',
      'workspaces:catalog',
      ...['pinned', 'favorite', 'recent', 'running'].map((kind) => `sessions:bucket:${kind}:count`),
      ...workspaceIds.flatMap((id) => [`workspace:${id}:sessions:roots`, `workspace:${id}:stats`]),
    ]
    for (const key of resourceKeys) tokens.set(key, 'granularity-http:100')
    const rows = new Map(
      workspaceIds.map((id) => [
        id,
        [
          {
            id: Number(id) + 100,
            workspace_id: Number(id),
            title: `Original ${id}`,
            root_id: Number(id) + 100,
            parent_id: null,
            child_session_count: 0,
            version: 1,
            state: { kind: 'ready', data: {} },
          },
        ],
      ]),
    )
    setFetch((url, init) => {
      if (url.pathname === '/api/v1/workspaces')
        return reply('workspaces:catalog', {
          items: workspaceIds.map((id) => ({ id: Number(id), path: `/repo-${id}` })),
          page: { has_more: false },
        })
      if (url.pathname === '/api/v1/workspaces/session-stats')
        return Response.json({
          items: url.searchParams
            .get('ids')!
            .split(',')
            .map((id) => ({
              workspace_id: Number(id),
              revision: tokens.get(`workspace:${id}:stats`),
              stats: { total: rows.get(id)!.length, roots: rows.get(id)!.length, pinned: 0, running: 0, attention: 0 },
            })),
        })
      expect(url.pathname).toBe('/api/v1/sessions')
      if (init?.method === 'POST') {
        const input = JSON.parse(String(init.body))
        expect(input.workspace_id).toBe(99601)
        const created = { ...rows.get('99601')![0]!, id: 99901, root_id: 99901, title: input.title }
        rows.get('99601')!.unshift(created)
        for (const key of ['workspace:99601:sessions:roots', 'workspace:99601:stats', 'sessions:bucket:recent:count'])
          tokens.set(key, 'granularity-http:102')
        return Response.json(created)
      }
      const id = url.searchParams.get('workspace_id')
      const bucket = url.searchParams.get('bucket')
      const key = id ? `workspace:${id}:sessions:roots` : bucket ? `sessions:bucket:${bucket}:count` : 'sessions'
      return reply(key, {
        items: id ? rows.get(id) : [],
        total: id ? rows.get(id)!.length : bucket === 'recent' ? [...rows.values()].flat().length : 0,
        page: { has_more: false },
      })
    })
    let sidebar!: ReturnType<typeof useDirectorySessionStore>
    mount(() => {
      sidebar = useDirectorySessionStore()
      for (const kind of ['pinned', 'favorite', 'recent', 'running'] as const)
        sidebar.uiPrefs[`${kind}SessionsOpen`] = false
    })
    const release = sync.startResourceSync()
    try {
      const boot = sidebar.revalidateFromApi()
      await advance(1000)
      expect(await boot).toBe(true)
      const otherDirectory = sidebar.directorySidebarById['99602']
      // Complete an ordinary refresh immediately before creating. The old
      // recovery interval then prevents all further updates for ten seconds.
      rows.get('99601')![0]!.title = 'Updated before creation'
      tokens.set('workspace:99601:sessions:roots', 'granularity-http:101')
      sync.applyResourceEvent({
        type: 'session_changed',
        properties: { resource_revisions: { 'workspace:99601:sessions:roots': 'granularity-http:101' } },
      })
      await advance(1000)
      expect(sidebar.directorySidebarById['99601']!.recentRows[0]!.session!.title).toBe('Updated before creation')
      calls.length = 0
      // No stream event announces this mutation. Its successful HTTP response
      // must still cause the precise lists and counts to be reconciled.
      const created = await chatApi.createSession({ workspaceId: 99601, title: 'Just created' })
      await advance(1000)
      expect(sidebar.error).toBeNull()
      expect(sidebar.directorySidebarById['99601']!.recentRows.map((row) => row.id)).toContain(created.id)
      expect(sidebar.directorySidebarById['99601']!.sessionCount).toBe(2)
      expect(sidebar.recentFooterView.total).toBe(3)
      expect(sidebar.directorySidebarById['99602']).toBe(otherDirectory)
      expect(calls.filter((url) => url.pathname === '/api/v1/workspaces')).toHaveLength(0)
      expect(calls.filter((url) => url.searchParams.get('workspace_id') === '99602')).toHaveLength(0)
      expect(
        calls
          .filter((url) => url.pathname === '/api/v1/workspaces/session-stats')
          .every((url) => url.searchParams.get('ids') === '99601'),
      ).toBe(true)
      expect(
        calls
          .filter((url) => url.pathname === '/api/v1/sessions' && url.searchParams.has('bucket'))
          .every((url) => url.searchParams.get('bucket') === 'recent'),
      ).toBe(true)
    } finally {
      release()
    }
  }))

test('a first-seen session can enter the open recent footer while its directory remains collapsed', () =>
  withRuntime(async ({ mount, advance, tokens, calls, reply, setFetch }) => {
    const workspaceId = 99603
    const otherWorkspaceId = 99605
    const recentKey = 'sessions:bucket:recent'
    for (const key of [
      'workspaces:catalog',
      recentKey,
      `workspace:${workspaceId}:stats`,
      `workspace:${otherWorkspaceId}:stats`,
      ...['pinned', 'favorite', 'recent', 'running'].map((kind) => `sessions:bucket:${kind}:count`),
    ])
      tokens.set(key, 'granularity-http:110')
    const rows: Array<Record<string, unknown>> = []
    setFetch((url, init) => {
      if (url.pathname === '/api/v1/workspaces')
        return reply('workspaces:catalog', {
          items: [
            { id: workspaceId, path: '/repo-footer' },
            { id: otherWorkspaceId, path: '/repo-footer-other' },
          ],
          page: { has_more: false },
        })
      if (url.pathname === '/api/v1/workspaces/session-stats')
        return Response.json({
          items: url.searchParams
            .get('ids')!
            .split(',')
            .map((id) => ({
              workspace_id: Number(id),
              revision: tokens.get(`workspace:${id}:stats`),
              stats: {
                total: Number(id) === workspaceId ? rows.length : 0,
                roots: Number(id) === workspaceId ? rows.length : 0,
                pinned: 0,
                running: 0,
                attention: 0,
              },
            })),
        })
      if (url.pathname === '/api/v1/sessions/99903') return Response.json(rows[0])
      expect(url.pathname).toBe('/api/v1/sessions')
      if (init?.method === 'POST') {
        const created = {
          id: 99903,
          root_id: 99903,
          parent_id: null,
          workspace_id: workspaceId,
          title: 'Footer first insertion',
          version: 1,
          child_session_count: 0,
          state: { kind: 'ready', data: {} },
        }
        rows.push(created)
        tokens.set(recentKey, 'granularity-http:111')
        tokens.set(`workspace:${workspaceId}:stats`, 'granularity-http:111')
        return Response.json(created)
      }
      expect(url.searchParams.has('workspace_id')).toBe(false)
      const bucket = url.searchParams.get('bucket')!
      return reply(`sessions:bucket:${bucket}${url.searchParams.get('count_only') === 'true' ? ':count' : ''}`, {
        items: bucket === 'recent' ? rows : [],
        total: bucket === 'recent' ? rows.length : 0,
        page: { has_more: false },
      })
    })
    let sidebar!: ReturnType<typeof useDirectorySessionStore>
    mount(() => {
      sidebar = useDirectorySessionStore()
      sidebar.uiPrefs.collapsedDirectoryIds = [String(workspaceId), String(otherWorkspaceId)]
      sidebar.uiPrefs.recentSessionsOpen = true
    })
    sync.invalidateResources()
    const release = sync.startResourceSync()
    try {
      const boot = sidebar.revalidateFromApi()
      await advance(1000)
      expect(await boot).toBe(true)
      expect(sidebar.stateBySessionId['99903']).toBeUndefined()
      const otherDirectory = sidebar.directorySidebarById[String(otherWorkspaceId)]
      const favoriteFooter = sidebar.favoriteFooterView
      calls.length = 0
      const created = await chatApi.createSession({ workspaceId, title: 'Footer first insertion' })
      await advance(1000)
      expect(sidebar.error).toBeNull()
      expect(sidebar.recentFooterView.rows.map((row) => row.id)).toEqual([created.id])
      expect(sidebar.recentFooterView.total).toBe(1)
      expect(sidebar.stateBySessionId[created.id]!.state.kind).toBe('ready')
      expect(sidebar.recentFooterView.rows[0]!.directory?.id).toBe(String(workspaceId))
      expect(sidebar.directorySidebarById[String(workspaceId)]!.sessionCount).toBe(1)
      expect(sidebar.directorySidebarById[String(workspaceId)]!.recentRows).toHaveLength(0)
      expect(sidebar.directorySidebarById[String(otherWorkspaceId)]).toBe(otherDirectory)
      expect(sidebar.favoriteFooterView).toBe(favoriteFooter)
      expect(calls.filter((url) => url.pathname === '/api/v1/sessions/99903')).toHaveLength(0)
      expect(calls.some((url) => url.searchParams.has('workspace_id'))).toBe(false)
      expect(
        calls
          .filter((url) => url.pathname === '/api/v1/sessions' && url.searchParams.has('bucket'))
          .map((url) => url.searchParams.get('bucket')),
      ).toEqual(['recent'])
    } finally {
      release()
    }
  }))

test.each([0, 1500])(
  'the shared create action reveals a new root from a collapsed older page (%d ms chat hydration)',
  (hydrationDelay) =>
    withRuntime(async ({ mount, advance, tokens, calls, reply, setFetch }) => {
      const workspaceId = 99604
      const rootKey = `workspace:${workspaceId}:sessions:roots`
      const statsKey = `workspace:${workspaceId}:stats`
      for (const key of [
        'workspaces:catalog',
        rootKey,
        statsKey,
        ...['pinned', 'favorite', 'recent', 'running'].map((kind) => `sessions:bucket:${kind}:count`),
      ])
        tokens.set(key, `granularity-http:${120 + hydrationDelay}`)
      const rows = Array.from({ length: 12 }, (_, index) => ({
        id: 99800 + index,
        root_id: 99800 + index,
        parent_id: null,
        workspace_id: workspaceId,
        title: `Older ${index}`,
        version: 1,
        child_session_count: 0,
        state: { kind: 'ready', data: {} },
      }))
      setFetch((url, init) => {
        if (url.pathname === '/api/v1/workspaces')
          return reply('workspaces:catalog', {
            items: [{ id: workspaceId, path: '/repo-reveal' }],
            page: { has_more: false },
          })
        if (url.pathname === `/api/v1/workspaces/${workspaceId}`)
          return Response.json({ id: workspaceId, path: '/repo-reveal' })
        if (url.pathname === '/api/v1/workspaces/session-stats')
          return Response.json({
            items: [
              {
                workspace_id: workspaceId,
                revision: tokens.get(statsKey),
                stats: { total: rows.length, roots: rows.length, pinned: 0, running: 0, attention: 0 },
              },
            ],
          })
        if (url.pathname === '/api/v1/sessions/99904/runs')
          return new Promise<Response>((resolve) =>
            setTimeout(
              () =>
                resolve(
                  reply('session:99904:parts', {
                    session_id: 99904,
                    version: 1,
                    parts: [],
                    runs: [], user_message_count: 0,
                    page: { has_more: false },
                  }),
                ),
              hydrationDelay,
            ),
          )
        expect(url.pathname).toBe('/api/v1/sessions')
        if (init?.method === 'POST') {
          const created = { ...rows[0]!, id: 99904, root_id: 99904, title: 'Visible after creation' }
          rows.unshift(created)
          for (const key of [rootKey, statsKey, 'sessions:bucket:recent:count'])
            tokens.set(key, `granularity-http:${121 + hydrationDelay}`)
          return Response.json(created)
        }
        const directory = url.searchParams.has('workspace_id')
        const bucket = url.searchParams.get('bucket')!
        const offset = Number(url.searchParams.get('offset'))
        const limit = Number(url.searchParams.get('limit'))
        return reply(directory ? rootKey : `sessions:bucket:${bucket}:count`, {
          items: directory ? rows.slice(offset, offset + limit) : [],
          total: directory || bucket === 'recent' ? rows.length : 0,
          page: { has_more: directory && offset + limit < rows.length },
        })
      })
      let sidebar!: ReturnType<typeof useDirectorySessionStore>
      mount(() => {
        sidebar = useDirectorySessionStore()
        sidebar.uiPrefs.collapsedDirectoryIds = [String(workspaceId)]
        sidebar.uiPrefs.sessionRootPageByDirectoryId = { [workspaceId]: 1 }
      })
      sync.invalidateResources()
      const release = sync.startResourceSync()
      try {
        const boot = sidebar.revalidateFromApi()
        await advance(1000)
        expect(await boot).toBe(true)
        expect(sidebar.directorySidebarById[String(workspaceId)]!.rootPage).toBe(1)
        calls.length = 0
        const creation = useChatStore().createSession({ workspaceId })
        await advance(1000)
        if (hydrationDelay) {
          // The local revision check finishes before the action can return.
          // Revealing its directory later must still schedule a targeted read.
          expect(sidebar.directorySidebarById[String(workspaceId)]!.sessionCount).toBe(13)
          expect(sidebar.uiPrefs.collapsedDirectoryIds).toContain(String(workspaceId))
          await advance(1000)
        }
        const created = await creation
        expect(created).not.toBeNull()
        expect(sidebar.error).toBeNull()
        expect(sidebar.uiPrefs.collapsedDirectoryIds).not.toContain(String(workspaceId))
        expect(sidebar.uiPrefs.sessionRootPageByDirectoryId[String(workspaceId)]).toBe(0)
        expect(sidebar.directorySidebarById[String(workspaceId)]!.recentRows[0]!.id).toBe(created!.id)
        expect(sidebar.directorySidebarById[String(workspaceId)]!.sessionCount).toBe(13)
        expect(
          calls
            .filter((url) => url.searchParams.has('workspace_id'))
            .map((url) => url.searchParams.get('offset') ?? '0'),
        ).toEqual(['0'])
        expect(calls.filter((url) => url.pathname === '/api/v1/workspaces')).toHaveLength(0)
      } finally {
        release()
      }
    }),
)

test('metadata, deletion and execution writes reconcile mounted lists without SSE, leaving another workspace untouched', () =>
  withRuntime(async ({ mount, advance, tokens, calls, reply, setFetch }) => {
    const workspaces = [99501, 99502]
    type Row = {
      id: number
      workspace_id: number
      root_id: number
      parent_id: number | null
      title: string
      version: number
      favorite: boolean
      pinned: boolean
      child_session_count: number
      state: { kind: string; data: object }
    }
    const rows: Row[] = workspaces.map((id) => ({
      id: id + 100,
      workspace_id: id,
      root_id: id + 100,
      parent_id: null,
      title: `Original ${id}`,
      version: 1,
      favorite: false,
      pinned: false,
      child_session_count: 0,
      state: { kind: 'ready', data: {} },
    }))
    const own = rows[0]!,
      sid = String(own.id)
    let revision = 1000
    const bump = (key: string) => tokens.set(key, `granularity-http:${++revision}`)
    const belongs = (row: Row, bucket: string) =>
      bucket === 'pinned'
        ? row.pinned
        : bucket === 'favorite'
          ? row.favorite
          : bucket === 'running'
            ? row.state.kind === 'running'
            : bucket === 'attention'
              ? row.state.kind === 'awaiting_interaction'
              : row.state.kind === 'ready'
    const changed = (previous: Row, current?: Row) => {
      for (const key of [
        `workspace:${previous.workspace_id}:sessions`,
        previous.parent_id
          ? `workspace:${previous.workspace_id}:sessions:parent:${previous.parent_id}`
          : `workspace:${previous.workspace_id}:sessions:roots`,
        `workspace:${previous.workspace_id}:stats`,
        `session:${previous.id}:state`,
        `session:${previous.id}:parts`,
      ])
        bump(key)
      for (const bucket of ['pinned', 'favorite', 'running', 'attention', 'recent']) {
        if (belongs(previous, bucket) || (current && belongs(current, bucket))) {
          bump(`sessions:bucket:${bucket}`)
          bump(`workspace:${previous.workspace_id}:sessions:bucket:${bucket}`)
        }
        if (belongs(previous, bucket) !== Boolean(current && belongs(current, bucket)))
          bump(`sessions:bucket:${bucket}:count`)
      }
    }
    for (const key of [
      'sessions',
      'workspaces:catalog',
      'activities',
      ...['pinned', 'favorite', 'running', 'attention', 'recent'].flatMap((kind) => [
        `sessions:bucket:${kind}`,
        `sessions:bucket:${kind}:count`,
      ]),
      ...workspaces.flatMap((id) => [`workspace:${id}:sessions:roots`, `workspace:${id}:stats`]),
      ...rows.flatMap((row) => [`session:${row.id}:state`, `session:${row.id}:parts`]),
    ])
      bump(key)
    setFetch((url, init) => {
      if (url.pathname === '/api/v1/workspaces')
        return reply('workspaces:catalog', {
          items: workspaces.map((id) => ({ id, path: `/mutations-${id}` })),
          page: { has_more: false },
        })
      if (url.pathname === '/api/v1/workspaces/session-stats')
        return Response.json({
          items: url.searchParams
            .get('ids')!
            .split(',')
            .map((id) => {
              const list = rows.filter((row) => row.workspace_id === Number(id))
              return {
                workspace_id: Number(id),
                revision: tokens.get(`workspace:${id}:stats`),
                stats: {
                  total: list.length,
                  roots: list.filter((row) => !row.parent_id).length,
                  pinned: list.filter((row) => row.pinned).length,
                  running: list.filter((row) => belongs(row, 'running')).length,
                  attention: list.filter((row) => belongs(row, 'attention')).length,
                },
              }
            }),
        })
      if (url.pathname === '/api/v1/sessions') {
        const opts = {
          workspaceId: url.searchParams.get('workspace_id') ?? undefined,
          parentId: url.searchParams.get('parent_id') ?? undefined,
          roots: url.searchParams.get('roots') === 'true',
          bucket: url.searchParams.get('bucket') ?? undefined,
          countOnly: url.searchParams.get('count_only') === 'true',
        }
        const list = rows.filter(
          (row) =>
            (!opts.workspaceId || row.workspace_id === Number(opts.workspaceId)) &&
            (!opts.parentId || row.parent_id === Number(opts.parentId)) &&
            (!opts.roots || !row.parent_id) &&
            (!opts.bucket || belongs(row, opts.bucket)),
        )
        return reply(chatApi.sessionListResourceKey(opts), {
          items: opts.countOnly ? [] : list,
          total: list.length,
          page: { has_more: false },
        })
      }
      if (url.pathname === `/api/v1/sessions/${sid}`) {
        const previous = structuredClone(own)
        if (init?.method === 'PUT') {
          Object.assign(own, JSON.parse(String(init.body)))
          own.version++
          changed(previous, own)
        }
        if (init?.method === 'DELETE') {
          for (let i = rows.length - 1; i >= 0; i--)
            if (rows[i]!.id === own.id || rows[i]!.parent_id === own.id) rows.splice(i, 1)
          changed(previous)
          return Response.json({})
        }
        return Response.json(own)
      }
      if (url.pathname === `/api/v1/sessions/${sid}/state`)
        return reply(`session:${sid}:state`, { session: own, parts: [], background_activities: [] })
      if (url.pathname === `/api/v1/sessions/${sid}/runs`)
        return reply(`session:${sid}:parts`, {
          session_id: own.id,
          version: own.version,
          parts: [],
          runs: [], user_message_count: 0,
          page: { has_more: false },
        })
      if (init?.method === 'POST' && (url.pathname.endsWith('/fork') || url.pathname.endsWith('/rewind'))) {
        const previous = structuredClone(own)
        const child: Row = {
          ...own,
          id: own.id + 1000 + own.child_session_count,
          parent_id: own.id,
          title: url.pathname.endsWith('/fork') ? 'Forked' : 'Rewound',
          child_session_count: 0,
          version: 1,
        }
        rows.push(child)
        own.child_session_count++
        own.version++
        changed(previous, own)
        bump(`workspace:${own.workspace_id}:sessions:parent:${own.id}`)
        bump('sessions:bucket:recent:count')
        return Response.json({ session: child, parts: [] })
      }
      if (init?.method === 'POST' && url.pathname.startsWith(`/api/v1/sessions/${sid}/`)) {
        const previous = structuredClone(own)
        own.version++
        own.state = { kind: url.pathname.endsWith('/cancel') ? 'ready' : 'running', data: {} }
        changed(previous, own)
        return Response.json(
          url.pathname.endsWith('/messages')
            ? { session: own, parts: [] }
            : url.pathname.endsWith('/cancel')
              ? { result: 'already_terminal' }
              : {},
        )
      }
      throw new Error(`Unexpected mutation request ${url}`)
    })
    let sidebar!: ReturnType<typeof useDirectorySessionStore>
    mount(() => {
      sidebar = useDirectorySessionStore()
      for (const kind of ['pinned', 'favorite', 'recent', 'running'] as const)
        sidebar.uiPrefs[`${kind}SessionsOpen`] = true
    })
    sync.invalidateResources()
    const release = sync.startResourceSync(),
      chat = useChatStore()
    const releaseSession = chat.retainSession(sid)
    try {
      const boot = sidebar.revalidateFromApi()
      await advance(1000)
      expect(await boot).toBe(true)
      await chat.refreshMessages(sid)
      await advance(1000)
      const other = sidebar.directorySidebarById[String(workspaces[1])]
      const assertScoped = () => {
        expect(sidebar.error).toBeNull()
        expect(sidebar.directorySidebarById[String(workspaces[1])]).toBe(other)
        expect(
          calls.some(
            (url) =>
              url.pathname === '/api/v1/workspaces' || url.searchParams.get('workspace_id') === String(workspaces[1]),
          ),
        ).toBe(false)
        expect(
          calls
            .filter((url) => url.pathname === '/api/v1/workspaces/session-stats')
            .every((url) => url.searchParams.get('ids') === String(workspaces[0])),
        ).toBe(true)
        expect(
          calls.some(
            (url) =>
              url.pathname === '/api/v1/sessions' &&
              !url.searchParams.has('workspace_id') &&
              !url.searchParams.has('bucket'),
          ),
        ).toBe(false)
      }
      calls.length = 0
      await chat.renameSession(sid, 'Renamed immediately')
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.recentRows[0]!.session!.title).toBe(
        'Renamed immediately',
      )
      expect(sidebar.recentFooterView.rows[0]!.session!.title).toBe('Renamed immediately')
      await advance(1000)
      assertScoped()
      for (const field of ['favorite', 'pinned'] as const) {
        for (const enabled of [true, false]) {
          calls.length = 0
          await chat.updateSessionMetadata(sid, { [field]: enabled })
          const footer = field === 'favorite' ? sidebar.favoriteFooterView : sidebar.pinnedFooterView
          // Use the returned metadata immediately, before any revision probe.
          expect(footer.rows.some((row) => row.id === sid)).toBe(enabled)
          expect(footer.total).toBe(enabled ? 1 : 0)
          await advance(1000)
          expect(footer.rows.some((row) => row.id === sid)).toBe(enabled)
          expect(footer.total).toBe(enabled ? 1 : 0)
          assertScoped()
        }
      }
      for (const operation of [
        () => chat.sendMessage(sid, { text: 'hello' }),
        () => chat.continueSession(sid),
        () => chat.compactSession(sid),
        () => chat.replyPermission(sid, 'permission-1', 'once'),
        () => chat.replyQuestion(sid, 'question-1', [['yes']]),
        () => chat.rejectQuestion(sid, 'question-2'),
        () => chat.abortSession(sid),
      ]) {
        calls.length = 0
        await operation()
        await advance(1000)
        expect(chat.getSessionState(sid).kind).toBe(own.state.kind)
        expect(sidebar.runningFooterView.rows.some((row) => row.id === sid)).toBe(own.state.kind === 'running')
        expect(sidebar.recentFooterView.rows.some((row) => row.id === sid)).toBe(own.state.kind === 'ready')
        assertScoped()
      }
      calls.length = 0
      const fork = await chat.forkSession(sid)
      await advance(1000)
      expect(sidebar.uiPrefs.expandedParentSessionIds).toContain(sid)
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.recentRows.map((row) => row.id)).toContain(fork!.id)
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.sessionCount).toBe(2)
      assertScoped()
      calls.length = 0
      chat.getMessagesForSession(sid).push({
        info: { id: '88111', sessionID: sid, role: 'user', runState: 'completed', time: { created: 1 } },
        parts: [],
      })
      const rewound = await chat.revertToMessage(sid, '88111')
      await advance(1000)
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.recentRows.map((row) => row.id)).toContain(
        rewound.session.id,
      )
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.sessionCount).toBe(3)
      assertScoped()
      calls.length = 0
      await chat.deleteSession(sid)
      // The durable acknowledgement removes the whole visible subtree even
      // before refreshes can run, including existing footer memberships.
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.recentRows).toHaveLength(0)
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.sessionCount).toBe(0)
      expect(sidebar.recentFooterView.rows.some((row) => row.id === sid)).toBe(false)
      await advance(1000)
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.recentRows).toHaveLength(0)
      expect(sidebar.directorySidebarById[String(workspaces[0])]!.sessionCount).toBe(0)
      expect(sidebar.recentFooterView.rows.some((row) => row.id === sid)).toBe(false)
      assertScoped()
      // Simulate a pre-delete list arriving last. Successful deletion's
      // tombstone wins for both the parent and cached descendants.
      setFetch(() =>
        reply(`workspace:${workspaces[0]}:sessions:roots`, {
          items: [
            own,
            { ...own, id: Number(fork!.id), parent_id: own.id },
            { ...own, id: Number(rewound.session.id), parent_id: own.id },
          ],
          total: 3,
          page: { has_more: false },
        }),
      )
      bump(`workspace:${workspaces[0]}:sessions:roots`)
      sync.invalidateResources([`workspace:${workspaces[0]}:sessions:roots`])
      const lateRead = chatApi.listSessions({ workspaceId: workspaces[0], roots: true })
      await advance(1000)
      expect((await lateRead).sessions).toHaveLength(0)
    } finally {
      releaseSession()
      release()
    }
  }))

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
      if (url.pathname.endsWith('/runs'))
        return reply(`session:${sid}:parts`, {
          session_id: sid,
          version: 1,
          parts: [],
          runs: [], user_message_count: 0,
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
      tokens.set('session:9961:parts', 'granularity-http:31')
      chat.reconcileLiveState()
      await advance(3000)
      expect(calls.filter((url) => url.pathname.endsWith('/runs')).map((url) => url.pathname)).toEqual([
        '/api/v1/sessions/9961/runs',
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

test('activity controls refresh other mounted projections without SSE or unopened logs', () =>
  withRuntime(async ({ advance, tokens, calls, reply, setFetch }) => {
    const { controlActivity } = await vite.ssrLoadModule('/src/lib/activityApi.ts')
    const activityId = 'task_fresh_control',
      sid = '99401',
      otherSid = '99402'
    let stopped = false
    const descriptor = () => ({
      id: activityId,
      kind: 'task',
      status: stopped ? 'succeeded' : 'running',
      title: 'Task',
      description: '',
      session_id: Number(sid),
      last_seq: 1,
      controls: ['stop'],
    })
    for (const key of ['activities', `session:${sid}:state`, `session:${otherSid}:state`])
      tokens.set(key, 'restarted-api:12000')
    setFetch((url, init) => {
      if (url.pathname === `/api/v1/activities/${activityId}/stop` && init?.method === 'POST') {
        stopped = true
        tokens.set('activities', 'restarted-api:12001')
        tokens.set(`session:${sid}:state`, 'restarted-api:12001')
        return Response.json(descriptor())
      }
      if (url.pathname === '/api/v1/activities') return reply('activities', [descriptor()])
      const target = url.pathname.match(/sessions\/(\d+)\/state/)?.[1]
      if (target)
        return reply(`session:${target}:state`, {
          session: {
            id: Number(target),
            workspace_id: 99400,
            title: 'Session',
            version: stopped ? 2 : 1,
            state: { kind: 'ready', data: {} },
          },
          parts: [],
          background_activities: target === sid && !stopped ? [descriptor()] : [],
        })
      throw new Error(`Unexpected activity mutation request ${url}`)
    })
    const chat = useChatStore(),
      busy = useSessionActivityStore()
    const releaseSession = chat.retainSession(sid),
      releaseOther = chat.retainSession(otherSid),
      release = sync.startResourceSync()
    try {
      sync.invalidateResources(['activities'])
      const boot = Promise.all([
        chat.refreshExecutionStatus(sid),
        chat.refreshExecutionStatus(otherSid),
        busy.refresh(),
      ])
      await advance(1000)
      await boot
      expect(chat.sessionBackgroundActivities(sid)).toHaveLength(1)
      expect(busy.snapshot[sid]?.type).toBe('busy')
      calls.length = 0
      await controlActivity(activityId, 'stop')
      await advance(1000)
      expect(chat.sessionBackgroundActivities(sid)).toHaveLength(0)
      expect(busy.snapshot[sid]).toBeUndefined()
      expect(calls.filter((url) => url.pathname === '/api/v1/activities')).toHaveLength(1)
      expect(calls.filter((url) => url.pathname === `/api/v1/sessions/${sid}/state`)).toHaveLength(1)
      expect(calls.some((url) => url.pathname.includes(`/${otherSid}/`) || url.pathname.endsWith('/logs'))).toBe(false)
    } finally {
      releaseSession()
      releaseOther()
      release()
    }
  }))

test('saving a default model updates mounted pickers with one shared runtime read and no model inventory reads', () =>
  withRuntime(async ({ mount, advance, calls, setFetch }) => {
    const { useModelSelectionCatalog } = await vite.ssrLoadModule('/src/pages/chat/modelSelectionCatalog.ts')
    const { mutateModelConfiguration } = await vite.ssrLoadModule('/src/lib/modelConfigurationApi.ts')
    const { setRuntimeSetting, patchRuntimeSettings } = await vite.ssrLoadModule('/src/lib/runtimeSettings.ts')
    let model = 'first'
    setFetch((url, init) => {
      if (url.pathname === '/api/v1/settings' && ['PATCH', 'PUT'].includes(init?.method || '')) {
        const body = JSON.parse(String(init?.body || '{}'))
        if (!body.dry_run && body.reload !== false) {
          if (body.path === 'providers.default_selection.model') model = body.value
          else if (body.changes?.default_selection?.model) model = body.changes.default_selection.model
          else if (!body.path) model = 'second'
        }
        return Response.json({ changed: true, dry_run: !!body.dry_run })
      }
      if (url.pathname === '/api/v1/runtime')
        return Response.json({ default_selection: { provider: 'fake', adapter: 'adapter', model } })
      if (url.pathname === '/api/v1/providers') return Response.json([{ provider_id: 'fake' }])
      if (url.pathname.endsWith('/configured-models'))
        return Response.json([
          {
            adapter_id: 'adapter',
            enabled: true,
            models: ['first', 'second'].map((id) => ({ provider_id: 'fake', adapter_id: 'adapter', id })),
          },
        ])
      throw new Error(`Unexpected model configuration request ${url}`)
    })
    const catalogs: Array<ReturnType<typeof useModelSelectionCatalog>> = []
    mount(() => catalogs.push(useModelSelectionCatalog()))
    mount(() => catalogs.push(useModelSelectionCatalog()))
    await Promise.all(catalogs.map((catalog) => catalog.loadProvidersAndModels()))
    expect(calls.filter((url) => url.pathname === '/api/v1/runtime')).toHaveLength(1)
    const originalInventories = catalogs.map((catalog) => catalog.providers.value)
    calls.length = 0
    await mutateModelConfiguration('/api/v1/settings', { method: 'PATCH', body: '{}' })
    await advance(500)
    for (const [i, catalog] of catalogs.entries()) {
      expect(catalog.runtimeDefaultSelection.value.model).toBe('second')
      expect(catalog.providers.value).toBe(originalInventories[i])
    }
    expect(calls.filter((url) => url.pathname === '/api/v1/runtime')).toHaveLength(1)
    expect(
      calls.some((url) => url.pathname === '/api/v1/providers' || url.pathname.endsWith('/configured-models')),
    ).toBe(false)

    for (const update of [
      () => setRuntimeSetting('providers.default_selection.model', 'first'),
      () =>
        patchRuntimeSettings('providers', {
          default_selection: { provider: 'fake', adapter: 'adapter', model: 'second' },
        }),
    ]) {
      calls.length = 0
      await update()
      await advance(500)
      for (const [i, catalog] of catalogs.entries()) {
        expect(catalog.runtimeDefaultSelection.value.model).toBe(model)
        expect(catalog.providers.value).toBe(originalInventories[i])
      }
      expect(calls.filter((url) => url.pathname === '/api/v1/runtime')).toHaveLength(1)
      expect(
        calls.some((url) => url.pathname === '/api/v1/providers' || url.pathname.endsWith('/configured-models')),
      ).toBe(false)
    }

    // An unrelated setting, validation-only write, or saved-but-not-reloaded
    // file cannot trigger reads of the runtime/model inventory.
    calls.length = 0
    await setRuntimeSetting('runtime.telemetry.enabled', false)
    await setRuntimeSetting('providers.default_selection.model', 'first', { dry_run: true })
    await setRuntimeSetting('providers.default_selection.model', 'first', { reload: false })
    await advance(500)
    expect(
      calls.filter(
        (url) =>
          url.pathname === '/api/v1/runtime' ||
          url.pathname === '/api/v1/providers' ||
          url.pathname.endsWith('/configured-models'),
      ),
    ).toHaveLength(0)
  }))

test('catalog refresh follows completion with small status probes and stops reading when the job finishes', () =>
  withRuntime(async ({ mount, advance, calls, setFetch }) => {
    const { default: component } = await vite.ssrLoadModule('/src/components/settings/ModelCatalogPanel.vue')
    let running = false,
      finished = false
    setFetch((url, init) => {
      if (url.pathname === '/api/v1/model-catalog/refresh' && init?.method === 'POST') {
        running = true
        return Response.json({})
      }
      if (url.pathname === '/api/v1/model-catalog')
        return Response.json({
          summary: { refreshing: running },
          total: 1,
          limit: Number(url.searchParams.get('limit')),
          items: [{ model_id: finished ? 'fresh' : 'previous', source: 'mock' }],
        })
      throw new Error(`Unexpected catalog request ${url}`)
    })
    let state!: {
      refreshCatalog(): Promise<void>
      items: { value: Array<{ model_id: string }> }
      origin: { value: string }
    }
    mount(() => {
      state = component.setup({}, { expose() {} })
    })
    await advance(500)
    expect(state.items.value[0]!.model_id).toBe('previous')
    await state.refreshCatalog()
    await advance(0)
    calls.length = 0
    await advance(1500)
    expect(calls).toHaveLength(1)
    expect(calls[0]!.searchParams.get('limit')).toBe('1')
    running = false
    finished = true
    calls.length = 0
    await advance(1500)
    expect(state.items.value[0]!.model_id).toBe('fresh')
    expect(calls).toHaveLength(2)
    expect(calls[0]!.searchParams.get('limit')).toBe('1')
    expect(Number(calls[1]!.searchParams.get('limit'))).toBeGreaterThan(1)
    calls.length = 0
    await advance(60_000)
    expect(calls).toHaveLength(0)
    // Switching filters during a slow read must start the new query and
    // prevent the old result from replacing it.
    let finishOld!: (response: Response) => void
    setFetch((url) =>
      url.searchParams.get('origin') === 'old'
        ? new Promise<Response>((resolve) => {
            finishOld = resolve
          })
        : Response.json({
            summary: { refreshing: false },
            total: 1,
            items: [{ model_id: 'new-filter', source: 'mock' }],
          }),
    )
    state.origin.value = 'old'
    await advance(0)
    state.origin.value = 'new'
    await advance(0)
    expect(state.items.value[0]!.model_id).toBe('new-filter')
    finishOld(
      Response.json({ summary: { refreshing: false }, total: 1, items: [{ model_id: 'old-filter', source: 'mock' }] }),
    )
    await advance(0)
    expect(state.items.value[0]!.model_id).toBe('new-filter')
  }))

test('temporary local read contention retries promptly without consuming the network failure backoff', () =>
  withRuntime(async ({ advance }) => {
    const { createRevalidator } = await vite.ssrLoadModule('/src/lib/revalidation.ts')
    let busy = true,
      reads = 0
    const queue = createRevalidator(
      async () => {
        reads++
        if (busy) throw new DOMException('Local file read is busy', 'AbortError')
      },
      { intervalMs: 250, retryMs: 5000 },
    )
    try {
      await queue.refresh().catch(() => {})
      await advance(249)
      expect(reads).toBe(1)
      busy = false
      await advance(1)
      expect(reads).toBe(2)
      await advance(60_000)
      expect(reads).toBe(2)
    } finally {
      queue.dispose()
    }
  }))
