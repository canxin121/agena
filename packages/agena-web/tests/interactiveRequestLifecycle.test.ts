import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { createRenderer, h, nextTick, reactive, ssrContextKey } from 'vue'
import { createI18n } from 'vue-i18n'
import { ensureBrowserTestRuntime } from './testRuntime'

ensureBrowserTestRuntime()
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  plugins: [
    {
      name: 'interaction-test-memory-history',
      transform(code, id) {
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
      },
    },
  ],
})
after(() => vite.close())
const { useChatStore } = (await vite.ssrLoadModule('/src/stores/chat.ts')) as typeof import('../src/stores/chat')
const { operationPresentation } = await vite.ssrLoadModule('/src/pages/chat/transcriptPartPresentation.ts')
const { projectLocalPart, partHasPendingInteraction } = await vite.ssrLoadModule(
  '/src/pages/chat/transcriptProjection.ts',
)
const { default: interactionComponent } = await vite.ssrLoadModule('/src/components/chat/AgenaInteractionPart.vue')
const requestId = 'host-input:7:2:0'
const question = { header: 'Plan', question: 'Approve?', options: [{ label: 'Approve' }] }
const request = { request_id: requestId, session_id: 7, kind: 'review', questions: [question] }
const marker = { part_id: 1, kind: 'run', role: 'assistant', state: 'in_progress', content: {}, revision: 1 }
const tool = (resolved = false) => ({
  part_id: 2,
  run_id: 1,
  origin_session_id: 7,
  kind: 'tool_call',
  role: 'assistant',
  state: resolved ? 'cancelled' : 'in_progress',
  revision: resolved ? 2 : 1,
  content: { name: 'plan.set', user_input: { requests: [{ request, reply: null }] } },
})
const state = (pending = false, version = 3, sessionId = 7) => ({
  session: {
    id: sessionId,
    version,
    title: 'Plan',
    state: pending
      ? { kind: 'awaiting_interaction', data: { requests: [{ ...request, kind: 'user_input', input_kind: 'review' }] } }
      : { kind: 'ready', data: {} },
  },
  execution: {},
  parts: [],
})
const deferred = <T>() => {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

async function withChat(run: (chat: ReturnType<typeof useChatStore>) => Promise<void>) {
  const pinia = createPinia()
  setActivePinia(pinia)
  const original = globalThis.fetch
  const chat = useChatStore()
  chat.retainSession('7')
  try {
    await run(chat)
  } finally {
    disposePinia(pinia)
    globalThis.fetch = original
  }
}

function installPending(chat: ReturnType<typeof useChatStore>) {
  for (const part of [marker, tool()])
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: 7, part } })
}

test('status recovery refreshes an old pending owner by id even outside the recent transcript window', async () =>
  withChat(async (chat) => {
    const paths: string[] = []
    globalThis.fetch = (async (url) => {
      const path = String(url)
      paths.push(path)
      if (path.includes('/parts?')) return Response.json({ parts: [tool(true)] })
      return Response.json(state())
    }) as typeof fetch
    installPending(chat)
    await chat.refreshExecutionStatus('7')
    assert.equal(chat.getMessagesForSession('7')[0]?.parts[0]?.partState, 'cancelled')
    assert.ok(
      paths.some((path) => path.includes('ids=2')),
      'only the loaded stale owner needs revalidation',
    )
  }))

test('an already resolved request is reconciled without posting another answer or rejection', async () =>
  withChat(async (chat) => {
    let posts = 0
    globalThis.fetch = (async (url, options) => {
      if (options?.method === 'POST') posts++
      if (String(url).includes('/parts?')) return Response.json({ parts: [tool(true)] })
      return Response.json(state())
    }) as typeof fetch
    installPending(chat)
    assert.equal(await chat.replyQuestion('7', requestId, [['Approve']]), false)
    assert.equal(await chat.rejectQuestion('7', requestId), false)
    assert.equal(posts, 0)
    assert.equal(chat.getMessagesForSession('7')[0]?.parts[0]?.partState, 'cancelled')
  }))

for (const source of ['older history', 'folded activity'] as const) {
  test(`a page of ${source} arriving after resolution reconciles its stale interaction owner`, async () =>
    withChat(async (chat) => {
      const olderPage = deferred<Response>()
      const refreshedIds: string[] = []
      globalThis.fetch = (async (url) => {
        const path = String(url)
        if (path.includes('/runs?') || path.includes('run_ids=')) return olderPage.promise
        if (path.includes('/parts?')) {
          refreshedIds.push(new URL(path, 'http://agena.test').searchParams.get('ids') || '')
          return Response.json({ parts: [tool(true)] })
        }
        return Response.json(state())
      }) as typeof fetch
      const loading =
        source === 'older history'
          ? chat.loadOlderMessages('7')
          : chat.loadFoldedActivity('7', {
              runId: 1,
              runIds: [1],
              anchorPartId: '3',
              hiddenCount: 1,
              nextCursor: 'older',
            })
      await chat.refreshExecutionStatus('7')
      olderPage.resolve(
        Response.json({ parts: [marker, tool()], runs: [], user_message_count: 1, page: { has_more: false } }),
      )
      assert.equal(await loading, true)
      for (let i = 0; i < 20; i++) await nextTick()
      assert.deepEqual(refreshedIds, ['2'])
      assert.equal(chat.getMessagesForSession('7')[0]?.parts[0]?.partState, 'cancelled')
    }))
}

test('a request removed by rewind retires its loaded membership and cannot return through an old page', async () =>
  withChat(async (chat) => {
    globalThis.fetch = (async (url) =>
      String(url).includes('/parts?') ? Response.json({ parts: [] }) : Response.json(state())) as typeof fetch
    installPending(chat)
    await chat.refreshExecutionStatus('7')
    assert.equal(chat.getMessagesForSession('7')[0]?.parts.length, 0)
    installPending(chat)
    assert.equal(chat.getMessagesForSession('7')[0]?.parts.length, 0)
  }))

test('a request resolved between preflight and commit refreshes its facts instead of surfacing an internal error', async () =>
  withChat(async (chat) => {
    let replied = false
    let posts = 0
    globalThis.fetch = (async (url) => {
      const path = String(url)
      if (path.includes('/user-input-replies')) {
        posts++
        replied = true
        return Response.json(
          {
            problem: {
              code: 'interaction.request_not_pending',
              category: 'conflict',
              user: { fallback: 'This request is no longer pending.' },
            },
          },
          { status: 409 },
        )
      }
      if (path.includes('/parts?')) return Response.json({ parts: [tool(true)] })
      return Response.json(state(!replied, replied ? 4 : 3))
    }) as typeof fetch
    installPending(chat)
    assert.equal(await chat.replyQuestion('7', requestId, [['Approve']]), false)
    assert.equal(posts, 1)
    assert.equal(chat.getPendingInteraction('7', requestId, 'user_input'), null)
    assert.equal(chat.getMessagesForSession('7')[0]?.parts[0]?.partState, 'cancelled')
  }))

test('a failed preflight never sends a reply, and a real reply failure remains visible', async () =>
  withChat(async (chat) => {
    let posts = 0
    globalThis.fetch = (async (_url, options) => {
      if (options?.method === 'POST') posts++
      throw new Error('offline')
    }) as typeof fetch
    installPending(chat)
    await assert.rejects(chat.replyQuestion('7', requestId, [['Approve']]), /offline/)
    assert.equal(posts, 0)
    globalThis.fetch = (async (url) =>
      String(url).includes('/user-input-replies')
        ? Response.json({ error: 'Reply validation failed' }, { status: 422 })
        : Response.json(state(true))) as typeof fetch
    await assert.rejects(chat.replyQuestion('7', requestId, [['Approve']]), /Reply validation failed/)
  }))

test('a committed answer updates its old owner even when the response only contains a state shell', async () =>
  withChat(async (chat) => {
    let replied = false
    let body: any
    const answered = {
      ...tool(),
      revision: 2,
      content: {
        name: 'plan.set',
        user_input: {
          requests: [
            {
              request,
              reply: { request_id: requestId, kind: 'submit', answers: { '0': ['Approve'] } },
            },
          ],
        },
      },
    }
    globalThis.fetch = (async (url, options) => {
      const path = String(url)
      if (path.includes('/user-input-replies')) {
        body = JSON.parse(String(options?.body))
        replied = true
      }
      if (path.includes('/parts?')) return Response.json({ parts: [answered] })
      return Response.json(state(!replied, replied ? 4 : 3))
    }) as typeof fetch
    installPending(chat)
    assert.equal(await chat.replyQuestion('7', requestId, [['Approve']]), true)
    assert.deepEqual(body.reply.answers, { '0': ['Approve'] })
    assert.equal(chat.getPendingInteraction('7', requestId, 'user_input'), null)
    const content = chat.getMessagesForSession('7')[0]?.parts[0]?.agenaContent as any
    assert.equal(content.user_input.requests[0].reply.kind, 'submit')
  }))

test('a history refresh failure after commit does not turn a successful approval into an error', async () =>
  withChat(async (chat) => {
    let replied = false
    globalThis.fetch = (async (url) => {
      const path = String(url)
      if (path.includes('/user-input-replies')) replied = true
      if (path.includes('/parts?')) throw new Error('history temporarily offline')
      return Response.json(state(!replied, replied ? 4 : 3))
    }) as typeof fetch
    installPending(chat)
    assert.equal(await chat.replyQuestion('7', requestId, [['Approve']]), true)
    assert.equal(chat.getPendingInteraction('7', requestId, 'user_input'), null)
  }))

test('a failed owner refresh does not undo canonical retirement or send a stale reply', async () =>
  withChat(async (chat) => {
    let posts = 0
    globalThis.fetch = (async (url, options) => {
      if (options?.method === 'POST') posts++
      if (String(url).includes('/parts?')) throw new Error('history temporarily offline')
      return Response.json(state())
    }) as typeof fetch
    installPending(chat)
    assert.equal(await chat.replyQuestion('7', requestId, [['Approve']]), false)
    assert.equal(chat.getPendingInteraction('7', requestId, 'user_input'), null)
    assert.equal(posts, 0)
  }))

test('permission replies use the same canonical preflight and stop for retired requests', async () =>
  withChat(async (chat) => {
    let posts = 0
    globalThis.fetch = (async (_url, options) => {
      if (options?.method === 'POST') posts++
      return Response.json(state())
    }) as typeof fetch
    assert.equal(await chat.replyPermission('7', 'permission-1', 'once'), false)
    assert.equal(posts, 0)
  }))

test('a late presentation acknowledgement cannot resurrect a resolved request', async () =>
  withChat(async (chat) => {
    const acknowledgement = deferred<Response>()
    let pending = true
    globalThis.fetch = (async (url) =>
      String(url).includes('/present')
        ? acknowledgement.promise
        : Response.json(state(pending, pending ? 3 : 4))) as typeof fetch
    await chat.refreshExecutionStatus('7')
    assert.ok(chat.getPendingInteraction('7', requestId, 'user_input'))
    pending = false
    await chat.refreshExecutionStatus('7')
    acknowledgement.resolve(Response.json(state(true, 3)))
    for (let i = 0; i < 5; i++) await nextTick()
    assert.equal(chat.getPendingInteraction('7', requestId, 'user_input'), null)
    assert.equal(chat.getSessionAttention('7'), null)
  }))

test('parent attention routes replies and presentation acknowledgements to the typed request owner', async () =>
  withChat(async (chat) => {
    const posts: string[] = []
    let replied = false
    globalThis.fetch = (async (url, options) => {
      const path = String(url)
      if (options?.method === 'POST') posts.push(path)
      if (path.includes('/user-input-replies')) replied = true
      return Response.json(state(!replied, replied ? 4 : 3, path.includes('/sessions/9/') ? 9 : 7))
    }) as typeof fetch
    assert.equal(await chat.replyQuestion('9', requestId, [['Approve']]), true)
    assert.ok(posts.some((path) => path.includes('/sessions/7/user-input-replies')))
    assert.ok(posts.some((path) => path.includes('/sessions/7/interactive/')))
    assert.ok(posts.every((path) => !path.includes('/sessions/9/')))
  }))

test('inherited interaction history is read-only and does not count as a pending interaction', async () =>
  withChat(async (chat) => {
    chat.retainSession('9')
    for (const part of [marker, tool()])
      chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: 9, part } })
    const inherited = chat.getMessagesForSession('9')[0]!.parts[0]!
    assert.equal(inherited.originSessionId, 7)
    assert.equal(partHasPendingInteraction(inherited), false)
    assert.equal(operationPresentation(projectLocalPart(inherited, 'assistant')).userInputs[0].pending, false)
  }))

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

test('the mounted interaction retires controls immediately when a state shell resolves it, preserving its unsent draft', async () =>
  withChat(async (chat) => {
    let pending = true
    globalThis.fetch = (async () => Response.json(state(pending, pending ? 3 : 4))) as typeof fetch
    await chat.refreshExecutionStatus('7')
    const props = reactive({
      sessionId: '7',
      interaction: {
        requestId,
        title: 'Plan',
        kind: 'review',
        pending: true,
        reply: null,
        bodyMarkdown: '',
        questions: [
          {
            questionId: '0',
            header: 'Plan',
            question: 'Approve?',
            multiple: false,
            allowCustom: true,
            options: [{ label: 'Approve', description: '' }],
          },
        ],
      },
    })
    let ui: any
    const subject = {
      ...interactionComponent,
      setup(props: any, context: any) {
        ui = interactionComponent.setup(props, context)
        return () => null
      },
    }
    const app = renderer.createApp({ render: () => h(subject, props) })
    app.provide(ssrContextKey, {})
    app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
    app.mount({})
    try {
      assert.equal(ui.interaction.value.pending, true)
      chat.getInteractionDraft('7', requestId).customDrafts = ['Unsent work']
      pending = false
      await chat.refreshExecutionStatus('7')
      await nextTick()
      assert.equal(ui.interaction.value.pending, false)
      assert.equal(chat.getInteractionDraft('7', requestId).customDrafts[0], 'Unsent work')
      assert.deepEqual(ui.customDrafts.value, [''], 'an unsent draft is never presented as a committed answer')
    } finally {
      app.unmount()
    }
  }))

test('returning after transcript cache eviction rehydrates pending requests from a reusable state resource', async () =>
  withChat(async (chat) => {
    const token = 'interactive-remount-test:20'
    globalThis.fetch = (async (url) => {
      const path = String(url)
      if (path.includes('/present')) return Response.json({ error: 'advisory acknowledgement failed' }, { status: 409 })
      if (path.includes('/changes/revisions?')) {
        const resources = JSON.parse(new URL(path, 'http://agena.test').searchParams.get('resources') || '[]')
        return Response.json(Object.fromEntries(resources.map((key: string) => [key, token])))
      }
      return Response.json(state(true, 20), { headers: { etag: `"${token}"` } })
    }) as typeof fetch
    await chat.refreshExecutionStatus('7')
    assert.ok(chat.getPendingInteraction('7', requestId, 'user_input'))
    chat.clearTranscriptCache()
    assert.equal(chat.hasInteractionSnapshot('7'), false)
    await chat.refreshExecutionStatus('7')
    assert.ok(chat.getPendingInteraction('7', requestId, 'user_input'))
  }))

test('reply preflight revalidates a reusable HTTP state cache even when no resolution event arrived', async () =>
  withChat(async (chat) => {
    const api = await vite.ssrLoadModule('/src/stores/chat/api.ts')
    let pending = true
    let reads = 0
    let posts = 0
    globalThis.fetch = (async (_url, options) => {
      if (options?.method === 'POST') posts++
      reads++
      return Response.json(state(pending, pending ? 10 : 11), {
        headers: { etag: `"interactive-lifecycle-test:${pending ? 10 : 11}"` },
      })
    }) as typeof fetch
    await api.getSessionExecutionStatus('7', true)
    pending = false
    assert.equal(await chat.replyQuestion('7', requestId, [['Approve']]), false)
    assert.equal(reads, 2, 'preflight performs a server validation instead of reusing an unverified cache')
    assert.equal(posts, 0)
  }))
