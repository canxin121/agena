import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createPinia, setActivePinia } from 'pinia'
import { createSSRApp } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createI18n } from 'vue-i18n'
import { ensureBrowserTestRuntime } from './testRuntime'
import type { useChatStore as ChatStoreFactory } from '../src/stores/chat'
import { transcriptFoldKey } from '../src/stores/chat/transcriptFolds'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  plugins: [
    {
      name: 'ssr-test-memory-history',
      transform(code, id) {
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
      },
    },
  ],
})
after(() => vite.close())
const { useChatStore } = (await vite.ssrLoadModule('/src/stores/chat.ts')) as { useChatStore: typeof ChatStoreFactory }

test('the rendered reply exposes left-aligned busy/retry controls even with a filtered-out anchor', async () => {
  const { default: MessageItem } = await vite.ssrLoadModule('/src/components/chat/MessageItem.vue')
  const replyFold = { runId: 3, runIds: [3], anchorPartId: '14', hiddenCount: 10, nextCursor: 'before-14' }
  const render = async (loading: boolean, error = '') => {
    const app = createSSRApp(MessageItem, {
      message: { info: { id: '3', sessionID: '7', role: 'assistant' }, parts: [], folds: [replyFold] },
      displayParts: [],
      sessionId: '7',
      showTimestamps: false,
      formatTime: () => '',
      copiedMessageId: '',
      revertBusyMessageId: '',
      isStreaming: false,
      collapseSignal: 0,
      activityPageSize: 5,
      isCompactTouch: false,
      isPartExpanded: () => false,
      foldLoadingByKey: { '7:3': loading },
      foldErrorByKey: { '7:3': error },
    })
    app.use(
      createI18n({
        legacy: false,
        locale: 'en',
        missingWarn: false,
        fallbackWarn: false,
        messages: { en: { common: { loading: 'Loading' } } },
      }),
    )
    return renderToString(app)
  }
  const busy = await render(true)
  assert.match(busy, /data-part-expand-next="true"/)
  assert.match(busy, /Loading/)
  assert.doesNotMatch(busy, /data-part-page-size/)
  assert.equal((busy.match(/\sdisabled(?:="")?(?=\s|>)/g) || []).length, 2)
  assert.doesNotMatch(busy, /justify-end/)
  const retry = await render(false, 'Temporary failure')
  assert.match(retry, /role="alert"[^>]*>Temporary failure/)
  assert.equal((retry.match(/\sdisabled(?:="")?(?=\s|>)/g) || []).length, 0)
  // The editable page size sits inside the expand label: lead text, then the
  // number input, then the trailing label.
  const leadIndex = retry.indexOf('expandNextLead')
  const inputIndex = retry.indexOf('data-part-page-size="true"')
  const tailIndex = retry.indexOf('expandNextTail')
  assert.ok(leadIndex >= 0 && inputIndex > leadIndex && tailIndex > inputIndex)
})

const part = (id: number) => ({
  part_id: id,
  run_id: id === 3 ? null : 3,
  kind: id === 3 ? 'run' : 'text',
  role: 'assistant',
  state: 'completed',
  content: { text: `part ${id}` },
  created_at_ms: id,
})
const fold = (anchor = 14, hidden = 10) => ({
  run_id: 3,
  run_ids: [3],
  anchor_part_id: anchor,
  hidden_count: hidden,
  next_cursor: `before-${anchor}`,
})
const page = (ids: number[], cursor: string | null, folds: ReturnType<typeof fold>[] = []) => ({
  parts: ids.map(part),
  folds,
  user_message_count: 10,
  page: { has_more: cursor !== null, next_cursor: cursor },
})
const response = (body: unknown) =>
  new Response(JSON.stringify(body), { headers: { 'content-type': 'application/json' } })

function harness(handler: (url: URL) => Promise<Response> | Response) {
  ensureBrowserTestRuntime()
  setActivePinia(createPinia())
  const original = globalThis.fetch
  globalThis.fetch = ((input: unknown) => handler(new URL(String(input), 'http://agena.test'))) as typeof fetch
  const chat = useChatStore()
  return {
    chat,
    close: () => {
      chat.clearTranscriptCache()
      globalThis.fetch = original
    },
  }
}

test('history and reply paging are independent, empty fold pages advance, refresh retains the expanded prefix', async () => {
  const requests: URL[] = []
  let finishHistory!: (response: Response) => void
  const history = new Promise<Response>((resolve) => {
    finishHistory = resolve
  })
  const { chat, close } = harness((url) => {
    requests.push(url)
    if (url.pathname.endsWith('/folds')) {
      return response(
        url.searchParams.get('cursor') === 'before-14' ? page([], 'before-12') : page([9, 10, 11, 12, 13], 'before-9'),
      )
    }
    if (url.searchParams.has('cursor')) return history
    return response(page([3, 14, 15, 16, 17, 18], 'history', [fold()]))
  })
  try {
    await chat.refreshMessages('7')
    const initial = chat.getMessagesForSession('7')[0]!.folds![0]!
    const older = chat.loadOlderMessages('7')
    assert.equal(chat.getSessionHistory('7').loading, true)
    assert.equal(await chat.loadFoldedActivity('7', initial), true)
    assert.equal(chat.getSessionHistory('7').loading, true)
    assert.equal(chat.foldLoadingByKey[transcriptFoldKey('7', initial)], false)
    finishHistory(response(page([], null)))
    await older
    await chat.refreshMessages('7')
    const current = chat.getMessagesForSession('7')[0]!
    assert.equal(current.folds![0]!.nextCursor, 'before-9')
    assert.equal(current.folds![0]!.hiddenCount, 5)
    assert.equal(current.parts.filter((part) => part.agenaKind !== 'run').length, 10)
    assert.equal(requests.find((url) => url.searchParams.get('cursor') === 'history')!.searchParams.get('limit'), '6')
  } finally {
    close()
  }
})

test('a failed or stalled reply request releases its lock and can be retried', async () => {
  let stalled = true
  const { chat, close } = harness((url) => {
    if (url.pathname.endsWith('/folds'))
      return response(stalled ? page([], 'before-14') : page([4, 5, 6, 7, 8, 9, 10, 11, 12, 13], null))
    return response(page([3, 14, 15, 16, 17, 18], 'history', [fold()]))
  })
  try {
    await chat.refreshMessages('7')
    const initial = chat.getMessagesForSession('7')[0]!.folds![0]!
    const key = transcriptFoldKey('7', initial)
    assert.equal(await chat.loadFoldedActivity('7', initial), false)
    assert.match(chat.foldErrorByKey[key]!, /did not advance/)
    assert.equal(chat.foldLoadingByKey[key], false)
    stalled = false
    assert.equal(await chat.loadFoldedActivity('7', initial, true), true)
    assert.equal(chat.foldErrorByKey[key], '')
    assert.equal(chat.getMessagesForSession('7')[0]!.folds?.length || 0, 0)
  } finally {
    close()
  }
})

test('an in-flight older fold response cannot overwrite a new streaming gap', async () => {
  let finishFold!: (response: Response) => void
  const pending = new Promise<Response>((resolve) => {
    finishFold = resolve
  })
  let streamed = false
  const { chat, close } = harness((url) => {
    if (url.pathname.endsWith('/folds')) return pending
    return response(
      streamed
        ? page([3, 24, 25, 26, 27, 28], 'history', [fold(24, 20)])
        : page([3, 14, 15, 16, 17, 18], 'history', [fold()]),
    )
  })
  try {
    await chat.refreshMessages('7')
    const initial = chat.getMessagesForSession('7')[0]!.folds![0]!
    const load = chat.loadFoldedActivity('7', initial)
    streamed = true
    await chat.refreshMessages('7')
    finishFold(response(page([9, 10, 11, 12, 13], 'before-9')))
    await load
    const current = chat.getMessagesForSession('7')[0]!.folds![0]!
    assert.equal(current.nextCursor, 'before-24')
    assert.equal(current.hiddenCount, 10)
  } finally {
    close()
  }
})

test('load all detects a multi-page cursor cycle and leaves the fold retryable', async () => {
  let count = 0
  const { chat, close } = harness((url) => {
    if (url.pathname.endsWith('/folds')) {
      count++
      return response(page([], count === 1 ? 'before-12' : 'before-14'))
    }
    return response(page([3, 14, 15], null, [fold()]))
  })
  try {
    await chat.refreshMessages('7')
    const current = chat.getMessagesForSession('7')[0]!.folds![0]!
    assert.equal(await chat.loadFoldedActivity('7', current, true), false)
    assert.equal(count, 2)
    assert.match(chat.foldErrorByKey[transcriptFoldKey('7', current)]!, /did not advance/)
    assert.equal(chat.foldLoadingByKey[transcriptFoldKey('7', current)], false)
  } finally {
    chat.$dispose()
    close()
  }
})
