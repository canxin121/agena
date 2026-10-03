import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createRenderer, createSSRApp, h, nextTick, reactive, ssrContextKey, type Ref } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createPinia } from 'pinia'
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
  app.use(createPinia())
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': messages } }))
  return renderToString(app)
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
  app.provide(ssrContextKey, {})
  app.use(createPinia())
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
  const requests: Array<{ url: string; resolve: (response: Response) => void }> = []
  const originalFetch = globalThis.fetch
  globalThis.fetch = ((url: string | URL | Request) =>
    new Promise<Response>((resolve) => requests.push({ url: String(url), resolve }))) as typeof fetch
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
    assert.equal(requests.length, 1)
    props.part.status = 'completed'
    await nextTick()
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
    assert.equal(requests.length, 3, 'reopening refreshes a previously open child')
    requests[2]!.resolve(Response.json({ message: 'temporary failure' }, { status: 500 }))
    await flush()
    assert.match(state!.sectionErrors.value.output!, /temporary failure/)
    assert.equal(state!.loadingSections.value.size, 0)
    const retry = state!.loadSection('output')
    reply(3, 'retried result')
    await retry
    assert.equal(state!.sectionValues.value.output, 'retried result')
    assert.equal(state!.sectionErrors.value.output, '')

    const input = state!.toggleSection('input')
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
    assert.equal(state!.detailsExpanded.value, false)
  } finally {
    app.unmount()
    globalThis.fetch = originalFetch
  }
})
