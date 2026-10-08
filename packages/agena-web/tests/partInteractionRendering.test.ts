import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { computed, createRenderer, h, nextTick, reactive, ssrContextKey } from 'vue'
import { createI18n } from 'vue-i18n'
import type { MessageLike, TranscriptDisplayPart } from '../src/components/chat/messageList.types'
import { projectLocalPart } from '../src/pages/chat/transcriptProjection'
import { ensureBrowserTestRuntime } from './testRuntime'

ensureBrowserTestRuntime()
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  plugins: [
    {
      name: 'part-test-memory-history',
      transform(code, id) {
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
      },
    },
  ],
})
after(() => vite.close())
const { default: component } = await vite.ssrLoadModule('/src/components/chat/MessageItem.vue')
const { useChatRenderBlocks } = await vite.ssrLoadModule('/src/pages/chat/useChatRenderBlocks.ts')
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
const settle = async () => {
  for (let i = 0; i < 5; i++) await nextTick()
}

function part(id: number): TranscriptDisplayPart {
  return projectLocalPart(
    {
      id: String(id),
      agenaKind: 'tool_call',
      partState: 'in_progress',
      agenaContent: { name: 'shell.exec' },
      agenaPresentation: { title: `Part ${id}`, blocks: [] },
    },
    'assistant',
  )
}

function mount(initial: TranscriptDisplayPart[] = Array.from({ length: 12 }, (_, i) => part(i + 1))) {
  const expansions = reactive<Record<string, boolean>>({})
  let renderState: any
  const props = reactive({
    message: { info: { id: 'reply', role: 'assistant' }, parts: [] } as MessageLike,
    displayParts: initial,
    showTimestamps: false,
    formatTime: () => '',
    copiedMessageId: '',
    revertBusyMessageId: '',
    isStreaming: true,
    collapseSignal: 0,
    activityPageSize: 5,
    isCompactTouch: false,
    isPartExpanded: (part: TranscriptDisplayPart) => renderState.transcriptPartExpanded(part),
    foldLoadingByKey: {} as Record<string, boolean>,
  })
  let state: any
  const subject = {
    ...component,
    setup(props: object, context: object) {
      state = component.setup(props, context)
      return () => null
    },
  }
  const app = renderer.createApp({
    setup() {
      renderState = useChatRenderBlocks({
        chat: { messages: [] },
        settings: {},
        showThinking: computed(() => true),
        formatTime: () => '',
      })
      renderState.activityExpandedByBlockKey.value = expansions
      return () =>
        h(subject, {
          ...props,
          onPartToggle: (part: TranscriptDisplayPart, expanded: boolean) => {
            renderState.setActivityExpanded(part.key, expanded)
          },
        })
    },
  })
  app.provide(ssrContextKey, {})
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
  app.mount({})
  return {
    app,
    props,
    expansions,
    renderState,
    state,
    ids: () =>
      Array.from(
        state.transcriptRows.value.filter((row: any) => row.kind === 'part').map((row: any) => row.part.id),
      ) as string[],
  }
}

test('first explicit choices reactively close default-open parts and open default-closed siblings', async () => {
  const parts = Array.from({ length: 3 }, (_, i) => part(i + 1))
  parts[1]!.defaultExpanded = true
  const subject = mount(parts)
  const rendered = computed(() => subject.props.displayParts.map(subject.props.isPartExpanded))
  try {
    assert.deepEqual(rendered.value, [false, true, false])
    // Establish an explicit choice on another row before trying the two
    // untouched siblings: no unrelated visibility update should be needed.
    subject.state.togglePart(parts[0])
    await settle()
    assert.deepEqual(rendered.value, [true, true, false])
    subject.state.togglePart(parts[1])
    await settle()
    assert.deepEqual(rendered.value, [true, false, false])
    subject.state.togglePart(parts[2])
    await settle()
    assert.deepEqual(rendered.value, [true, false, true])
    for (let revision = 1; revision <= 3; revision++) {
      subject.props.displayParts[1]!.source.revision = revision
      await settle()
      assert.deepEqual(rendered.value, [true, false, true])
    }
    const configured = computed(() => subject.renderState.transcriptPartExpanded(parts[2], true))
    assert.equal(configured.value, true)
    subject.state.togglePart(parts[2])
    await settle()
    assert.equal(configured.value, false, 'an explicit collapse overrides configured expanded defaults')
  } finally {
    subject.app.unmount()
  }
})

test('remote loading stays observable when its final page retires the fold', async () => {
  const subject = mount()
  const fold = { runId: 3, runIds: [3], anchorPartId: '1', hiddenCount: 3, nextCursor: 'before-1' }
  try {
    subject.props.message.folds = [fold]
    await settle()
    subject.state.revealSummary(subject.state.transcriptRows.value[0], true)
    subject.props.foldLoadingByKey[':3'] = true
    await settle()
    subject.props.message.folds = []
    await settle()
    assert.equal(subject.state.foldLoading(null), true)
    subject.state.collapseParts()
    await settle()
    assert.equal(subject.ids().length, 5, 'collapse is immediate even while a remote page is pending')
    subject.props.displayParts.unshift(part(0))
    await settle()
    assert.equal(subject.ids().length, 5, 'a late page does not reveal a collapsed prefix')
    subject.props.foldLoadingByKey[':3'] = false
    await settle()
    assert.equal(subject.state.foldLoading(null), false)
  } finally {
    subject.app.unmount()
  }
})

test('an opened shell remains mounted while revisions and new siblings arrive, including idle', async () => {
  const subject = mount()
  try {
    subject.state.togglePart(subject.props.displayParts[7])
    await settle()
    for (let revision = 1; revision <= 20; revision++) {
      subject.props.displayParts[7]!.source.revision = revision
      subject.props.displayParts.push(part(12 + revision))
      await settle()
      assert.ok(subject.ids().includes('8'), 'streaming must not fold the open shell out of the DOM')
    }
    subject.props.collapseSignal++
    await settle()
    assert.ok(subject.ids().includes('8'))
    assert.equal(subject.expansions['part:8'], true)
    subject.state.collapseParts()
    await settle()
    assert.deepEqual(subject.ids(), ['28', '29', '30', '31', '32'])
    assert.equal(subject.expansions['part:8'], true, 'list collapse retains the shell detail choice')
  } finally {
    subject.app.unmount()
  }
})

test('a Vim expansion pins the same suffix, and load-all keeps a collapse row', async () => {
  const subject = mount()
  try {
    subject.expansions['part:8'] = false
    await settle()
    subject.expansions['part:8'] = true
    await settle()
    subject.props.displayParts.push(part(13))
    await settle()
    assert.ok(subject.ids().includes('8'))
    const summary = subject.state.transcriptRows.value.find((row: any) => row.kind === 'summary')
    subject.state.revealSummary(summary, true)
    await settle()
    assert.equal(subject.ids().length, 13)
    const collapseRow = subject.state.transcriptRows.value.find((row: any) => row.kind === 'summary')
    assert.equal(collapseRow.hiddenCount, 0)
    assert.equal(subject.state.canCollapseParts.value, true)
    subject.state.collapseParts()
    await settle()
    assert.equal(subject.ids().length, 5)
  } finally {
    subject.app.unmount()
  }
})

test('a pending interaction stays reachable after collapse and uses an expanded default', async () => {
  const parts = Array.from({ length: 12 }, (_, i) => part(i + 1))
  parts[2] = projectLocalPart(
    {
      id: '3',
      agenaKind: 'tool_call',
      partState: 'in_progress',
      agenaContent: {
        name: 'interaction.ask',
        user_input: { requests: [{ request: { request_id: 'ask' }, reply: null }] },
      },
    },
    'assistant',
  )
  const subject = mount(parts)
  try {
    assert.equal(subject.props.isPartExpanded(subject.props.displayParts[2]!), true)
    assert.ok(subject.ids().includes('3'))
    const summary = subject.state.transcriptRows.value.find((row: any) => row.kind === 'summary')
    subject.state.revealSummary(summary, true)
    await settle()
    subject.state.collapseParts()
    await settle()
    assert.ok(subject.ids().includes('3'), 'an unanswered request remains available for recovery')
  } finally {
    subject.app.unmount()
  }
})

test('automatic idle folding preserves explicit open and closed detail choices', () => {
  let state: any
  const app = renderer.createApp({
    setup() {
      state = useChatRenderBlocks({
        chat: { messages: [] },
        settings: {},
        showThinking: computed(() => true),
        formatTime: () => '',
      })
      return () => null
    },
  })
  try {
    app.mount({})
    state.setActivityExpanded('part:open', true)
    state.setActivityExpanded('part:closed', false)
    state.collapseAllActivities()
    assert.deepEqual({ ...state.activityExpandedByBlockKey.value }, { 'part:open': true, 'part:closed': false })
  } finally {
    app.unmount()
  }
})

test('a new request reopens its previously collapsed operation, while repeated updates respect a manual collapse', async () => {
  const subject = mount()
  try {
    subject.expansions['part:8'] = false
    subject.props.displayParts[7]!.source.agenaContent = {
      name: 'shell.exec',
      user_input: {
        requests: [{ request: { request_id: 'new-request' }, reply: null }],
      },
    }
    await settle()
    assert.equal(subject.expansions['part:8'], true)
    subject.expansions['part:8'] = false
    for (let revision = 1; revision <= 3; revision++) {
      subject.props.displayParts[7]!.source.revision = revision
      await settle()
      assert.equal(subject.expansions['part:8'], false, 'the same request must not reopen on every streamed update')
    }
  } finally {
    subject.app.unmount()
  }
})
