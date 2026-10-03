import assert from 'node:assert/strict'
import test, { after, afterEach, beforeEach } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { computed, createSSRApp, reactive, ref } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createI18n } from 'vue-i18n'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { forkSession, listSessionMessages, rewindSession } from '../src/stores/chat/api'
import { copyTextToClipboard } from '../src/lib/clipboard'
import type { useChatMessageActions as MessageActionsFactory } from '../src/pages/chat/useChatMessageActions'
import type { useChatSessionActions as SessionActionsFactory } from '../src/pages/chat/useChatSessionActions'
import type { MessageEntry, Session } from '../src/types/chat'
import type { ComposerExpose, ComposerSegment } from '../src/pages/chat/composerInput'
import type { AttachedFile } from '../src/pages/chat/useChatAttachments'

// Use the application's module transforms for aliases and its locale glob.
// Middleware mode opens no HTTP listener and needs no browser DOM.
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
after(() => vite.close())
const { useChatMessageActions } = (await vite.ssrLoadModule('/src/pages/chat/useChatMessageActions.ts')) as {
  useChatMessageActions: typeof MessageActionsFactory
}
const { useChatSessionActions } = (await vite.ssrLoadModule('/src/pages/chat/useChatSessionActions.ts')) as {
  useChatSessionActions: typeof SessionActionsFactory
}

let originalWindow: unknown
beforeEach(() => {
  originalWindow = (globalThis as Record<string, unknown>).window
  ;(globalThis as Record<string, unknown>).window = {
    performance: globalThis.performance,
    location: { search: '', origin: 'http://localhost' },
  }
})
afterEach(() => {
  if (originalWindow === undefined) delete (globalThis as Record<string, unknown>).window
  else (globalThis as Record<string, unknown>).window = originalWindow
})

function response(value: unknown) {
  return new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json' } })
}

async function withFetch<T>(fetcher: typeof fetch, run: () => Promise<T>): Promise<T> {
  const original = globalThis.fetch
  globalThis.fetch = fetcher
  try {
    return await run()
  } finally {
    globalThis.fetch = original
  }
}

async function inSetup<T>(create: () => T): Promise<T> {
  let result!: T
  const app = createSSRApp({
    setup: () => {
      result = create()
      return () => null
    },
  })
  app.use(createI18n({ legacy: false, locale: 'en', missingWarn: false, fallbackWarn: false, messages: { en: {} } }))
  await renderToString(app)
  return result
}

const child: Session = { id: '2', title: 'Branch', state: { kind: 'ready' } }
const target: MessageEntry = {
  info: { id: '10', sessionID: '1', role: 'user', runState: 'completed' },
  parts: [
    { id: '11', sessionID: '1', messageID: '10', type: 'text', text: 'before ' },
    {
      id: '12',
      sessionID: '1',
      messageID: '10',
      type: 'file',
      filename: 'input.png',
      mime: 'image/png',
      serverPath: '.agena/uploads/input.png',
      url: 'data:image/png;base64,aGVsbG8=',
      agenaContent: { delivery: 'model_input' },
    },
    { id: '13', sessionID: '1', messageID: '10', type: 'text', text: ' after\nsecond line' },
  ],
}

async function messageHarness(query = {}) {
  const chat = reactive({
    selectedSessionId: '1',
    async selectSession(id: string) {
      this.selectedSessionId = id
    },
    async forkSession(_sessionId: string, _opts?: { at_message_id?: number }) {
      return child
    },
    async revertToMessage(_sessionId: string, _messageId: string) {
      return { session: child, message: target }
    },
  })
  const drafts = reactive<Record<string, string>>({ '1': 'unfinished original draft' })
  const draft = computed({
    get: () => drafts[chat.selectedSessionId] || '',
    set: (text) => {
      drafts[chat.selectedSessionId] = text
    },
  })
  const files = ref<AttachedFile[]>([])
  const notices: Array<[string, string]> = []
  const navigations: unknown[] = []
  let segments: ComposerSegment[] = []
  const actions = await inSetup(() =>
    useChatMessageActions({
      chat,
      toasts: {
        push: (kind, message) => {
          notices.push([kind, message])
        },
      },
      route: { query } as RouteLocationNormalizedLoaded,
      router: {
        replace: async (to: unknown) => {
          navigations.push(to)
        },
      } as Router,
      sessionDirectory: ref('/workspace'),
      draft,
      attachedFiles: files,
      clearAttachments: () => {
        files.value = []
      },
      composerRef: ref({
        restoreSegments: (value: ComposerSegment[]) => {
          segments = value
        },
      } as ComposerExpose),
      getTextParts: (parts) => parts.filter((part) => part.type === 'text'),
      copyToClipboard: async () => {},
      scrollToBottom: () => {},
    }),
  )
  return { chat, drafts, draft, files, notices, navigations, actions, segments: () => segments }
}

test('fork and rewind parse the nested session in an execution response', async () => {
  const requests: Array<{ path: string; body: unknown }> = []
  await withFetch(
    (async (url, init) => {
      requests.push({ path: String(url), body: JSON.parse(String(init?.body)) })
      return response({ session: { id: 2, title: 'Branch', state: { kind: 'ready' }, parent_id: 1 }, parts: [] })
    }) as typeof fetch,
    async () => {
      assert.equal((await forkSession('1', { at_message_id: 10 })).id, '2')
      assert.equal((await rewindSession('1', 10)).id, '2')
      assert.deepEqual(
        requests.map((request) => request.body),
        [{ at_message_id: 10 }, { at_message_id: 10 }],
      )
      assert.ok(requests[0]!.path.endsWith('/sessions/1/fork'))
      assert.ok(requests[1]!.path.endsWith('/sessions/1/rewind'))
      await assert.rejects(rewindSession('1', 0), /valid user message id/)
      assert.equal(requests.length, 2)
    },
  )
})

const marker = (id: number, role = 'user') => ({
  part_id: id,
  kind: 'run',
  role,
  state: 'completed',
  content: {},
  created_at_ms: id,
})
const text = (id: number, run: number, body: string) => ({
  part_id: id,
  run_id: run,
  kind: 'text',
  role: 'user',
  state: 'completed',
  content: { text: body },
  created_at_ms: id,
})

test('complete history joins runs split across pages and keeps chronological order', async () => {
  const requests: string[] = []
  await withFetch(
    (async (url) => {
      const path = String(url)
      requests.push(path)
      return response(
        path.includes('cursor=older')
          ? {
              session_id: 1,
              version: 1,
              parts: [marker(1), text(2, 1, 'oldest input'), marker(3)],
              page: { has_more: false },
            }
          : {
              session_id: 1,
              version: 1,
              parts: [text(4, 3, 'latest input')],
              page: { has_more: true, next_cursor: 'older' },
            },
      )
    }) as typeof fetch,
    async () => {
      const messages = await listSessionMessages('1')
      assert.deepEqual(
        messages.map((message) => [message.info.id, message.parts.map((part) => part.text)]),
        [
          ['1', ['oldest input']],
          ['3', ['latest input']],
        ],
      )
      assert.equal(requests.length, 2)
      assert.ok(requests.every((path) => path.includes('/sessions/1/parts?')))
    },
  )
})

test('history rejects a missing or repeated cursor instead of copying incomplete content', async () => {
  for (const next_cursor of [undefined, 'same']) {
    await withFetch(
      (async () => response({ parts: [], page: { has_more: true, next_cursor } })) as typeof fetch,
      async () => {
        await assert.rejects(listSessionMessages('1'), /pagination did not advance/)
      },
    )
  }
})

test('rewind opens the returned branch and restores text, media and inline order', async () => {
  const harness = await messageHarness({ windowId: 'win-1', agenaEmbed: '1', sessionId: '1' })
  await harness.actions.handleRevertFromMessage('10')
  assert.equal(harness.chat.selectedSessionId, '2')
  assert.equal(harness.drafts['1'], 'unfinished original draft')
  assert.equal(harness.draft.value, 'before  after\nsecond line')
  assert.equal(harness.files.value[0]?.serverPath, '.agena/uploads/input.png')
  assert.equal(harness.files.value[0]?.url, 'data:image/png;base64,aGVsbG8=')
  assert.equal(harness.files.value[0]?.delivery, 'model_input')
  assert.deepEqual(
    harness.segments().map((segment) => segment.type),
    ['text', 'attachment', 'text'],
  )
  assert.deepEqual(harness.navigations[0], {
    path: '/chat',
    query: { windowId: 'win-1', agenaEmbed: '1', sessionId: '2' },
  })
  assert.equal(harness.actions.revertBusyMessageId.value, '')
})

test('rewind errors preserve the draft, clear busy state and allow a retry', async () => {
  const harness = await messageHarness()
  let calls = 0
  harness.chat.revertToMessage = async () => {
    if (++calls === 1) throw new Error('The server rejected rewind')
    return { session: child, message: target }
  }
  await harness.actions.handleRevertFromMessage('10')
  assert.equal(harness.chat.selectedSessionId, '1')
  assert.equal(harness.draft.value, 'unfinished original draft')
  assert.equal(harness.actions.revertBusyMessageId.value, '')
  assert.deepEqual(harness.notices, [['error', 'The server rejected rewind']])
  await harness.actions.handleRevertFromMessage('10')
  assert.equal(calls, 2)
  assert.equal(harness.chat.selectedSessionId, '2')
})

test('message fork validates its cutoff and reports backend errors', async () => {
  const harness = await messageHarness()
  let calls = 0
  harness.chat.forkSession = async () => {
    calls++
    throw new Error('fork failed')
  }
  await harness.actions.handleForkFromMessage('not-an-id')
  assert.equal(calls, 0)
  await harness.actions.handleForkFromMessage('10')
  assert.equal(calls, 1)
  assert.deepEqual(harness.notices[1], ['error', 'fork failed'])
  assert.equal(harness.chat.selectedSessionId, '1')
})

test('copy transcript reads the complete server history and reports read failures', async () => {
  let copied = ''
  const notices: Array<[string, string]> = []
  const actions = await inSetup(() =>
    useChatSessionActions({
      chat: {
        selectedSessionId: '1',
        selectedSession: { id: '1' },
        renameSession: async () => null,
        forkSession: async () => child,
        compactSession: async () => null,
      },
      toasts: {
        push: (kind, message) => {
          notices.push([kind, message])
        },
      },
      sessionTitle: computed(() => 'Full history'),
      showThinking: ref(false),
      copyToClipboard: async (value) => {
        copied = await value
      },
    }),
  )
  await withFetch(
    (async (url) =>
      response(
        String(url).includes('cursor=older')
          ? { parts: [marker(1), text(2, 1, 'oldest input')], page: { has_more: false } }
          : { parts: [marker(3), text(4, 3, 'latest input')], page: { has_more: true, next_cursor: 'older' } },
      )) as typeof fetch,
    () => actions.copyTranscript(),
  )
  assert.ok(copied.includes('oldest input'))
  assert.ok(copied.includes('latest input'))
  assert.ok(copied.indexOf('oldest input') < copied.indexOf('latest input'))
  await withFetch(
    (async () => {
      throw new Error('history unavailable')
    }) as typeof fetch,
    () => actions.copyTranscript(),
  )
  assert.deepEqual(notices.at(-1), ['error', 'history unavailable'])
})

test('session fork waits for branch selection and coalesces repeated clicks', async () => {
  let finish!: () => void
  const navigation = new Promise<void>((resolve) => {
    finish = resolve
  })
  let calls = 0
  const notices: string[] = []
  const actions = await inSetup(() =>
    useChatSessionActions({
      chat: {
        selectedSessionId: '1',
        selectedSession: { id: '1' },
        renameSession: async () => null,
        forkSession: async () => {
          calls++
          return child
        },
        compactSession: async () => null,
      },
      toasts: {
        push: (_kind, message) => {
          notices.push(message)
        },
      },
      sessionTitle: computed(() => 'Session'),
      showThinking: ref(false),
      copyToClipboard: async () => {},
      onSessionForked: () => navigation,
    }),
  )
  const first = actions.handleForkSession()
  await actions.handleForkSession()
  assert.equal(calls, 1)
  assert.equal(actions.forkBusy.value, true, JSON.stringify(notices))
  finish()
  await first
  assert.equal(actions.forkBusy.value, false)
})

test('complete-history clipboard writing starts before its asynchronous read resolves', async () => {
  const scope = globalThis as Record<string, any>
  const originalNavigator = scope.navigator
  const originalClipboardItem = scope.ClipboardItem
  let finish!: (text: string) => void
  const pending = new Promise<string>((resolve) => {
    finish = resolve
  })
  let started = false
  let copied = ''
  scope.window.isSecureContext = true
  scope.ClipboardItem = class {
    constructor(public data: Record<string, Promise<Blob>>) {}
  }
  scope.navigator = {
    clipboard: {
      write: async (items: Array<{ data: Record<string, Promise<Blob>> }>) => {
        started = true
        copied = await (await items[0]!.data['text/plain']!).text()
      },
    },
  }
  try {
    const result = copyTextToClipboard(pending)
    assert.equal(started, true)
    finish('all of the session history')
    assert.equal(await result, true)
    assert.equal(copied, 'all of the session history')
  } finally {
    if (originalNavigator === undefined) delete scope.navigator
    else scope.navigator = originalNavigator
    if (originalClipboardItem === undefined) delete scope.ClipboardItem
    else scope.ClipboardItem = originalClipboardItem
  }
})
