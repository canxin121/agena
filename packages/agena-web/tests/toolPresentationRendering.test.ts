import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { computed, createRenderer, createSSRApp, h, nextTick, reactive, ref, ssrContextKey, type Ref } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createPinia, disposePinia } from 'pinia'
import { createI18n } from 'vue-i18n'
import { ensureBrowserTestRuntime } from './testRuntime'
import { transcriptDiffFiles } from '../src/pages/chat/transcriptDiff'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  plugins: [
    {
      name: 'ssr-memory-history',
      transform(code, id) {
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
      },
    },
  ],
})
after(() => vite.close())

function prepareRuntime() {
  ensureBrowserTestRuntime()
  Object.assign(document, {
    querySelector: () => null,
    compatMode: 'CSS1Compat',
    addEventListener() {},
    removeEventListener() {},
  })
  Object.assign(window, { addEventListener() {}, removeEventListener() {} })
}

async function render(path: string, props: Record<string, unknown>) {
  prepareRuntime()
  const { default: component } = await vite.ssrLoadModule(path)
  const { default: messages } = await vite.ssrLoadModule('/src/i18n/messages/en-US.ts')
  const app = createSSRApp(component, props)
  const pinia = createPinia()
  app.use(pinia)
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': messages } }))
  try {
    return await renderToString(app)
  } finally {
    disposePinia(pinia)
  }
}

function controlTimers() {
  const original = {
    now: Date.now,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    windowSetTimeout: window.setTimeout,
    windowClearTimeout: window.clearTimeout,
  }
  let now = original.now(),
    serial = 0
  const timers = new Map<number, { at: number; callback: () => void }>()
  Date.now = () => now
  globalThis.setTimeout = ((callback: () => void, delay = 0) => {
    const id = ++serial
    timers.set(id, { at: now + delay, callback })
    return id
  }) as typeof setTimeout
  globalThis.clearTimeout = ((id: number) => {
    timers.delete(id)
  }) as typeof clearTimeout
  window.setTimeout = globalThis.setTimeout as typeof window.setTimeout
  window.clearTimeout = globalThis.clearTimeout as typeof window.clearTimeout
  const flush = async () => {
    await new Promise<void>((resolve) => setImmediate(resolve))
    await nextTick()
  }
  return {
    async advance(ms: number) {
      const end = now + ms
      for (let iteration = 0; iteration < 1000; iteration++) {
        await flush()
        const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
        if (!next) {
          now = end
          await flush()
          return
        }
        now = next[1].at
        timers.delete(next[0])
        next[1].callback()
      }
      throw new Error('timers did not converge')
    },
    restore() {
      Date.now = original.now
      globalThis.setTimeout = original.setTimeout
      globalThis.clearTimeout = original.clearTimeout
      window.setTimeout = original.windowSetTimeout
      window.clearTimeout = original.windowClearTimeout
    },
  }
}

const diff = '--- a/src/a.rs\n+++ b/src/a.rs\n@@ -99,2 +99,2 @@\n-old\n+new\n context\n'

test('human output and actual diff remain visible while both technical levels are closed', async () => {
  const html = await render('/src/components/chat/AgenaOperationPart.vue', {
    expanded: true,
    collapseSignal: 0,
    sessionId: '7',
    part: {
      id: '4',
      kind: 'operation',
      status: 'completed',
      title: 'File edited',
      summary: '1 file changed',
      source: {
        id: '4',
        agenaKind: 'tool_call',
        agenaContent: {
          name: 'fs.replace',
          input: { old: 'private-input-sentinel' },
          metadata: { hash: 'metadata-sentinel' },
          output: { text: 'raw-output-sentinel' },
        },
        agenaPresentation: {
          title: 'File edited',
          summary: '1 file changed',
          blocks: [
            { type: 'markdown', text: '**Updated**\n\n- Keep the context.' },
            { type: 'file_changes', changes: [{ path: 'src/a.rs', kind: 'updated' }] },
            { type: 'diff', diff },
          ],
        },
      },
    },
  })
  assert.match(html, /<strong>Updated<\/strong>/)
  assert.match(html, /<li>Keep the context\./)
  assert.match(html, /data-transcript-diff/)
  assert.match(html, /aria-expanded="false"[^>]*data-tool-details-toggle/)
  assert.doesNotMatch(html, /data-tool-detail-section|private-input-sentinel|raw-output-sentinel|metadata-sentinel/)
  assert.equal((html.match(/src\/a\.rs/g) || []).length, 2) // one file heading and its accessible label
  assert.doesNotMatch(html, /1 file changed/)
})

test('delegated task log resources stream in the expanded parent part without a content presentation block', async () => {
  const html = await render('/src/components/chat/AgenaOperationPart.vue', {
    expanded: true,
    collapseSignal: 0,
    sessionId: '7',
    part: {
      id: '4',
      kind: 'operation',
      status: 'in_progress',
      title: 'Review project',
      summary: '',
      source: {
        id: '4',
        agenaKind: 'tool_call',
        agenaContent: {
          name: 'tasks.run',
          input: { description: 'Review project' },
          metadata: { child_session_id: 9, task_id: 'task-9' },
          resources: [{ resource_id: 'task-output', kind: 'log' }],
        },
        agenaPresentation: { blocks: [{ type: 'nested_task', task_id: 'task-9', title: 'Review project' }] },
      },
    },
  })
  assert.equal((html.match(/data-operation-content-output/g) || []).length, 1)
  assert.equal((html.match(/data-content-output/g) || []).length, 1)
  assert.match(html, /data-tool-details-toggle/)
  assert.doesNotMatch(html, /data-tool-detail-section/)
})

test('an operation content block is not duplicated from the part resource list', async () => {
  const html = await render('/src/components/chat/AgenaOperationPart.vue', {
    expanded: true,
    collapseSignal: 0,
    sessionId: '7',
    part: {
      id: '5',
      kind: 'operation',
      status: 'in_progress',
      title: 'Shell',
      summary: '',
      source: {
        id: '5',
        agenaKind: 'tool_call',
        agenaContent: {
          name: 'shell.exec',
          resources: [{ resource_id: 'shell-output', kind: 'log' }],
        },
        agenaPresentation: {
          blocks: [{ type: 'content', resource: { resource_id: 'shell-output', kind: 'log' } }],
        },
      },
    },
  })
  assert.equal((html.match(/data-content-output/g) || []).length, 1)
  assert.doesNotMatch(html, /data-operation-content-output/)
})

test('a running delegated task renders its child-session log resource through the live content viewer', async () => {
  const html = await render('/src/components/chat/AgenaOperationPart.vue', {
    expanded: true,
    collapseSignal: 0,
    sessionId: '7',
    part: {
      id: '6',
      kind: 'operation',
      status: 'in_progress',
      title: 'Review project',
      summary: 'running',
      source: {
        id: '6',
        agenaKind: 'tool_call',
        agenaContent: {
          plugin: 'agena.tasks',
          name: 'run',
          metadata: { child_session_id: 9, task_id: 'task-9' },
          resources: [{ resource_id: 'task-output', kind: 'log' }],
        },
        agenaPresentation: {
          blocks: [
            { type: 'nested_task', task_id: 'task-9', title: 'Review project', status: 'running' },
            { type: 'content', id: 'output:task-output', resource: { resource_id: 'task-output', kind: 'log' } },
          ],
        },
      },
    },
  })
  assert.equal((html.match(/data-content-output/g) || []).length, 1)
  assert.doesNotMatch(html, /data-operation-content-output/)
  assert.match(html, /data-tool-presentation/)
})

test('diff renderer gives long patches a visible left-aligned expansion control and escapes source HTML', async () => {
  const source = `--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,100 @@\n${Array.from({ length: 100 }, (_, i) => `+line ${i} <img onerror="bad">`).join('\n')}`
  const html = await render('/src/components/chat/AgenaDiffBlock.vue', { diff: source })
  assert.equal((html.match(/data-diff-line="added"/g) || []).length, 79)
  assert.match(html, /Show 21 more lines/)
  assert.match(html, /&lt;img/)
  assert.doesNotMatch(html, /<img|justify-end|float-right/)
  assert.match(html, /\+100/)
})

test('multi-file diffs preserve hunk line numbers, header-like source, deletion and newline markers', () => {
  const files = transcriptDiffFiles(
    '--- a/old.rs\n+++ b/new.rs\n@@ -10,2 +20,2 @@\n--- source\n+++ source\n context\n--- a/deleted.py\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n\\ No newline at end of file\n',
  )
  assert.deepEqual(
    files.map((f) => [f.path, f.additions, f.deletions]),
    [
      ['new.rs', 1, 1],
      ['deleted.py', 0, 1],
    ],
  )
  assert.deepEqual(files[0]!.rows[1], { kind: 'removed', text: '-- source', oldLine: 10, newLine: null })
  assert.deepEqual(files[0]!.rows[2], { kind: 'added', text: '++ source', oldLine: null, newLine: 20 })
  assert.equal(files[1]!.rows.at(-1)!.kind, 'note')
  assert.deepEqual(
    files.map((file) => file.change),
    ['R', 'D'],
  )
  assert.equal(transcriptDiffFiles('+++ b/新.txt\n@@\n+\n+你好\n')[0]!.rows[2]!.newLine, 2)
})

test('diff file operations include empty additions, deletions and renames', () => {
  const files = transcriptDiffFiles(
    'diff --git a/empty b/empty\nnew file mode 100644\n' +
      'diff --git a/gone b/gone\ndeleted file mode 100644\n' +
      'diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n',
  )
  assert.deepEqual(
    files.map((file) => [file.path, file.change, file.rows.length]),
    [
      ['empty', 'A', 0],
      ['gone', 'D', 0],
      ['new', 'R', 0],
    ],
  )
  assert.equal(transcriptDiffFiles(diff)[0]!.change, 'M')
  assert.deepEqual(
    transcriptDiffFiles('--- /dev/null\n+++ b/empty\n--- a/gone\n+++ /dev/null\n').map((file) => [
      file.path,
      file.change,
    ]),
    [
      ['empty', 'A'],
      ['gone', 'D'],
    ],
  )
})

test('technical details load only visible children, reject stale responses and support retry', async () => {
  prepareRuntime()
  const { default: component } = await vite.ssrLoadModule('/src/components/chat/AgenaOperationPart.vue')
  type DetailState = {
    detailsExpanded: Ref<boolean>
    outputExpanded: Ref<boolean>
    sectionValues: Ref<Record<string, unknown>>
    sectionErrors: Ref<Record<string, string>>
    loadingSections: Ref<Set<string>>
    toggleDetails(): void
    toggleSection(section: string): Promise<void>
    loadSection(section: string): Promise<void>
  }
  let state: DetailState
  const subject = {
    ...component,
    setup(props: object, context: object) {
      state = component.setup(props, context)
      return () => null
    },
  }
  // Run the component's real lifecycle and watches without a browser; the
  // separate SSR tests above exercise its actual template and child renderers.
  const renderer = createRenderer<object, object>({
    patchProp() {},
    insert() {},
    remove() {},
    setText() {},
    setElementText() {},
    createElement: () => ({}),
    createText: () => ({}),
    createComment: () => ({}),
    parentNode: () => null,
    nextSibling: () => null,
  })
  const props = reactive({
    part: { id: '4', status: 'in_progress', source: {} },
    expanded: true,
    collapseSignal: 0,
    sessionId: '7',
  })
  const app = renderer.createApp({ setup: () => () => h(subject, props) })
  const { workspacePaneContextKey } = await vite.ssrLoadModule('/src/app/workspace/workspacePaneContext.ts')
  const paneVisible = ref(true)
  app.provide(workspacePaneContextKey, {
    windowId: computed(() => 'tool-detail-pane'),
    isFocused: computed(() => true),
    isVisible: computed(() => paneVisible.value),
    route: computed(() => ({ path: '/chat', query: {} })),
    navigate: async () => {},
  })
  app.provide(ssrContextKey, {})
  const pinia = createPinia()
  app.use(pinia)
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
  const requests: Array<{ url: string; resolve: (response: Response) => void }> = []
  const originalFetch = globalThis.fetch
  const clock = controlTimers()
  globalThis.fetch = ((url: string | URL | Request, init?: RequestInit) =>
    new Promise<Response>((resolve, reject) => {
      requests.push({ url: String(url), resolve })
      init?.signal?.addEventListener('abort', () => reject(init.signal!.reason), { once: true })
    })) as typeof fetch
  const reply = (index: number, value: unknown) => {
    const request = requests[index]!
    const match = /\/parts\/(\d+)\/tool-sections\/(\w+)/.exec(request.url)!
    request.resolve(Response.json({ part_id: Number(match[1]), section: match[2], value }))
  }
  const flush = async () => {
    await new Promise<void>((resolve) => setImmediate(resolve))
    await nextTick()
  }
  try {
    app.mount({})
    assert.equal(requests.length, 0)
    state!.toggleDetails()
    await nextTick()
    assert.equal(requests.length, 0, 'opening the parent must not load collapsed children')
    const oldOutput = state!.toggleSection('output')
    await flush()
    assert.equal(requests.length, 1)
    props.part.status = 'completed'
    await nextTick()
    await clock.advance(1000)
    assert.equal(requests.length, 2)
    reply(1, 'completed result')
    await flush()
    reply(0, 'obsolete running result')
    await oldOutput
    assert.equal(state!.sectionValues.value.output, 'completed result')

    state!.toggleDetails()
    props.part.status = 'in_progress'
    await nextTick()
    assert.equal(requests.length, 2, 'closed parent must defer refresh')
    state!.toggleDetails()
    await nextTick()
    await clock.advance(1000)
    assert.equal(requests.length, 3, 'reopening refreshes a previously open child')
    requests[2]!.resolve(Response.json({ message: 'temporary failure' }, { status: 500 }))
    await flush()
    assert.match(state!.sectionErrors.value.output!, /temporary failure/)
    assert.equal(state!.loadingSections.value.size, 0)
    const retry = state!.loadSection('output')
    await flush()
    assert.equal(requests.length, 3, 'an explicit retry respects failure backoff')
    await clock.advance(5100)
    reply(3, 'retried result')
    await retry
    await flush()
    assert.equal(state!.sectionValues.value.output, 'retried result')
    assert.equal(state!.sectionErrors.value.output, '')

    const input = state!.toggleSection('input')
    await clock.advance(1000)
    props.part.id = '5'
    await nextTick()
    reply(4, 'previous part input')
    await input
    assert.equal(Object.keys(state!.sectionValues.value).length, 0)
    assert.equal(state!.detailsExpanded.value, false)
    assert.equal(state!.outputExpanded.value, false)
    state!.toggleDetails()
    props.collapseSignal++
    await nextTick()
    assert.equal(state!.detailsExpanded.value, true, 'an idle transition retains explicitly opened details')

    paneVisible.value = false
    const hiddenOutput = state!.toggleSection('output')
    await flush()
    await clock.advance(2100)
    assert.equal(requests.length, 5, 'a hidden pane must not start a direct or scheduled detail read')
    await hiddenOutput
    paneVisible.value = true
    await clock.advance(1000)
    assert.equal(requests.length, 6, 'revealing the pane resumes its open disclosures')
    reply(5, 'visible again')
    await flush()
    assert.equal(state!.sectionValues.value.output, 'visible again')
  } finally {
    app.unmount()
    disposePinia(pinia)
    clock.restore()
    globalThis.fetch = originalFetch
  }
})

test('a real expanded tool subscribes by section and output changes read no input or metadata after cache eviction', async () => {
  prepareRuntime()
  const { default: component } = await vite.ssrLoadModule('/src/components/chat/AgenaOperationPart.vue')
  const resourceSync = (await vite.ssrLoadModule(
    '/src/lib/resourceSync.ts',
  )) as typeof import('../src/lib/resourceSync')
  const { conditionalJson } = (await vite.ssrLoadModule(
    '/src/lib/conditionalJson.ts',
  )) as typeof import('../src/lib/conditionalJson')
  let state!: {
    toggleDetails(): void
    toggleSection(section: string): Promise<void>
    sectionValues: Ref<Record<string, unknown>>
  }
  const subject = {
    ...component,
    setup(props: object, context: object) {
      state = component.setup(props, context)
      return () => null
    },
  }
  const renderer = createRenderer<object, object>({
    patchProp() {},
    insert() {},
    remove() {},
    setText() {},
    setElementText() {},
    createElement: () => ({}),
    createText: () => ({}),
    createComment: () => ({}),
    parentNode: () => null,
    nextSibling: () => null,
  })
  const props = reactive({
    part: { id: '8401', status: 'in_progress', source: { revision: 1, updatedAt: 1, partState: 'in_progress' } },
    expanded: true,
    collapseSignal: 0,
    sessionId: '8400',
  })
  const app = renderer.createApp({ setup: () => () => h(subject, props) })
  app.provide(ssrContextKey, {})
  const pinia = createPinia()
  app.use(pinia)
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
  const originalFetch = globalThis.fetch
  const clock = controlTimers()
  const calls: string[] = []
  const versions = new Map(['input', 'metadata', 'output'].map((section) => [`part:8401:${section}`, 'tool-grain:1']))
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'http://agena.test')
    if (url.pathname === '/api/v1/changes/revisions')
      return Response.json(
        Object.fromEntries(
          JSON.parse(url.searchParams.get('resources')!).map((key: string) => [
            key,
            versions.get(key) || 'tool-grain:1',
          ]),
        ),
      )
    const match = /\/tool-sections\/(\w+)/.exec(url.pathname)
    if (match) {
      calls.push(match[1]!)
      return Response.json(
        {
          part_id: 8401,
          section: match[1],
          value: { version: versions.get(`part:8401:${match[1]}`) },
          part_state: props.part.source.partState,
          revision: 1,
          updated_at_ms: props.part.source.updatedAt,
        },
        { headers: { etag: `W/"${versions.get(`part:8401:${match[1]}`)}"` } },
      )
    }
    if (url.pathname.startsWith('/evict-tool/')) return Response.json({}, { headers: { etag: 'W/"tool-grain:1"' } })
    throw new Error(`Unexpected request ${url}`)
  }) as typeof fetch
  try {
    app.mount({})
    state.toggleDetails()
    await nextTick()
    for (const section of ['input', 'metadata', 'output']) {
      const opening = state.toggleSection(section)
      await clock.advance(1100)
      await opening
    }
    assert.deepEqual(calls.sort(), ['input', 'metadata', 'output'])
    const savedInput = state.sectionValues.value.input
    const savedMetadata = state.sectionValues.value.metadata
    await Promise.all(Array.from({ length: 90 }, (_, i) => conditionalJson('sessions', `/evict-tool/${i}`)))
    calls.length = 0
    versions.set('part:8401:output', 'tool-grain:2')
    resourceSync.applyResourceEvent({
      type: 'session_changed',
      properties: { kind: 'part_updated', session_id: 8400, resource_revisions: Object.fromEntries(versions) },
    })
    props.part.source.updatedAt = 2
    await clock.advance(2100)
    assert.deepEqual(calls, ['output'])
    assert.equal(state.sectionValues.value.input, savedInput)
    assert.equal(state.sectionValues.value.metadata, savedMetadata)
    assert.equal((state.sectionValues.value.output as { version: string }).version, 'tool-grain:2')

    await state.toggleSection('input')
    await nextTick()
    calls.length = 0
    versions.set('part:8401:input', 'tool-grain:3')
    versions.set('part:8401:output', 'tool-grain:3')
    resourceSync.applyResourceEvent({
      type: 'session_changed',
      properties: { kind: 'part_updated', resource_revisions: Object.fromEntries(versions) },
    })
    await clock.advance(2100)
    assert.deepEqual(calls, ['output'], 'a closed input disclosure sends no request even when its own token changes')

    calls.length = 0
    // The transcript has caught up through HTTP while the detail clock is
    // still fresh in the browser: the completion SSE was lost. A cached
    // running output must not defer the final result until the heartbeat.
    versions.set('part:8401:output', 'tool-grain:4')
    props.part.status = 'completed'
    props.part.source.partState = 'completed'
    props.part.source.updatedAt = 4
    await clock.advance(2100)
    assert.deepEqual(calls, ['output'], 'a missing terminal hint refreshes output without reading metadata or input')
    assert.equal((state.sectionValues.value.output as { version: string }).version, 'tool-grain:4')
    assert.equal(state.sectionValues.value.metadata, savedMetadata)
    calls.length = 0
    await clock.advance(2100)
    assert.equal(calls.length, 0, 'an output that has reached the terminal state needs no further body read')
  } finally {
    app.unmount()
    disposePinia(pinia)
    clock.restore()
    globalThis.fetch = originalFetch
  }
})
