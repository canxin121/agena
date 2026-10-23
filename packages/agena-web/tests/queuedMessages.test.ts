import { afterAll, expect, test } from 'bun:test'
import { createPinia } from 'pinia'
import { computed, createSSRApp, reactive } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { entriesFromParts } from '../src/stores/chat/api'
import { createTranscriptProjector, projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'
import { queuedMessagePreviews } from '../src/pages/chat/queuedMessages'
import { foldedMessageCount, upsertMessageEntryIn } from '../src/stores/chat/messageIndex'

const queue = [
  { execution_id: 'next', message_id: 2 },
  { execution_id: 'last', message_id: 3 },
]
const user = (id: number, pending: boolean, text = `message ${id}`) => ({
  part_id: id,
  run_id: id,
  message_id: id,
  message_boundary: true,
  kind: 'text',
  role: 'user',
  state: pending ? 'pending' : 'completed',
  run_state: pending ? 'pending' : 'completed',
  revision: pending ? 1 : 2,
  updated_at_ms: pending ? 1 : 2,
  content: { text, queue_only: pending, execution_id: id === 2 ? 'next' : 'last' },
})
const answer = (id: number, messageId: number) => ({
  part_id: id,
  run_id: id,
  message_id: messageId,
  message_boundary: true,
  kind: 'text',
  role: 'assistant',
  state: 'running',
  run_state: 'running',
  content: { text: `answer ${messageId}` },
})

test('queued inputs stay in FIFO previews while the current reply stays beside its input', () => {
  const messages = entriesFromParts('1', [user(1, false), user(2, true), user(3, true), answer(4, 1)])
  expect(projectTranscriptBlocks(messages).map((block) => block.message.info.id)).toEqual(['1', '4'])
  expect(foldedMessageCount(messages)).toBe(2)
  expect(queuedMessagePreviews(messages, queue)).toEqual([
    { messageId: '2', executionId: 'next', preview: 'message 2' },
    { messageId: '3', executionId: 'last', preview: 'message 3' },
  ])
  // A pending boundary can lead a delayed execution shell.
  expect(queuedMessagePreviews(messages, [])).toHaveLength(2)
})

test('claiming a queue item moves it after the entire preceding reply without rebuilding older blocks', () => {
  const messages = reactive(entriesFromParts('1', [user(1, false), user(2, true), user(3, true), answer(4, 1)]))
  const project = createTranscriptProjector(() => ({ showReasoning: true }))
  const blocks = computed(() => project(messages))
  const before = blocks.value
  upsertMessageEntryIn(messages, entriesFromParts('1', [user(2, false)])[0]!.info)
  expect(blocks.value.map((block) => block.message.info.id)).toEqual(['1', '4', '2'])
  expect(blocks.value[0]).toBe(before[0])
  expect(blocks.value[1]).toBe(before[1])
  // A stale shell cannot keep a visibly started input in the preview.
  expect(queuedMessagePreviews(messages, queue)).toEqual([
    { messageId: '3', executionId: 'last', preview: 'message 3' },
  ])
  upsertMessageEntryIn(messages, entriesFromParts('1', [user(2, true)])[0]!.info)
  expect(blocks.value.map((block) => block.message.info.id)).toEqual(['1', '4', '2'])
  messages.push(entriesFromParts('1', [answer(5, 2)])[0]!)
  expect(blocks.value.map((block) => block.message.info.id)).toEqual(['1', '4', '2', '5'])
  // Reloaded history uses the same ownership regardless of wire append order.
  expect(projectTranscriptBlocks([...messages]).map((block) => block.message.info.id)).toEqual(['1', '4', '2', '5'])
})

test('withdrawn inputs stay out of history and out of the queue, and distinct replies never fold', () => {
  const withdrawn = user(2, true)
  withdrawn.run_state = 'cancelled'
  const messages = entriesFromParts('1', [user(1, false), withdrawn, answer(4, 1), answer(5, 3)])
  expect(projectTranscriptBlocks(messages).map((block) => block.message.info.id)).toEqual(['1', '4', '5'])
  expect(queuedMessagePreviews(messages, queue.slice(0, 1))).toEqual([])
})

test('a queued retry previews the original input while retaining its existing conversation turn', () => {
  const retry = user(2, false)
  Object.assign(retry.content, { execution_phase: 'queued' })
  const messages = entriesFromParts('1', [retry, answer(4, 2)])
  expect(projectTranscriptBlocks(messages)).toHaveLength(2)
  expect(queuedMessagePreviews(messages, queue.slice(0, 1))).toEqual([
    { messageId: '2', executionId: 'next', preview: 'message 2' },
  ])
})

test('queue previews load by backend identity and represent attachments, multiline input and duplicates', () => {
  const attachment = user(2, true, '')
  Object.assign(attachment.content, {
    document: [
      {
        type: 'activity',
        activity: {
          payload: {
            activity_type: 'resource',
            name: 'report.pdf',
          },
        },
      },
    ],
  })
  const messages = entriesFromParts('1', [attachment, user(3, true, '  line one\n\nline two  ')])
  expect(queuedMessagePreviews(messages, [...queue, { execution_id: 'missing', message_id: 8 }])).toEqual([
    { messageId: '2', executionId: 'next', preview: 'report.pdf' },
    { messageId: '3', executionId: 'last', preview: 'line one line two' },
    { messageId: '8', executionId: 'missing', preview: '' },
  ])
  expect(
    queuedMessagePreviews(entriesFromParts('1', [user(2, true, '你好'), user(3, true, '你好')]), queue),
  ).toHaveLength(2)
})

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  optimizeDeps: { noDiscovery: true, entries: [] },
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())

test('queue renders as a bounded ordered preview, without ordinary message roles or conversation actions', async () => {
  const { default: QueuedMessages } = await vite.ssrLoadModule('/src/components/chat/QueuedMessages.vue')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  const render = async (
    messages: Array<{ messageId: string; executionId: string; preview: string }>,
    paused = false,
    busy = false,
  ) => {
    const app = createSSRApp(QueuedMessages, { messages, paused, busy })
    app.use(createPinia())
    app.use(i18n)
    return renderToString(app)
  }
  const locale = i18n.global.locale.value
  try {
    i18n.global.locale.value = 'zh-CN'
    const html = await render([
      { messageId: '2', executionId: 'next', preview: '你好' },
      { messageId: '3', executionId: 'last', preview: '<script>alert(1)</script>' },
    ])
    expect(html).toContain('待发消息 · 2')
    expect(html).toContain('当前回复结束后按顺序发送')
    expect(html).toContain('<ol')
    expect(html.match(/<li /g)).toHaveLength(2)
    expect(html).toContain('overflow-y-auto')
    expect(html).toContain('&lt;script&gt;')
    expect(html).not.toContain('<script>')
    expect(html).not.toContain('data-message-role')
    expect(html).toContain('取回输入框编辑')
    expect(html).toContain('删除这条待发消息')
    expect(html).toContain('清空队列')
    expect(html).toContain('暂停队列')
    expect(await render([])).not.toContain('data-queued-messages')
    const paused = await render([{ messageId: '2', executionId: 'next', preview: '保留' }], true, true)
    expect(paused).toContain('已暂停')
    expect(paused).toContain('恢复发送')
    expect(paused).toContain('消息已保留')
    expect(paused).toContain('disabled')
    expect(await render([], true)).toContain('恢复发送')
  } finally {
    i18n.global.locale.value = locale
  }
})
