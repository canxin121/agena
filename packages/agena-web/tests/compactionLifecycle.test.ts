import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { computed, createSSRApp, reactive, ref } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createPinia } from 'pinia'

import { entriesFromParts } from '../src/stores/chat/api'
import { createTranscriptProjector, projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'
import type { MessageLike } from '../src/components/chat/messageList.types'
import { ensureBrowserTestRuntime } from './testRuntime'
import { formatTranscript } from '../src/lib/transcript'

function transcript(state: string, detail = '', outcome = '') {
  const part = (id: number, kind: string, role: string, content: object, runId?: number) => ({
    part_id: id,
    kind,
    role,
    content,
    state: 'completed',
    run_id: runId,
    created_at_ms: id * 10,
    revision: 1,
  })
  return entriesFromParts('1', [
    part(1, 'run', 'user', { run_kind: 'user_send' }),
    part(2, 'text', 'user', { text: 'helli' }, 1),
    part(3, 'run', 'assistant', { run_kind: 'continue', reply_id: 'reply-1' }),
    part(4, 'text', 'assistant', { text: 'Hi! How can I help?' }, 3),
    { ...part(5, 'run', 'assistant', { run_kind: 'compaction', reply_id: 'reply-1' }), state },
    {
      ...part(
        6,
        'compaction',
        'assistant',
        {
          detail,
          outcome,
          checkpoint:
            outcome === 'compacted'
              ? { content: { type: 'text_summary', summary: 'Checkpoint summary' } }
              : { content: { type: 'open_ai_responses', items: [{ encrypted_content: 'opaque provider data' }] } },
        },
        5,
      ),
      state,
    },
  ] as never) as MessageLike[]
}

test('every compaction state stays on one assistant part without changing the completed answer', () => {
  for (const state of ['pending', 'in_progress', 'completed', 'failed', 'cancelled']) {
    const messages = transcript(state, 'Compaction detail')
    const blocks = projectTranscriptBlocks(messages)
    assert.equal(blocks.length, 2)
    assert.equal(blocks[1]?.message.info.runState, 'completed')
    assert.deepEqual(
      blocks[1]?.displayParts.map((part) => part.kind),
      ['answer', 'compaction'],
    )
    const part = blocks[1]!.displayParts[1]!
    assert.equal(part.key, 'part:6')
    assert.equal(part.status, state)
    assert.equal(part.role, 'assistant')
    assert.equal(part.summary, 'Compaction detail')
    assert.equal(part.defaultExpanded, false)
    assert.equal(part.toggleable, true)
    assert.ok(!part.copyText.includes('opaque provider data'))
    assert.deepEqual(
      projectTranscriptBlocks(messages.slice(-1))[0]!.displayParts.map((part) => part.kind),
      ['compaction'],
    )
  }
})

test('a streamed completion retains the same part key and the preceding answer object', () => {
  const messages = reactive(transcript('pending'))
  const project = createTranscriptProjector(() => ({ showReasoning: true }))
  const running = project(messages)
  const next = transcript('completed', '', 'unchanged').at(-1)!
  Object.assign(messages[2]!.info, next.info)
  Object.assign(messages[2]!.parts[0]!, next.parts[0])
  const completed = project(messages)
  assert.equal(completed[1]!.displayParts[0], running[1]!.displayParts[0])
  assert.equal(completed[1]!.displayParts[1]!.key, running[1]!.displayParts[1]!.key)
  assert.equal(completed[1]!.displayParts[1]!.status, 'completed')
  assert.equal(completed[1]!.displayParts.length, 2)
})

test('content after automatic compaction retains its durable order and native results stay opaque', () => {
  const messages = transcript('completed', '', 'compacted')
  const answer = messages[1]!
  const later = { ...answer.parts[0]!, id: '7', text: 'Final answer', agenaContent: { text: 'Final answer' } }
  answer.parts.push(later)
  const block = projectTranscriptBlocks(messages)[1]!
  assert.deepEqual(
    block.displayParts.map((part) => [part.id, part.kind]),
    [
      ['4', 'text_segment'],
      ['6', 'compaction'],
      ['7', 'answer'],
    ],
  )
  assert.equal(block.displayParts[1]!.summary, 'Checkpoint summary')
  const native = messages[2]!.parts[0]!
  native.agenaContent = {
    outcome: 'compacted',
    checkpoint: {
      before_tokens: 10000,
      after_tokens: 3000,
      content: { type: 'open_ai_responses', items: [{ encrypted_content: 'opaque provider data' }] },
    },
  }
  native.text = ''
  const projected = projectTranscriptBlocks(messages)[1]!.displayParts[1]!
  assert.equal(projected.summary, '10000 → 3000 tokens')
  assert.ok(!projected.copyText.includes('opaque provider data'))
  const exported = formatTranscript({ id: '1' }, messages as never, {
    thinking: true,
    toolDetails: true,
    assistantMetadata: false,
  })
  assert.ok(exported.includes('10000 → 3000 tokens'))
  assert.ok(!exported.includes('opaque provider data'))
})

test('a page beginning with compaction takes the latest maintenance or assistant state', () => {
  const failed = transcript('failed').slice(-1)
  const completed = transcript('completed').at(-1)!
  completed.info.id = '6'
  completed.parts[0]!.id = '6'
  const maintenance = projectTranscriptBlocks([...failed, completed])[0]!
  assert.equal(maintenance.message.info.runState, 'completed')
  assert.deepEqual(
    maintenance.displayParts.map((part) => part.kind),
    ['compaction', 'compaction'],
  )
  const answer = transcript('completed')[1]!
  answer.info.id = '7'
  const continued = projectTranscriptBlocks([...failed, answer])[0]!
  assert.equal(continued.message.info.runState, 'completed')
  assert.ok(!continued.displayParts.some((part) => part.kind === 'lifecycle'))
})

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  optimizeDeps: { noDiscovery: true, include: [] },
  logLevel: 'error',
  plugins: [
    {
      name: 'compaction-ssr-router',
      enforce: 'pre',
      transform(code, id) {
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
      },
    },
  ],
})
after(() => vite.close())

test('the real message list nests compaction beside the answer with identical part indentation', async () => {
  ensureBrowserTestRuntime()
  const events = new EventTarget()
  window.addEventListener ??= events.addEventListener.bind(events)
  window.removeEventListener ??= events.removeEventListener.bind(events)
  const { default: MessageList } = await vite.ssrLoadModule('/src/components/chat/MessageList.vue')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  i18n.global.locale.value = 'zh-CN'
  for (const state of ['pending', 'completed', 'failed', 'cancelled']) {
    const blocks = projectTranscriptBlocks(
      transcript(
        state,
        state === 'failed' ? 'Compaction provider unavailable' : '',
        state === 'completed' ? 'unchanged' : '',
      ),
      {
        showReasoning: true,
        labels: { answer: '回答', compaction: '上下文压缩', compactionUnchanged: '当前上下文已足够精简，无需压缩。' },
      },
    )
    const app = createSSRApp(MessageList, {
      selectedSessionId: '1',
      renderBlocks: blocks,
      isCompactLayout: false,
      isCompactTouch: false,
      messagesLoading: false,
      messagesError: null,
      pendingInitialScrollSessionId: null,
      loadingOlder: false,
      activityPageSize: 10,
      showTimestamps: true,
      formatTime: () => '16:02',
      copiedMessageId: '',
      revertBusyMessageId: '',
      isStreamingAssistantMessage: () => false,
      showAssistantPlaceholder: false,
      sessionEnded: true,
      retryStatus: null,
      currentPhase: '',
      activityCollapseSignal: 0,
      isPartExpanded: () => true,
    })
      .use(createPinia())
      .use(i18n)
    const html = await renderToString(app)
    const articles = html.match(/<article\b[\s\S]*?<\/article>/g) || []
    assert.equal(articles.length, 2)
    const assistant = articles[1]!
    assert.ok(assistant.includes('assistant'))
    assert.ok(assistant.includes('Hi! How can I help?'))
    assert.ok(assistant.includes('上下文压缩'))
    assert.equal(html.match(/data-part-kind="compaction"/g)?.length, 1)
    const wrappers = [...assistant.matchAll(/<div class="([^"]*)"[^>]*data-transcript-node="part"/g)]
    assert.equal(wrappers.length, 2)
    assert.equal(wrappers[0]![1], wrappers[1]![1], 'answer and compaction use the same part wrapper')
    assert.ok(!html.includes('回复进行中') && !html.includes('Response failed'))
    if (state === 'completed') assert.ok(html.includes('当前上下文已足够精简，无需压缩。'))
    if (state === 'failed') assert.ok(html.includes('Compaction provider unavailable'))
  }
})

test('fast completion, remount and terminal snapshots cannot leave a phantom compacting state', async () => {
  const { useChatSessionActions } = await vite.ssrLoadModule('/src/pages/chat/useChatSessionActions.ts')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  const chat = reactive({
    selectedSessionId: '1',
    selectedSession: { id: '1' },
    messages: transcript('completed'),
    renameSession: async () => null,
    forkSession: async () => null,
    compactSession: async () => {
      chat.messages = transcript('completed', '', 'unchanged')
      return null
    },
  })
  let actions: ReturnType<typeof useChatSessionActions>
  const app = createSSRApp({
    setup() {
      actions = useChatSessionActions({
        chat,
        toasts: { push() {} },
        sessionTitle: computed(() => 'session'),
        showThinking: ref(true),
        copyToClipboard: async () => {},
      })
      return () => null
    },
  }).use(i18n)
  await renderToString(app)
  await actions!.handleCompactSession()
  assert.equal(actions!.compactBusy.value, false)
  assert.equal(actions!.compactingSessionId.value, null)
  chat.messages = transcript('pending')
  assert.equal(actions!.compactingSessionId.value, '1')
  for (const state of ['completed', 'failed', 'cancelled']) {
    chat.messages = transcript(state)
    assert.equal(actions!.compactingSessionId.value, null)
  }
})
