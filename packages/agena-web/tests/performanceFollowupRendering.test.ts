import assert from 'node:assert/strict'
import { test as nodeTest, after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { computed, createRenderer, h, nextTick, reactive, ref, ssrContextKey } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'
import { createPinia, disposePinia } from 'pinia'
import { renderToString } from 'vue/server-renderer'
import { createSSRApp } from 'vue'
import { ensureBrowserTestRuntime } from './testRuntime'

const test = (name: string, run: () => Promise<void>) => nodeTest(name, { timeout: 20_000 }, run)

const models = new Map<string, any>()
const editors: any[] = []
const codeEditors: any[] = []
let codeImports = 0
let paneSerial = 0
const terminals: any[] = []
const terminalStreams: Array<{ options: any; closed: boolean }> = []
let terminalFits = 0
let modelDisposals = 0,
  editorDisposals = 0,
  zoneAdds = 0,
  frameRequests = 0
const subscription = () => ({ dispose() {} })
const monaco = {
  Uri: { parse: (path: string) => ({ toString: () => path }) },
  KeyCode: {},
  Emitter: class {
    event() {}
    fire() {}
  },
  languages: { registerCodeLensProvider: subscription },
  editor: {
    ScrollType: { Immediate: 0 },
    EditorOption: { renderSideBySide: 0 },
    getModel: (uri: { toString(): string }) => models.get(uri.toString()),
    createModel(value: string, language: string, uri: { toString(): string }) {
      let disposed = false,
        version = 1
      const model = {
        uri,
        getValue: () => value,
        setValue(next: string) {
          value = next
          version++
        },
        getVersionId: () => version,
        getLanguageId: () => language,
        setLanguage(next: string) {
          language = next
        },
        getLineCount: () => value.split('\n').length,
        isDisposed: () => disposed,
        dispose() {
          disposed = true
          models.delete(uri.toString())
          modelDisposals++
        },
        findMatches: () => [],
      }
      models.set(uri.toString(), model)
      return model
    },
    setModelLanguage(model: any, language: string) {
      model.setLanguage(language)
    },
    setTheme() {},
    registerCommand: subscription,
    onDidChangeMarkers: subscription,
    getModelMarkers: () => [],
    create(_host: any, options: any) {
      let model = options.model
      const editor = {
        options: [] as any[],
        getModel: () => model,
        setModel(next: any) {
          model = next
        },
        getValue: () => model.getValue(),
        setValue(value: string) {
          model.setValue(value)
        },
        updateOptions(value: any) {
          this.options.push(value)
        },
        onDidChangeModelContent: subscription,
        onDidScrollChange: subscription,
        onKeyDown: subscription,
        getVisibleRanges: () => [],
        changeViewZones(callback: (accessor: any) => void) {
          callback({ removeZone() {}, addZone: () => 'code-zone' })
        },
        saveViewState: () => ({ state: true }),
        restoreViewState() {},
        revealLine() {},
        dispose() {
          editorDisposals++
        },
      }
      codeEditors.push(editor)
      return editor
    },
    createDiffEditor(host: any) {
      let model: any = null
      const createSide = (side: 'original' | 'modified') => ({
        options: [] as any[],
        layout: { width: 0, height: 0 },
        layoutListeners: [] as Array<() => void>,
        selections: [] as any[],
        scrolls: [] as number[],
        getModel: () => model?.[side],
        updateOptions(options: any) {
          this.options.push(options)
        },
        getLayoutInfo() {
          return this.layout
        },
        onDidLayoutChange(callback: () => void) {
          this.layoutListeners.push(callback)
          return subscription()
        },
        onKeyDown: subscription,
        onDidFocusEditorText: subscription,
        onDidChangeModelContent: subscription,
        createDecorationsCollection: () => ({ set() {} }),
        getTopForLineNumber: (line: number) => line * 20,
        setScrollTop(value: number) {
          this.scrolls.push(value)
        },
        changeViewZones(callback: (accessor: any) => void) {
          callback({ removeZone() {}, addZone: () => String(++zoneAdds) })
        },
        getSelection: () => null,
        setSelection(selection: any) {
          this.selections.push(selection)
        },
        revealRangeInCenterIfOutsideViewport() {},
        focus() {},
      })
      const original = createSide('original'),
        modified = createSide('modified')
      const editor = {
        original,
        modified,
        setModels: 0,
        options: [] as any[],
        updates: [] as Array<() => void>,
        getOriginalEditor: () => original,
        getModifiedEditor: () => modified,
        getModel: () => model,
        setModel(next: any) {
          model = next
          this.setModels++
        },
        getContainerDomNode: () => host,
        updateOptions(options: any) {
          this.options.push(options)
        },
        getOption: () => true,
        getLineChanges: () => [],
        onDidUpdateDiff(callback: () => void) {
          this.updates.push(callback)
          return subscription()
        },
        layout() {},
        dispose() {
          editorDisposals++
        },
      }
      editors.push(editor)
      return editor
    },
  },
}
;(globalThis as any).__agenaPerformanceMonaco = monaco
;(globalThis as any).__agenaPerformanceTerminal = class {
  cols = 80
  rows = 24
  resets = 0
  disposed = false
  options = {}
  writes: string[] = []
  constructor() {
    terminals.push(this)
  }
  loadAddon() {}
  open() {}
  focus() {}
  refresh() {}
  reset() {
    this.resets++
  }
  write(value: string) {
    this.writes.push(value)
  }
  onData() {
    return subscription()
  }
  attachCustomKeyEventHandler() {}
  dispose() {
    this.disposed = true
  }
}
;(globalThis as any).__agenaPerformanceTerminalFit = class {
  fit() {
    terminalFits++
  }
}
;(globalThis as any).__agenaPerformanceTerminalSse = (options: any) => {
  const stream = { options, closed: false }
  terminalStreams.push(stream)
  return {
    close() {
      stream.closed = true
    },
  }
}
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  optimizeDeps: { noDiscovery: true, include: [] },
  ssr: { noExternal: ['monaco-editor', '@monaco-editor/loader', '@xterm/xterm', '@xterm/addon-fit'] },
  plugins: [
    {
      name: 'performance-lifecycle-monaco',
      enforce: 'pre',
      resolveId(source) {
        if (source === 'monaco-editor') return '\0performance-monaco'
        if (source === '@monaco-editor/loader') return '\0performance-monaco-loader'
        if (source === '@xterm/xterm') return '\0performance-terminal'
        if (source === '@xterm/addon-fit') return '\0performance-terminal-fit'
        if (source === 'performance-terminal-sse') return '\0performance-terminal-sse'
      },
      load(id) {
        if (id === '\0performance-monaco')
          return 'const m = globalThis.__agenaPerformanceMonaco; export const editor = m.editor; export const Uri = m.Uri; export const KeyCode = m.KeyCode;'
        if (id === '\0performance-monaco-loader') return 'export default globalThis.__agenaPerformanceLoader;'
        if (id === '\0performance-terminal') return 'export const Terminal = globalThis.__agenaPerformanceTerminal;'
        if (id === '\0performance-terminal-fit')
          return 'export const FitAddon = globalThis.__agenaPerformanceTerminalFit;'
        if (id === '\0performance-terminal-sse')
          return 'export const connectSse = globalThis.__agenaPerformanceTerminalSse;'
      },
      transform(code, id) {
        if (id.endsWith('/src/lib/monacoSetup.ts')) return 'export async function ensureMonacoReady() {}'
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
        if (id.endsWith('/src/pages/TerminalPage.vue'))
          return code.replace("from '@/lib/sse'", "from 'performance-terminal-sse'")
      },
    },
  ],
})
;(globalThis as any).__agenaPerformanceLoader = {
  __getMonacoInstance: () => null,
  init() {
    codeImports++
    return Object.assign(Promise.resolve(monaco), { cancel() {} })
  },
}
after(() => {
  delete (globalThis as any).__agenaPerformanceMonaco
  delete (globalThis as any).__agenaPerformanceLoader
  delete (globalThis as any).__agenaPerformanceTerminal
  delete (globalThis as any).__agenaPerformanceTerminalFit
  delete (globalThis as any).__agenaPerformanceTerminalSse
  return vite.close()
})

const flush = async () => {
  await new Promise<void>((resolve) => setImmediate(resolve))
  await nextTick()
}
const deferred = <T>() => {
  let resolve!: (value: T) => void
  return {
    promise: new Promise<T>((done) => {
      resolve = done
    }),
    resolve: (value: T) => resolve(value),
  }
}
function element() {
  return {
    childNodes: [] as any[],
    clientWidth: 800,
    clientHeight: 400,
    style: { setProperty() {} },
    classList: { add() {}, toggle() {}, contains: () => false },
    appendChild(child: any) {
      this.childNodes.push(child)
    },
    addEventListener() {},
  }
}
function prepareRuntime() {
  ensureBrowserTestRuntime()
  ;(globalThis as any).HTMLElement ??= class {}
  Object.assign(document, {
    hidden: false,
    visibilityState: 'visible',
    documentElement: element(),
    createElement: element,
    addEventListener() {},
    removeEventListener() {},
  })
  Object.assign(window, {
    addEventListener() {},
    removeEventListener() {},
    requestAnimationFrame() {
      frameRequests++
      return frameRequests
    },
    cancelAnimationFrame() {},
  })
}
const renderer = createRenderer<any, any>({
  createElement: element,
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
async function mount(
  path: string,
  props: any,
  shown = ref(true),
  host: boolean | 'terminal' = false,
  routePath?: string,
  configure?: (pinia: ReturnType<typeof createPinia>) => Promise<void>,
) {
  const { default: component } = await vite.ssrLoadModule(path)
  const { workspacePaneContextKey } = await vite.ssrLoadModule('/src/app/workspace/workspacePaneContext.ts')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  let state: any
  const subject = {
    ...component,
    setup(values: any, context: any) {
      state = component.setup(values, context)
      return host ? () => h('div', { ref: host === 'terminal' ? state.el : state.containerRef }) : () => null
    },
  }
  const app = renderer.createApp({ setup: () => () => h(subject, props) })
  app.config.idPrefix = `performance-${++paneSerial}`
  app.provide(workspacePaneContextKey, {
    windowId: computed(() => 'performance-pane'),
    isFocused: computed(() => false),
    isVisible: computed(() => shown.value),
    route: computed(() => ({ path: '/test', query: {} })),
    navigate: async () => {},
  })
  app.provide(ssrContextKey, {})
  const pinia = createPinia()
  app.use(pinia)
  app.use(i18n)
  let router: ReturnType<typeof createRouter> | undefined
  if (routePath) {
    router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/settings/:section', component: subject },
        { path: '/terminal', component: subject },
      ],
    })
    await router.push(routePath)
    await router.isReady()
    app.use(router)
  }
  if (configure) await configure(pinia)
  app.mount({})
  return {
    state,
    shown,
    router,
    pinia,
    i18n,
    unmount() {
      app.unmount()
      disposePinia(pinia)
    },
  }
}
function sources(input: RequestInfo | URL, value = 'loaded') {
  const paths: string[] = JSON.parse(new URL(String(input), 'http://agena.test').searchParams.get('paths')!)
  return Response.json(
    Object.fromEntries(
      paths.map((path) => [
        path,
        Object.fromEntries(['effective', 'file', 'global', 'workspace'].map((source) => [source, { path, value }])),
      ]),
    ),
  )
}

test('a real settings field defers hidden reads, preserves draft on locale change, and cancels an old path autosave', async () => {
  prepareRuntime()
  const originalFetch = globalThis.fetch
  const requests: string[] = [],
    writes: string[] = []
  globalThis.fetch = (async (input, init) => {
    requests.push(String(input))
    if (init?.method === 'PUT') {
      writes.push(JSON.parse(String(init.body)).path)
      return Response.json({ changed: true })
    }
    return sources(input)
  }) as typeof fetch
  const props = reactive({ path: 'ui.old', label: 'A setting', reload: false })
  const pane = await mount('/src/components/settings/ServerSettingField.vue', props, ref(false))
  const locale = pane.i18n.global.locale.value
  try {
    await flush()
    assert.equal(requests.length, 0)
    pane.shown.value = true
    await flush()
    assert.equal(requests.length, 1)
    assert.equal(pane.state.localValue.value, 'loaded')
    pane.state.localValue.value = 'unsaved draft'
    pane.i18n.global.locale.value = 'en-US'
    await nextTick()
    const label = pane.state.booleanOptions.value[0].label
    pane.i18n.global.locale.value = 'zh-CN'
    await nextTick()
    assert.notEqual(pane.state.booleanOptions.value[0].label, label)
    assert.equal(pane.state.localValue.value, 'unsaved draft')
    assert.equal(requests.length, 1)
    pane.state.onTextInput({ target: { value: 'old path input' } })
    props.path = 'ui.new'
    await flush()
    // Reuse the old timer's deadline directly, without waiting in real time.
    assert.equal(pane.state.localValue.value, 'loaded')
    await new Promise((resolve) => setTimeout(resolve, 700))
    assert.deepEqual(writes, [])
    const completed = requests.length
    pane.shown.value = false
    pane.shown.value = true
    await flush()
    assert.equal(requests.length, completed, 'showing unchanged loaded fields must not reread them')
  } finally {
    pane.i18n.global.locale.value = locale
    pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('a settings write captures its original path and its late completion cannot refresh the replacement field', async () => {
  prepareRuntime()
  const originalFetch = globalThis.fetch
  const write = deferred<Response>(),
    reads: string[] = [],
    writes: any[] = []
  globalThis.fetch = ((input, init) => {
    if (init?.method === 'PUT') {
      writes.push(JSON.parse(String(init.body)))
      return write.promise
    }
    reads.push(String(input))
    return Promise.resolve(sources(input))
  }) as typeof fetch
  const props = reactive({ path: 'ui.before', label: 'A setting', reload: false })
  const pane = await mount('/src/components/settings/ServerSettingField.vue', props)
  try {
    await flush()
    pane.state.localValue.value = 'authorized value'
    const saving = pane.state.save()
    await flush()
    props.path = 'ui.after'
    await flush()
    const count = reads.length
    assert.equal(pane.state.localValue.value, 'loaded')
    write.resolve(Response.json({ changed: true }))
    await saving
    await flush()
    assert.equal(writes[0].path, 'ui.before')
    assert.equal(writes[0].value, 'authorized value')
    assert.equal(reads.length, count)
    assert.equal(pane.state.localValue.value, 'loaded')
  } finally {
    pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('Settings loads the chat catalog only for conversation display and language changes do not remount the workbench', async () => {
  prepareRuntime()
  const originalFetch = globalThis.fetch
  let catalogs = 0
  globalThis.fetch = (async (input) => {
    if (String(input).includes('/plugins/surface')) {
      catalogs++
      return Response.json({ permission_tools: [] })
    }
    return Response.json({})
  }) as typeof fetch
  const pane = await mount(
    '/src/pages/SettingsPage.vue',
    {},
    ref(true),
    false,
    '/settings/interface?view=web-appearance',
  )
  const locale = pane.i18n.global.locale.value
  try {
    await flush()
    assert.equal(catalogs, 0)
    pane.i18n.global.locale.value = locale === 'en-US' ? 'zh-CN' : 'en-US'
    await flush()
    assert.equal(pane.state.settingsRefreshNonce.value, 0)
    assert.equal(catalogs, 0)
    await pane.router!.replace('/settings/interface?view=conversation')
    await flush()
    assert.equal(catalogs, 1)
    await pane.router!.replace('/settings/interface?view=web-appearance')
    await pane.router!.replace('/settings/interface?view=conversation')
    await flush()
    assert.equal(catalogs, 1)
  } finally {
    pane.i18n.global.locale.value = locale
    pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('Git diffs preserve loaded model identity through repository switches, empty results, and failed refreshes', async () => {
  prepareRuntime()
  const originalFetch = globalThis.fetch
  const requests: Array<{ input: string; pending: ReturnType<typeof deferred<Response>> }> = []
  globalThis.fetch = ((input) => {
    const pending = deferred<Response>()
    requests.push({ input: String(input), pending })
    return pending.promise
  }) as typeof fetch
  const props = reactive({ directory: '/repo/one', path: 'src/a.ts', onStageHunk() {} })
  const pane = await mount('/src/components/git/GitEditorDiffViewer.vue', props, ref(false))
  const reply = (index: number, diff: string, original: string, modified: string) => {
    requests[index]!.pending.resolve(Response.json({ diff }))
    requests[index + 1]!.pending.resolve(Response.json({ original, modified }))
  }
  try {
    await flush()
    assert.equal(requests.length, 0)
    pane.shown.value = true
    const initial = pane.state.refresh()
    await flush()
    const diff = '--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1 @@\n-before\n+after\n'
    reply(0, diff, 'before', 'after')
    await initial
    await flush()
    const scope = pane.state.loadedModelScope.value
    assert.match(scope, /repo\/one/)
    assert.equal(pane.state.hunks.value.length, 1)
    props.directory = '/repo/two'
    await flush()
    assert.equal(pane.state.loadedModelScope.value, scope)
    assert.equal(pane.state.actionsReady.value, false)
    assert.equal(pane.state.displayModified.value, 'after')
    reply(2, '', '', '')
    await flush()
    assert.notEqual(pane.state.loadedModelScope.value, scope)
    assert.match(pane.state.loadedModelScope.value, /repo\/two/)
    assert.equal(pane.state.hunks.value.length, 0)
    assert.equal(pane.state.displayModified.value, '')
    await new Promise((resolve) => setTimeout(resolve, 760))
    const reload = pane.state.refresh()
    await flush()
    const loaded = pane.state.loadedModelScope.value
    requests[4]!.pending.resolve(Response.json({ message: 'temporary read failure' }, { status: 500 }))
    requests[5]!.pending.resolve(Response.json({ original: '', modified: '' }))
    await reload
    assert.equal(pane.state.loadedModelScope.value, loaded)
    assert.match(pane.state.error.value, /temporary read failure/)
    const count = requests.length
    pane.shown.value = false
    pane.shown.value = true
    await flush()
    assert.equal(requests.length, count, 'a resumed pane must preserve the error backoff')
  } finally {
    pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('real diff editors initialize only when visible, do not spin at zero size, share leases, and reuse unchanged hunk zones', async () => {
  prepareRuntime()
  models.clear()
  editors.length = 0
  modelDisposals = editorDisposals = zoneAdds = frameRequests = 0
  const props = reactive({
    originalValue: 'one\ntwo',
    modifiedValue: 'one\nchanged',
    modelId: 'shared-working',
    originalModelId: 'shared-base',
    initialTopLine: 2,
    hunkActions: [
      { id: 'h1', oldStart: 2, oldCount: 1, newStart: 2, newCount: 1, additions: 1, deletions: 1, stageEnabled: true },
    ],
    wrap: false,
    hunkActionsBusy: false,
  })
  const first = await mount('/src/components/MonacoDiffEditor.vue', props, ref(false), true)
  let second: Awaited<ReturnType<typeof mount>> | undefined
  try {
    await flush()
    assert.equal(editors.length, 0)
    first.shown.value = true
    await first.state.initializeDiffEditor()
    await flush()
    assert.equal(editors.length, 1)
    assert.equal(models.size, 2)
    assert.equal(frameRequests, 0)
    assert.equal(zoneAdds, 1)
    const editor = editors[0]!
    for (let i = 0; i < 40; i++) for (const callback of editor.updates) callback()
    await flush()
    assert.equal(zoneAdds, 1)
    props.hunkActionsBusy = true
    await flush()
    assert.equal(zoneAdds, 1, 'busy state updates must keep existing hunk DOM')
    props.hunkActionsBusy = false
    await flush()
    assert.equal(zoneAdds, 1)
    editor.modified.layout = { width: 800, height: 500 }
    for (const callback of editor.modified.layoutListeners) callback()
    assert.deepEqual(editor.modified.scrolls, [40])
    const options = editor.options.length
    first.shown.value = false
    props.wrap = true
    props.modifiedValue = 'one\nnew hidden body'
    await flush()
    assert.equal(editor.options.length, options)
    assert.equal(editor.modified.getModel().getValue(), 'one\nchanged')
    first.shown.value = true
    await flush()
    assert.equal(editor.modified.getModel().getValue(), 'one\nnew hidden body')
    assert.equal(editor.setModels, 1)
    second = await mount('/src/components/MonacoDiffEditor.vue', props, ref(true), true)
    await second.state.initializeDiffEditor()
    await flush()
    assert.equal(editors.length, 2)
    assert.equal(models.size, 2)
    first.unmount()
    assert.equal(modelDisposals, 0)
    assert.equal(editors[1]!.modified.getModel().isDisposed(), false)
    second.unmount()
    second = undefined
    assert.equal(modelDisposals, 2)
    assert.equal(editorDisposals, 2)
    assert.equal(models.size, 0)
  } finally {
    if (editorDisposals < 1) first.unmount()
    second?.unmount()
  }
})

test('file timeline viewers isolate independently selected revisions and keep their editor when the revision changes', async () => {
  prepareRuntime()
  models.clear()
  editors.length = 0
  modelDisposals = editorDisposals = 0
  const path = '/src/pages/files/components/FileViewerPane.vue'
  const { default: viewer } = await vite.ssrLoadModule(path)
  const defaults = Object.fromEntries(
    Object.entries(viewer.props).map(([key, descriptor]: [string, any]) => {
      const type = descriptor.type
      return [
        key,
        type === String ? '' : type === Boolean ? false : type === Array ? [] : type === Function ? () => {} : null,
      ]
    }),
  )
  const viewerProps = () =>
    reactive({
      ...defaults,
      selectedFile: { name: 'a.ts', path: '/repo/src/a.ts', type: 'file' },
      selectedPath: '/repo/src/a.ts',
      timelinePath: 'src/a.ts',
      timelineEnabled: true,
      timelineCommits: [],
      timelineLeftContent: 'base',
      timelineRightContent: 'first revision',
    })
  const firstViewer = await mount(path, viewerProps())
  const secondViewer = await mount(path, viewerProps())
  const firstProps = reactive({
    originalValue: 'base',
    modifiedValue: 'first revision',
    readOnly: true,
    originalPath: firstViewer.state.timelineLeftModelPath.value,
    path: firstViewer.state.timelineRightModelPath.value,
  })
  const secondProps = reactive({
    originalValue: 'other base',
    modifiedValue: 'other revision',
    readOnly: true,
    originalPath: secondViewer.state.timelineLeftModelPath.value,
    path: secondViewer.state.timelineRightModelPath.value,
  })
  let first: Awaited<ReturnType<typeof mount>> | undefined
  let second: Awaited<ReturnType<typeof mount>> | undefined
  try {
    assert.notEqual(firstProps.originalPath, secondProps.originalPath)
    assert.notEqual(firstProps.path, secondProps.path)
    first = await mount('/src/components/MonacoDiffEditor.vue', firstProps, ref(true), true)
    await first.state.initializeDiffEditor()
    second = await mount('/src/components/MonacoDiffEditor.vue', secondProps, ref(true), true)
    await second.state.initializeDiffEditor()
    await flush()
    assert.equal(editors.length, 2)
    assert.equal(models.size, 4)
    assert.equal(editors[0].modified.getModel().getValue(), 'first revision')
    assert.equal(editors[1].modified.getModel().getValue(), 'other revision')
    const originalModel = editors[0].modified.getModel()
    firstProps.modifiedValue = 'new selected revision'
    await flush()
    assert.equal(editors[0].modified.getModel(), originalModel)
    assert.equal(originalModel.getValue(), 'new selected revision')
    assert.equal(editors[1].modified.getModel().getValue(), 'other revision')
    assert.equal(editors[0].setModels, 1, 'changing a revision preserves the mounted editor and models')
    first.unmount()
    first = undefined
    assert.equal(models.size, 2)
    assert.equal(editors[1].modified.getModel().isDisposed(), false)
    second.unmount()
    second = undefined
    assert.equal(models.size, 0)
  } finally {
    first?.unmount()
    second?.unmount()
    firstViewer.unmount()
    secondViewer.unmount()
  }
})

test('the real code editor wrapper postpones initialization and applies hidden changes once without recreating its editor', async () => {
  prepareRuntime()
  models.clear()
  codeEditors.length = 0
  codeImports = modelDisposals = editorDisposals = 0
  const props = reactive({ path: 'file:old', value: 'old body', language: 'typescript', options: { readOnly: true } })
  const pane = await mount('/src/lib/monaco-editor/Editor.ts', props, ref(false), true)
  try {
    await flush()
    assert.equal(codeImports, 0)
    assert.equal(codeEditors.length, 0)
    pane.shown.value = true
    for (let i = 0; i < 5; i++) await flush()
    assert.equal(codeImports, 1)
    assert.equal(codeEditors.length, 1)
    const editor = codeEditors[0]!
    const updates = editor.options.length
    pane.shown.value = false
    props.path = 'file:new'
    props.value = 'latest hidden body'
    props.language = 'rust'
    props.options.readOnly = false
    await flush()
    assert.equal(editor.getModel().uri.toString(), 'file:old')
    assert.equal(editor.getValue(), 'old body')
    assert.equal(editor.options.length, updates)
    pane.shown.value = true
    await flush()
    assert.equal(codeEditors.length, 1)
    assert.equal(editor.getModel().uri.toString(), 'file:new')
    assert.equal(editor.getValue(), 'latest hidden body')
    assert.equal(editor.getModel().getLanguageId(), 'rust')
    assert.equal(editor.options.at(-1).readOnly, false)
    assert.equal(models.size, 1)
  } finally {
    pane.unmount()
  }
  assert.equal(models.size, 0)
  assert.equal(editorDisposals, 1)
  assert.equal(modelDisposals, 2)
})

test('closing or hiding one code lens owner preserves another pane using the same model', async () => {
  prepareRuntime()
  const oldRegistry = (globalThis as any).__ocCodeLensRegistry__
  delete (globalThis as any).__ocCodeLensRegistry__
  const model = monaco.editor.createModel('body', 'typescript', monaco.Uri.parse('file:shared-lenses'))
  const first = await mount(
    '/src/components/MonacoCodeEditor.vue',
    reactive({
      modelValue: 'body',
      path: 'file:shared-lenses',
      codeLensActions: [{ line: 1, title: 'First pane action', onClick() {} }],
    }),
  )
  const second = await mount(
    '/src/components/MonacoCodeEditor.vue',
    reactive({
      modelValue: 'body',
      path: 'file:shared-lenses',
      codeLensActions: [{ line: 1, title: 'Second pane action', onClick() {} }],
    }),
  )
  let firstUnmounted = false
  try {
    await flush()
    const a = monaco.editor.create({}, { model }),
      b = monaco.editor.create({}, { model })
    first.state.handleMount(a, monaco)
    second.state.handleMount(b, monaco)
    const registry = (globalThis as any).__ocCodeLensRegistry__
    assert.equal(registry.byModel.get('file:shared-lenses')[0].title, 'Second pane action')
    assert.equal(registry.actionsByKey.size, 2)
    first.unmount()
    firstUnmounted = true
    assert.equal(registry.byModel.get('file:shared-lenses')[0].title, 'Second pane action')
    assert.equal(registry.actionsByKey.size, 1)
    second.shown.value = false
    await flush()
    assert.equal(registry.byModel.size, 0)
    assert.equal(registry.actionsByKey.size, 0)
    second.shown.value = true
    await flush()
    assert.equal(registry.byModel.get('file:shared-lenses')[0].title, 'Second pane action')
  } finally {
    if (!firstUnmounted) first.unmount()
    second.unmount()
    model.dispose()
    ;(globalThis as any).__ocCodeLensRegistry__ = oldRegistry
  }
})

for (const [name, path, initialize, sentinel] of [
  [
    'activities',
    '/src/components/settings/ActivitiesPanel.vue',
    (state: any) => {
      state.activities.value = [{ id: 'a', title: 'retained-activity-sentinel', created_at_ms: 1, controls: [] }]
    },
    'retained-activity-sentinel',
  ],
  [
    'permissions',
    '/src/components/settings/PermissionsPanel.vue',
    (state: any) => {
      state.rules.value = [
        { id: 1, subject_kind: 'tool', tool_name: 'retained-permission-sentinel', scope: 'global' },
      ]
    },
    'retained-permission-sentinel',
  ],
  [
    'memories',
    '/src/components/settings/MemoriesPanel.vue',
    (state: any) => {
      state.items.value = [{ name: 'retained-memory-sentinel' }]
    },
    'retained-memory-sentinel',
  ],
  [
    'usage',
    '/src/components/settings/UsagePanel.vue',
    (state: any) => {
      const totals = Object.fromEntries(
        [
          'requests',
          'sessions',
          'input_tokens',
          'output_tokens',
          'reasoning_tokens',
          'cache_read_tokens',
          'total_tokens',
          'total_cost_usd',
          'recorded_cost_usd',
          'estimated_cost_usd',
          'unpriced_requests',
        ].map((key) => [key, 1]),
      )
      state.stats.value = { totals, by_provider: [{ ...totals, provider_id: 'retained-usage-sentinel' }] }
    },
    'retained-usage-sentinel',
  ],
] as const) {
  test(`${name} renders its loaded rows through refresh and an error banner`, async () => {
    prepareRuntime()
    const { default: component } = await vite.ssrLoadModule(path)
    const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
    for (const loading of [true, false]) {
      const subject = {
        ...component,
        setup(props: any, context: any) {
          const state = component.setup(props, context)
          initialize(state)
          state.loading.value = loading
          state.error.value = loading ? '' : 'temporary refresh problem'
          return state
        },
      }
      const app = createSSRApp(subject, name === 'permissions' ? { scope: 'global' } : {})
      const pinia = createPinia()
      const { settingsText } = await vite.ssrLoadModule('/src/i18n/settingsText.ts')
      app.config.globalProperties.$st = settingsText
      app.use(pinia)
      app.use(i18n)
      try {
        const html = await renderToString(app)
        assert.ok(html.includes(sentinel), 'loaded data remains mounted during refresh and a temporary failure')
        if (!loading) assert.ok(html.includes('temporary refresh problem'))
      } finally {
        disposePinia(pinia)
      }
    }
  })
}

test('a real preview keeps the same frame, shares a pending probe, rejects stale URLs, and can retry an error', async () => {
  prepareRuntime()
  const originalFetch = globalThis.fetch
  const requests: Array<{ input: string; signal: AbortSignal; pending: ReturnType<typeof deferred<Response>> }> = []
  globalThis.fetch = ((input, init) => {
    const pending = deferred<Response>()
    requests.push({ input: String(input), signal: init!.signal!, pending })
    return pending.promise
  }) as typeof fetch
  let preview: any,
    owners = 0
  const pane = await mount(
    '/src/features/workspacePreview/components/WorkspacePreviewDockPanel.vue',
    {},
    ref(false),
    false,
    '/settings/interface',
    async (pinia) => {
      const { useWorkspacePreviewStore } = await vite.ssrLoadModule('/src/stores/workspacePreview.ts')
      preview = useWorkspacePreviewStore(pinia)
      preview.sessions = [
        {
          id: 'probe',
          state: 'running',
          proxyBasePath: '/api/v1/workbench/preview/s/probe/',
          targetUrl: 'http://localhost:3000',
        },
      ]
      preview.activeSessionId = 'probe'
      preview.refreshSessions = async () => {}
      preview.retainLiveSessions = () => {
        owners++
        return () => {
          owners--
        }
      }
    },
  )
  try {
    await flush()
    assert.equal(requests.length, 0)
    assert.equal(owners, 0)
    pane.shown.value = true
    const initial = pane.state.setFrameUrlNow()
    await flush()
    await pane.state.setFrameUrlNow()
    assert.equal(requests.length, 1)
    assert.equal(owners, 1)
    requests[0]!.pending.resolve(new Response(null))
    await initial
    const src = pane.state.frameSrc.value
    assert.ok(src.includes('/api/v1/workbench/preview/s/probe/'))
    await pane.state.setFrameUrlNow()
    assert.equal(requests.length, 1, 'the same URL keeps the current iframe')
    preview.sessions = [{ ...preview.sessions[0], proxyBasePath: '/api/v1/workbench/preview/s/new/' }]
    const oldUrl = pane.state.setFrameUrlNow()
    await flush()
    preview.activeSessionId = ''
    preview.sessions = []
    await pane.state.setFrameUrlNow()
    assert.equal(requests[1]!.signal.aborted, true)
    requests[1]!.pending.resolve(new Response(null))
    await oldUrl
    assert.equal(pane.state.frameSrc.value, '')
    preview.sessions = [
      {
        id: 'probe',
        state: 'running',
        proxyBasePath: '/api/v1/workbench/preview/s/probe/',
        targetUrl: 'http://localhost:3000',
      },
    ]
    preview.activeSessionId = 'probe'
    pane.state.iframeError.value = 'previous transient error'
    const retry = pane.state.setFrameUrlNow()
    await flush()
    requests[2]!.pending.resolve(new Response(null))
    await retry
    assert.equal(pane.state.iframeError.value, '')
    assert.equal(pane.state.frameSrc.value, src)
    pane.shown.value = false
    await pane.state.setFrameUrlNow()
    assert.equal(requests.length, 3)
    assert.equal(owners, 0)
  } finally {
    pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('terminal panes defer initialization, stop hidden layout work, reuse xterm, and ignore repeated snapshots', async () => {
  prepareRuntime()
  terminals.length = terminalStreams.length = terminalFits = 0
  const originalFetch = globalThis.fetch
  const requests: string[] = []
  const snapshot = {
    version: 1,
    updatedAt: 1,
    activeSessionId: 'term-1',
    sessionIds: ['term-1'],
    sessionMetaById: { 'term-1': { name: 'Terminal' } },
    folders: [],
  }
  globalThis.fetch = (async (input) => {
    const url = String(input)
    requests.push(url)
    if (url.endsWith('/terminal/state')) return Response.json(snapshot)
    if (url.endsWith('/term-1') || url.endsWith('/term-1/start'))
      return Response.json({ sessionId: 'term-1', cwd: '/repo', running: true })
    return new Response(null)
  }) as typeof fetch
  const pane = await mount('/src/pages/TerminalPage.vue', {}, ref(false), 'terminal', '/terminal')
  try {
    await flush()
    assert.equal(requests.length, 0)
    assert.equal(terminals.length, 0)
    pane.state.scheduleResize()
    pane.state.el.value.clientWidth = 0
    pane.shown.value = true
    await pane.state.initializeTerminal()
    await new Promise((resolve) => setTimeout(resolve, 100))
    assert.equal(terminals.length, 0, 'a zero-sized container must not initialize or fit xterm')
    assert.equal(terminalFits, 0)
    const outputStream = terminalStreams.find((stream) => stream.options.debugLabel === 'sse:terminal:term-1')!
    outputStream.options.onEvent({ type: 'data', seq: 1, data: 'buffered until the host has a size' })
    pane.state.el.value.clientWidth = 800
    pane.state.scheduleResize()
    await new Promise((resolve) => setTimeout(resolve, 100))
    assert.equal(terminals.length, 1)
    const terminal = terminals[0]
    assert.ok(terminal.writes.includes('buffered until the host has a size'))
    const resets = terminal.resets
    const completed = requests.length
    const stateStream = terminalStreams.find((stream) => stream.options.debugLabel === 'sse:terminal-ui-state')!
    stateStream.options.onEvent({ type: 'terminal-ui-state.snapshot', seq: 1, state: snapshot })
    await flush()
    assert.equal(requests.length, completed)
    assert.equal(terminal.resets, resets)
    stateStream.options.onEvent({
      type: 'terminal-ui-state.snapshot',
      seq: 2,
      state: { ...snapshot, version: 2, updatedAt: 2, sessionMetaById: { 'term-1': { name: 'Renamed' } } },
    })
    await flush()
    assert.equal(terminal.resets, resets, 'metadata updates must not reset the viewport or replay its output')
    assert.equal(requests.length, completed, 'metadata updates must not probe unchanged terminal memberships')
    pane.state.scheduleResize()
    pane.shown.value = false
    pane.state.el.value.clientWidth = 0
    const fits = terminalFits
    const hiddenRequests = requests.length
    pane.state.scheduleResize()
    await new Promise((resolve) => setTimeout(resolve, 100))
    assert.equal(terminalFits, fits)
    assert.equal(requests.length, hiddenRequests)
    assert.ok(terminalStreams.every((stream) => stream.closed))
    pane.state.el.value.clientWidth = 800
    pane.shown.value = true
    await flush()
    assert.equal(terminals.length, 1)
    assert.equal(pane.state.term.value, terminal)
    assert.equal(requests.filter((url) => url.endsWith('/terminal/state')).length, 1)
    pane.unmount()
    assert.equal(terminal.disposed, true)
    assert.ok(terminalStreams.every((stream) => stream.closed))
  } finally {
    if (!terminals[0]?.disposed) pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('terminal list probes share a flight, obey the global concurrency budget, and cancel queued reads when hidden', async () => {
  prepareRuntime()
  terminals.length = terminalStreams.length = terminalFits = 0
  const originalFetch = globalThis.fetch
  const ids = Array.from({ length: 20 }, (_, index) => `probe-${index}`)
  const snapshot = {
    version: 1,
    updatedAt: 1,
    activeSessionId: ids[0],
    sessionIds: ids,
    sessionMetaById: Object.fromEntries(ids.map((id) => [id, { name: id }])),
    folders: [],
  }
  const probes: Array<{
    pending: ReturnType<typeof deferred<Response>>
    signal: AbortSignal
    settled: boolean
    id: string
  }> = []
  let active = 0,
    maximum = 0
  globalThis.fetch = ((input, init) => {
    const url = String(input)
    if (url.endsWith('/terminal/state')) return Promise.resolve(Response.json(snapshot))
    if (url.endsWith('/start')) return Promise.resolve(Response.json({ sessionId: ids[0], running: true }))
    if (url.endsWith('/resize')) return Promise.resolve(new Response(null))
    const pending = deferred<Response>()
    const probe = { pending, signal: init!.signal!, settled: false, id: url.split('/').pop()! }
    probes.push(probe)
    active++
    maximum = Math.max(maximum, active)
    const response = new Promise<Response>((resolve, reject) => {
      pending.promise.then(resolve, reject)
      probe.signal.addEventListener('abort', () => reject(probe.signal.reason), { once: true })
    })
    return response.finally(() => {
      active--
      probe.settled = true
    })
  }) as typeof fetch
  const pane = await mount('/src/pages/TerminalPage.vue', {}, ref(true), 'terminal', '/terminal')
  try {
    await flush()
    assert.equal(probes.length, 4)
    const shared = pane.state.refreshTrackedSessions()
    await flush()
    assert.equal(probes.length, 4)
    pane.shown.value = false
    await shared
    await flush()
    assert.ok(probes.every((probe) => probe.signal.aborted))
    assert.equal(probes.length, 4, 'queued requests must never dispatch after the pane hides')
    assert.equal(pane.state.sessionList.value.length, 20, 'cancellation must not remove existing terminal sessions')
    pane.shown.value = true
    for (let round = 0; round < 12; round++) {
      await flush()
      for (const probe of probes)
        if (!probe.settled) probe.pending.resolve(Response.json({ sessionId: probe.id, running: true }))
    }
    await pane.state.initializeTerminal()
    assert.equal(probes.filter((probe) => !probe.signal.aborted).length, 20)
    assert.ok(maximum <= 4)
  } finally {
    pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('a cancelled terminal bootstrap cannot dispatch follow-up reads or start a terminal after unmount', async () => {
  prepareRuntime()
  terminals.length = terminalStreams.length = terminalFits = 0
  const originalFetch = globalThis.fetch
  const pending = deferred<Response>()
  const requests: Array<{ input: string; signal: AbortSignal }> = []
  globalThis.fetch = ((input, init) => {
    requests.push({ input: String(input), signal: init!.signal! })
    return pending.promise
  }) as typeof fetch
  const pane = await mount('/src/pages/TerminalPage.vue', {}, ref(true), 'terminal', '/terminal')
  try {
    await flush()
    const initialization = pane.state.initializeTerminal()
    assert.equal(requests.length, 1)
    pane.shown.value = false
    pane.unmount()
    assert.equal(requests[0]!.signal.aborted, true)
    pending.resolve(Response.json({ version: 1, activeSessionId: 'late', sessionIds: ['late'] }))
    await initialization
    await flush()
    assert.equal(requests.length, 1)
    assert.equal(terminalStreams.length, 0)
    assert.equal(pane.state.terminalStateHydrated.value, false)
    assert.equal(terminals[0]?.disposed, true)
  } finally {
    if (!terminals[0]?.disposed) pane.unmount()
    globalThis.fetch = originalFetch
  }
})

test('manual permission refresh is serialized, hidden reads stop, and temporary failures keep loaded data', async () => {
  prepareRuntime()
  const originalFetch = globalThis.fetch
  const requests: Array<{ signal: AbortSignal; pending: ReturnType<typeof deferred<Response>> }> = []
  globalThis.fetch = ((_input, init) => {
    const pending = deferred<Response>()
    requests.push({ signal: init!.signal!, pending })
    return pending.promise
  }) as typeof fetch
  const pane = await mount('/src/components/settings/PermissionsPanel.vue', {}, ref(false))
  try {
    await flush()
    assert.equal(requests.length, 0)
    pane.shown.value = true
    const initial = pane.state.refresh()
    await flush()
    requests[0]!.pending.resolve(Response.json({ items: [{ id: 1, tool_name: 'keep this rule' }] }))
    await initial
    await flush()
    // A refresh during the first flight leaves exactly one trailing read.
    if (requests[1]) requests[1].pending.resolve(Response.json({ items: [{ id: 1, tool_name: 'keep this rule' }] }))
    await flush()
    const before = requests.length
    const retry = pane.state.refresh()
    await flush()
    assert.equal(pane.state.rules.value[0].tool_name, 'keep this rule')
    requests[before]!.pending.resolve(Response.json({ message: 'temporary failure' }, { status: 500 }))
    await retry
    assert.equal(pane.state.rules.value[0].tool_name, 'keep this rule')
    assert.match(pane.state.error.value, /temporary failure/)
    const pendingRead = pane.state.refresh()
    await flush()
    const active = requests.at(-1)!
    pane.shown.value = false
    assert.equal(active.signal.aborted, true)
    active.pending.resolve(Response.json({ items: [] }))
    await pendingRead
    assert.equal(pane.state.rules.value.length, 1)
  } finally {
    pane.unmount()
    globalThis.fetch = originalFetch
  }
})
