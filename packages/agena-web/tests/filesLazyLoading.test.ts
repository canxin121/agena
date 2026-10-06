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

type FileEditingState = {
  explorerRootPath: Ref<string>
  selectedFile: Ref<{ name: string; path: string; type: string } | null>
  viewerMode: Ref<string>
  fileContent: Ref<string>
  draftContent: Ref<string>
  dirty: Ref<boolean>
  autoSaveEnabled: Ref<boolean>
  isSaving: Ref<boolean>
  save(options?: { silent?: boolean }): Promise<boolean>
  clearAutoSaveTimer(): void
  entriesByDir: Ref<Record<string, ListEntry[]>>
  directoryNextOffset: Ref<Record<string, number>>
  loadDirectory(path: string, options?: { force?: boolean; preserveLoaded?: boolean }): Promise<void>
  cancelDirectoryRequests(): void
  expandedDirs: Ref<Set<string>>
  createNode(kind: 'createFile' | 'createFolder', base: string, name: string): Promise<void>
  renameNodePath(path: string, name: string): Promise<void>
  deletePaths(paths: string[]): Promise<void>
}

async function withFileEditor(run: (state: FileEditingState) => Promise<void>) {
  prepareRuntime()
  const { default: component } = await vite.ssrLoadModule('/src/pages/FilesPage.vue')
  const pinia = createPinia()
  let state!: FileEditingState
  const app = createSSRApp({
    ...component,
    ssrRender: undefined,
    setup(props: object, context: object) {
      state = component.setup(props, context)
      return () => null
    },
  })
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/', component: { render: () => null } }],
  })
  await router.push('/')
  app
    .use(router)
    .use(pinia)
    .use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
  const originalFetch = globalThis.fetch
  try {
    await renderToString(app)
    state.explorerRootPath.value = '/repo'
    state.selectedFile.value = { name: 'file.txt', path: '/repo/file.txt', type: 'file' }
    state.viewerMode.value = 'text'
    state.fileContent.value = 'original'
    state.draftContent.value = 'submitted'
    await run(state)
  } finally {
    state?.clearAutoSaveTimer()
    state?.cancelDirectoryRequests()
    disposePinia(pinia)
    globalThis.fetch = originalFetch
  }
}

test('saving captures submitted text and preserves edits made during the request; failures do not loop', async () =>
  withFileEditor(async (state) => {
    const requests: Array<{ body: { path: string; content: string }; resolve: (response: Response) => void }> = []
    globalThis.fetch = ((_, init) =>
      new Promise<Response>((resolve) =>
        requests.push({ body: JSON.parse(String(init?.body)), resolve }),
      )) as typeof fetch
    state.autoSaveEnabled.value = true
    const save = state.save({ silent: true })
    assert.equal(requests[0]?.body.content, 'submitted')
    state.draftContent.value = 'edited during save'
    requests[0]!.resolve(Response.json({ success: true }))
    assert.equal(await save, true)
    assert.equal(state.fileContent.value, 'submitted')
    assert.equal(state.draftContent.value, 'edited during save')
    assert.equal(state.dirty.value, true)
    await new Promise((resolve) => setTimeout(resolve, 700))
    assert.equal(requests.length, 2, 'successful save follows up once with the new draft')
    assert.equal(requests[1]?.body.content, 'edited during save')
    requests[1]!.resolve(new Response('{}', { status: 500 }))
    await new Promise((resolve) => setTimeout(resolve, 750))
    assert.equal(requests.length, 2, 'a failed autosave does not restart itself')
    assert.equal(state.dirty.value, true)
    assert.equal(state.isSaving.value, false)
  }))

test('a delayed save from a different root cannot replace the currently displayed file', async () =>
  withFileEditor(async (state) => {
    state.autoSaveEnabled.value = false
    let finish!: (response: Response) => void
    globalThis.fetch = (() =>
      new Promise<Response>((resolve) => {
        finish = resolve
      })) as typeof fetch
    const saving = state.save({ silent: true })
    state.explorerRootPath.value = '/another'
    state.selectedFile.value = { name: 'next.txt', path: '/another/next.txt', type: 'file' }
    state.fileContent.value = 'next saved'
    state.draftContent.value = 'next draft'
    finish(Response.json({ success: true }))
    await saving
    assert.equal(state.fileContent.value, 'next saved')
    assert.equal(state.draftContent.value, 'next draft')
  }))

test('automatic directory refresh preserves exactly the loaded pagination prefix', async () =>
  withFileEditor(async (state) => {
    state.entriesByDir.value = {
      '/repo': Array.from({ length: 800 }, (_, index) => ({ name: String(index), type: 'file' })),
    }
    state.directoryNextOffset.value = { '/repo': 800 }
    const offsets: number[] = []
    globalThis.fetch = (async (url) => {
      const params = new URL(String(url), 'http://agena.test').searchParams
      const offset = Number(params.get('offset'))
      const limit = Number(params.get('limit'))
      offsets.push(offset)
      assert.equal(limit, 400)
      return Response.json({
        entries: Array.from({ length: limit }, (_, index) => ({ name: `current-${offset + index}`, type: 'file' })),
        nextOffset: offset + limit,
        hasMore: true,
      })
    }) as typeof fetch
    await state.loadDirectory('/repo', { force: true, preserveLoaded: true })
    assert.deepEqual(offsets, [0, 400])
    assert.equal(state.entriesByDir.value['/repo']?.length, 800)
    assert.equal(state.entriesByDir.value['/repo']?.[0]?.name, 'current-0')
    assert.equal(state.directoryNextOffset.value['/repo'], 800)
  }))

test('creating, renaming and deleting a file refresh only its containing folder', async () =>
  withFileEditor(async (state) => {
    state.expandedDirs.value = new Set(['/repo/changed', '/repo/other'])
    state.entriesByDir.value = { '/repo': [], '/repo/changed': [], '/repo/other': [] }
    const lists: string[] = []
    globalThis.fetch = (async (input, init) => {
      const url = new URL(String(input), 'http://agena.test')
      if (init?.method && init.method !== 'GET') return Response.json({ success: true })
      lists.push(url.searchParams.get('path')!)
      return Response.json({ entries: [], nextOffset: 0, hasMore: false })
    }) as typeof fetch
    await state.createNode('createFile', '/repo/changed', 'new.txt')
    await state.renameNodePath('/repo/changed/new.txt', 'renamed.txt')
    await state.deletePaths(['/repo/changed/renamed.txt'])
    assert.deepEqual(lists, ['/repo/changed', '/repo/changed', '/repo/changed'])
  }))
