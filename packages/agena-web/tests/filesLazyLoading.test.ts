import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createSSRApp, type Ref } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createPinia, disposePinia } from 'pinia'
import { createI18n } from 'vue-i18n'
import { createMemoryHistory, createRouter } from 'vue-router'
import { ensureBrowserTestRuntime } from './testRuntime'
import type { ListEntry } from '../src/pages/files/types'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  plugins: [
    {
      name: 'ssr-memory-history',
      transform: (code, id) =>
        id.endsWith('/src/router.ts') ? code.replaceAll('createWebHistory', 'createMemoryHistory') : undefined,
    },
  ],
})
after(() => vite.close())

function prepareRuntime() {
  ensureBrowserTestRuntime()
  Object.assign(document, {
    compatMode: 'CSS1Compat',
    querySelector: () => null,
    addEventListener() {},
    removeEventListener() {},
  })
  Object.assign(window, {
    location: { search: '', pathname: '/', hash: '' },
    addEventListener() {},
    removeEventListener() {},
  })
  if (typeof HTMLElement === 'undefined') Object.assign(globalThis, { HTMLElement: class HTMLElement {} })
  if (typeof sessionStorage === 'undefined') {
    const values = new Map<string, string>()
    Object.assign(globalThis, {
      sessionStorage: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, value),
        removeItem: (key: string) => values.delete(key),
        clear: () => values.clear(),
      },
    })
  }
}

test('the file tree reads one page per action, coalesces requests, retries and rejects obsolete results', async () => {
  prepareRuntime()
  const { default: component } = await vite.ssrLoadModule('/src/pages/FilesPage.vue')
  const pinia = createPinia()
  type State = {
    explorerRootPath: Ref<string>
    entriesByDir: Ref<Record<string, ListEntry[]>>
    directoryNextOffset: Ref<Record<string, number>>
    directoryLoadErrors: Ref<Record<string, boolean>>
    inFlightDirs: Ref<Set<string>>
    loadDirectory(path: string, options?: { force?: boolean; more?: boolean }): Promise<void>
    cancelDirectoryRequests(): void
    persistExplorerNow(): void
    restoreExplorerState(path: string): boolean
  }
  let state!: State
  const subject = {
    ...component,
    ssrRender: undefined,
    setup(props: object, context: object) {
      state = component.setup(props, context)
      return () => null
    },
  }
  const app = createSSRApp(subject)
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/', component: { render: () => null } }],
  })
  await router.push('/')
  app
    .use(router)
    .use(pinia)
    .use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
  const requests: Array<{ url: URL; signal: AbortSignal; resolve: (response: Response) => void }> = []
  const originalFetch = globalThis.fetch
  globalThis.fetch = ((url, init) =>
    new Promise<Response>((resolve) =>
      requests.push({ url: new URL(String(url), 'http://agena.test'), signal: init!.signal!, resolve }),
    )) as typeof fetch
  const entry = (name: string) => ({ name, type: 'file' })
  const reply = (index: number, entries: unknown[], nextOffset: number, hasMore: boolean) =>
    requests[index]!.resolve(Response.json({ entries, nextOffset, hasMore }))
  try {
    await renderToString(app)
    state.explorerRootPath.value = '/repo'
    const first = state.loadDirectory('/repo')
    await state.loadDirectory('/repo')
    assert.equal(requests.length, 1)
    assert.equal(requests[0]!.url.searchParams.get('limit'), '400')
    reply(0, [entry('a'), entry('b')], 2, true)
    await first
    assert.equal(requests.length, 1, 'hasMore must not eagerly fetch another page')
    assert.equal(state.directoryNextOffset.value['/repo'], 2)
    const more = state.loadDirectory('/repo', { more: true })
    assert.equal(requests[1]!.url.searchParams.get('offset'), '2')
    reply(1, [entry('b'), entry('c')], 4, true)
    await more
    assert.deepEqual(
      state.entriesByDir.value['/repo']!.map((item) => item.name),
      ['a', 'b', 'c'],
    )
    state.persistExplorerNow()
    state.directoryNextOffset.value = {}
    assert.equal(state.restoreExplorerState('/repo'), true)
    assert.equal(state.directoryNextOffset.value['/repo'], 4, 'cache restores the cursor with its loaded prefix')

    const stalled = state.loadDirectory('/repo', { more: true })
    reply(2, [], 4, true)
    await stalled
    assert.equal(state.directoryLoadErrors.value['/repo'], true)
    assert.equal(state.inFlightDirs.value.size, 0)
    assert.equal(state.entriesByDir.value['/repo']!.length, 3)
    const old = state.loadDirectory('/repo', { more: true })
    const refreshed = state.loadDirectory('/repo', { force: true })
    assert.equal(requests[3]!.signal.aborted, true)
    reply(3, [entry('obsolete')], 5, false)
    await old
    assert.equal(state.inFlightDirs.value.has('/repo'), true, 'old completion cannot release the new request')
    reply(4, [entry('fresh')], 1, false)
    await refreshed
    assert.deepEqual(
      state.entriesByDir.value['/repo']!.map((item) => item.name),
      ['fresh'],
    )
    assert.equal(state.directoryNextOffset.value['/repo'], undefined)
    assert.equal(state.directoryLoadErrors.value['/repo'], false)
  } finally {
    state?.cancelDirectoryRequests()
    disposePinia(pinia)
    globalThis.fetch = originalFetch
  }
})

test('file-tree paging controls stay on the left, follow their own children and hide inside closed directories', async () => {
  prepareRuntime()
  const { default: component } = await vite.ssrLoadModule('/src/pages/files/components/FilesExplorerPane.vue')
  const { default: messages } = await vite.ssrLoadModule('/src/i18n/messages/en-US.ts')
  const callbacks = Object.fromEntries(
    [
      'toggleMultiSelectMode',
      'selectAllVisibleNodes',
      'invertVisibleNodeSelection',
      'handleNodeClick',
      'handleNodeLongPress',
      'refreshRoot',
      'collapseAll',
      'expandDirectory',
      'createNode',
      'renameNode',
      'moveNodeByDrag',
      'runFileAction',
      'openDialog',
      'deleteNode',
      'deleteSelectedNodes',
      'openMoveSelectedDialog',
      'clearSelectedPaths',
      'uploadFiles',
    ].map((name) => [name, () => {}]),
  )
  const app = createSSRApp(component, {
    ...callbacks,
    root: '/repo',
    isCompactLayout: false,
    selectedFilePath: '',
    activeCreateDir: '/repo',
    hasRootChildren: true,
    deletingPaths: new Set(),
    selectedPaths: new Set(),
    multiSelectEnabled: false,
    uploading: false,
    flattenedTree: [
      { node: { path: '/repo/open', name: 'open', type: 'directory' }, depth: 0, isExpanded: true },
      { node: { path: '/repo/open/a', name: 'a', type: 'file' }, depth: 1, isExpanded: false },
      { node: { path: '/repo/closed', name: 'closed', type: 'directory' }, depth: 0, isExpanded: false },
    ],
    moreDirectories: {
      '/repo': { loading: false, error: false },
      '/repo/open': { loading: false, error: true },
      '/repo/closed': { loading: true, error: false },
    },
  })
  app.use(createPinia()).use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': messages } }))
  const html = await renderToString(app)
  assert.equal((html.match(/data-directory-page=/g) || []).length, 2)
  const retry = html.match(/<button[^>]*data-directory-page="\/repo\/open"[^>]*>[\s\S]*?<\/button>/)?.[0] || ''
  assert.match(retry, /text-left/)
  assert.match(retry, /> Retry<\/button>/)
  assert.ok(html.indexOf('data-directory-page="/repo/open"') < html.indexOf('data-directory-page="/repo"'))
  assert.doesNotMatch(html, /data-directory-page="\/repo\/closed"/)
})
