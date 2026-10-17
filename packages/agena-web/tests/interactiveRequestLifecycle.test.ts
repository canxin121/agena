import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { createRenderer, createSSRApp, h, nextTick, reactive, ssrContextKey } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createI18n } from 'vue-i18n'
import type { JsonValue } from '../src/types/json'
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
const { transcriptNativeControl } = await vite.ssrLoadModule('/src/pages/chat/transcriptControls.ts')
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

async function withChat(
  run: (chat: ReturnType<typeof useChatStore>, pinia: ReturnType<typeof createPinia>) => Promise<void>,
) {
  const pinia = createPinia()
  setActivePinia(pinia)
  const original = globalThis.fetch
  const chat = useChatStore()
  chat.retainSession('7')
  try {
    await run(chat, pinia)
  } finally {
    disposePinia(pinia)
    globalThis.fetch = original
  }
}

function installPending(chat: ReturnType<typeof useChatStore>) {
  for (const part of [marker, tool()])
    chat.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', session_id: 7, part } })
}

test('option labels and descriptions own pointer gestures while ordinary transcript text remains selectable', () => {
  const input = { type: 'checkbox' }
  const label = {
    querySelector(selector: string) {
      assert.equal(selector, 'input[type="radio"], input[type="checkbox"]')
      return input
    },
  }
  for (const content of ['label', 'description', 'nested icon']) {
    const target = { closest: (selector: string) => (selector === 'label' ? label : null), content }
    assert.equal(transcriptNativeControl(target as any), input)
  }
  assert.equal(transcriptNativeControl({ closest: () => null } as any), null)
  const button = { tagName: 'BUTTON' }
  assert.equal(transcriptNativeControl({ closest: () => button } as any), button)
})

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

function actionInteractionProps(kind: string, multiple = false) {
  return {
    sessionId: '7',
    interaction: {
      requestId,
      title: 'Decide',
      kind,
      pending: true,
      reply: null as JsonValue | null,
      bodyMarkdown: '',
      questions: [
        {
          questionId: '0',
          header: '',
          question: 'Choose',
          multiple,
          allowCustom: true,
          options: [
            { label: 'First', description: 'Long description' },
            { label: 'Second', description: '' },
          ],
        },
      ],
    },
  }
}

async function renderActionHistory(
  pinia: ReturnType<typeof createPinia>,
  kind: string,
  reply: JsonValue | null,
  multiple = false,
) {
  const props = actionInteractionProps(kind, multiple)
  props.interaction.pending = false
  props.interaction.reply = reply
  const app = createSSRApp(interactionComponent, props)
  const { default: messages } = await vite.ssrLoadModule('/src/i18n/messages/en-US.ts')
  app.use(pinia)
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': messages } }))
  return renderToString(app)
}

async function renderPermission(pinia: ReturnType<typeof createPinia>, pending: boolean, replyReason = '') {
  const app = createSSRApp(interactionComponent, {
    sessionId: '7',
    permission: {
      requestId,
      pending,
      status: pending ? 'Awaiting user approval' : 'Denied once',
      action: 'shell.exec · cargo test',
      reason: 'Validate the current change',
      explanation: 'Execute in the project workspace',
      provenance: 'tool · workspace',
      replyReason,
    },
  })
  const { default: messages } = await vite.ssrLoadModule('/src/i18n/messages/en-US.ts')
  app.use(pinia)
  app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': messages } }))
  return renderToString(app)
}

test('pending permissions expose exactly three explicit decisions and disable them together while replying', async () =>
  withChat(async (chat, pinia) => {
    let sends = 0
    chat.getPendingInteraction = () => ({ requestId }) as any
    chat.replyPermission = async () => {
      sends++
      return true
    }
    const html = await renderPermission(pinia, true)
    assert.deepEqual(
      [...html.matchAll(/data-interaction-control-key="([^"]+)"/g)].map((match) => match[1]),
      ['permission:once', 'permission:always', 'permission:reject'],
    )
    assert.match(html, /shell\.exec · cargo test/)
    assert.match(html, /Validate the current change/)
    assert.match(html, /Execute in the project workspace/)
    assert.doesNotMatch(html, /<button[^>]*\sdisabled(?:\s|[=>])|data-selected="true"/)
    chat.getInteractionDraft('7', requestId).busy = true
    const busyHtml = await renderPermission(pinia, true)
    assert.equal((busyHtml.match(/<button[^>]*\sdisabled(?:\s|[=>])/g) || []).length, 3)
    assert.equal(sends, 0, 'rendering and status updates never submit a decision')
  }))

test('permission history keeps the outcome and multiline reason without any old decision buttons', async () =>
  withChat(async (_chat, pinia) => {
    const html = await renderPermission(pinia, false, '请先修改命令\n保留当前工作区')
    assert.match(html, /Denied once/)
    assert.match(html, /请先修改命令\n保留当前工作区/)
    assert.doesNotMatch(html, /<button|data-interaction-control|Allow once|Always allow/)
  }))

function mountActionInteraction(chat: ReturnType<typeof useChatStore>, kind: string, multiple = false) {
  chat.getPendingInteraction = () => ({ requestId }) as any
  chat.hasInteractionSnapshot = () => true
  const props = reactive(actionInteractionProps(kind, multiple))
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
  app.use(
    createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} }, missingWarn: false, fallbackWarn: false }),
  )
  app.mount({})
  const option = (label: string, index = 0) =>
    ({
      getAttribute: (name: string) =>
        ({
          'data-interaction-question-index': String(index),
          'data-interaction-control-kind': 'option',
          'data-interaction-control-value': label,
        })[name] || null,
    }) as unknown as Element
  return { app, ui, props, option }
}

test('completed review renders only the committed decision, without obsolete controls or unsent drafts', async () =>
  withChat(async (chat, pinia) => {
    const draft = chat.getInteractionDraft('7', requestId)
    draft.selected = [['Second']]
    draft.customDrafts = ['Unsubmitted feedback']
    draft.customOpen = [true]
    const html = await renderActionHistory(pinia, 'review', { kind: 'submit', answers: { '0': ['First'] } })
    assert.match(html, /data-interaction-result/)
    assert.match(html, /First/)
    assert.match(html, /Long description/)
    assert.match(html, /Answered/)
    assert.doesNotMatch(html, /Second|Unsubmitted feedback/)
    assert.doesNotMatch(html, /<button|<textarea|data-interaction-control|data-interaction-options/)
  }))

test('completed custom feedback is readable text and preserves line breaks without an old editor', async () =>
  withChat(async (_chat, pinia) => {
    const html = await renderActionHistory(pinia, 'review', {
      kind: 'submit',
      answers: { '0': ['请修改第二步\n保留其他步骤'] },
    })
    assert.match(html, /请修改第二步\n保留其他步骤/)
    assert.doesNotMatch(html, /First|Second|<textarea|<button|data-interaction-control/)
  }))

test('completed multi-choice history shows every submitted answer and the full question', async () =>
  withChat(async (_chat, pinia) => {
    const html = await renderActionHistory(
      pinia,
      'ask_user',
      { answers: { '0': ['Second', 'Additional context'] } },
      true,
    )
    assert.match(html, /Choose/)
    assert.match(html, /Second/)
    assert.match(html, /Additional context/)
    assert.doesNotMatch(html, /First|<input|<button|<textarea|data-interaction-control/)
  }))

test('cancelled and unanswered history never presents a local draft as a submitted result', async () =>
  withChat(async (chat, pinia) => {
    const draft = chat.getInteractionDraft('7', requestId)
    draft.selected = [['First']]
    draft.customDrafts = ['Still a draft']
    draft.customOpen = [true]
    for (const [reply, status] of [
      [{ kind: 'cancel' }, 'Cancelled'],
      [null, 'Unanswered'],
    ] as const) {
      const html = await renderActionHistory(pinia, 'review', reply)
      assert.match(html, new RegExp(status))
      assert.doesNotMatch(html, /First|Second|Still a draft|<button|<input|<textarea|data-interaction-control/)
    }
    assert.deepEqual(JSON.parse(JSON.stringify(draft.customDrafts)), ['Still a draft'])
  }))

test('review cards submit the activated decision once and block repeated activation while busy', async () =>
  withChat(async (chat) => {
    globalThis.fetch = (async () => Response.json(state(true))) as typeof fetch
    await chat.refreshExecutionStatus('7')
    const reply = deferred<boolean>()
    const answers: string[][][] = []
    chat.replyQuestion = async (_session, _id, value) => {
      answers.push(value)
      return reply.promise
    }
    const { app, ui, option } = mountActionInteraction(chat, 'review')
    try {
      assert.deepEqual(JSON.parse(JSON.stringify(ui.selected.value)), [[]])
      const first = ui.confirmCurrentOption(option('Second'))
      await nextTick()
      assert.equal(ui.busy.value, true)
      await ui.confirmCurrentOption(option('First'))
      assert.deepEqual(JSON.parse(JSON.stringify(answers)), [[['Second']]])
      assert.deepEqual(JSON.parse(JSON.stringify(ui.selected.value)), [['Second']])
      reply.resolve(true)
      await first
      assert.equal(ui.busy.value, false)
    } finally {
      app.unmount()
    }
  }))

async function pressInteractionKey(
  ui: any,
  key: string,
  kind: string,
  label = '',
  index = 0,
  overrides: Record<string, unknown> = {},
) {
  const original = globalThis.HTMLElement
  class Control {
    closest(selector: string) {
      if (selector.includes('data-interaction-control')) return this
      return kind === 'textarea' && selector.startsWith('textarea') ? this : null
    }
    getAttribute(name: string) {
      return (
        {
          'data-interaction-control-kind': kind,
          'data-interaction-question-index': String(index),
          'data-interaction-control-value': label,
        }[name] || null
      )
    }
  }
  Object.assign(globalThis, { HTMLElement: Control })
  const event = {
    key,
    target: new Control(),
    defaultPrevented: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    preventDefault() {
      this.defaultPrevented = true
    },
    stopPropagation() {},
    ...overrides,
  }
  try {
    await ui.handleKeydown(event)
    return event
  } finally {
    globalThis.HTMLElement = original
  }
}

test('Space chooses and Enter submits the selected multi-choice answers without toggling the focused option', async () =>
  withChat(async (chat) => {
    const answers: string[][][] = []
    chat.replyQuestion = async (_session, _id, value) => {
      answers.push(value)
      return true
    }
    const { app, ui } = mountActionInteraction(chat, 'ask_user', true)
    try {
      await pressInteractionKey(ui, 'Enter', 'option', 'First')
      assert.deepEqual(answers, [], 'Enter on an unanswered question must not select or submit')
      assert.deepEqual(JSON.parse(JSON.stringify(ui.selected.value)), [[]])
      for (const label of ['First', 'Second', 'Second']) await pressInteractionKey(ui, ' ', 'option', label)
      assert.deepEqual(JSON.parse(JSON.stringify(ui.selected.value)), [['First']])
      await pressInteractionKey(ui, 'Enter', 'option', 'Second')
      assert.deepEqual(JSON.parse(JSON.stringify(answers)), [[['First']]])
      assert.equal(ui.questionPage.value, 0, 'the final question submits without requiring the summary')
    } finally {
      app.unmount()
    }
  }))

test('horizontal keys switch questions, preserve answers and stop at the first and last question', async () =>
  withChat(async (chat) => {
    const { app, ui, props } = mountActionInteraction(chat, 'ask_user')
    props.interaction.questions.push({ ...props.interaction.questions[0], questionId: '1', multiple: true })
    await nextTick()
    try {
      await pressInteractionKey(ui, ' ', 'option', 'Second')
      for (const key of ['h', 'ArrowLeft']) {
        await pressInteractionKey(ui, key, 'option', 'Second')
        assert.equal(ui.questionPage.value, 0)
      }
      for (const key of ['l', 'ArrowRight']) {
        await pressInteractionKey(ui, key, 'option', 'Second')
        assert.equal(ui.questionPage.value, 1)
        await pressInteractionKey(ui, 'ArrowLeft', 'option', 'First', 1)
        assert.equal(ui.questionPage.value, 0)
      }
      await pressInteractionKey(ui, 'Enter', 'option', 'First')
      assert.equal(ui.questionPage.value, 1, 'Enter confirms Second without selecting focused First')
      await pressInteractionKey(ui, 'ArrowRight', 'option', 'First', 1)
      assert.equal(ui.questionPage.value, 1, 'Right on the final question neither submits nor opens summary')
      assert.deepEqual(JSON.parse(JSON.stringify(ui.selected.value)), [['Second'], []])
      await pressInteractionKey(ui, 'Enter', 'option', 'First', 1)
      assert.equal(ui.questionPage.value, 1, 'an unanswered question stays in place')
    } finally {
      app.unmount()
    }
  }))

test('final Enter validates earlier questions before sending and retains all selected answers', async () =>
  withChat(async (chat) => {
    const answers: string[][][] = []
    chat.replyQuestion = async (_session, _id, value) => {
      answers.push(value)
      return true
    }
    const { app, ui, props } = mountActionInteraction(chat, 'ask_user')
    props.interaction.questions.push({ ...props.interaction.questions[0], questionId: '1', multiple: true })
    await nextTick()
    try {
      ui.moveQuestionPage(1)
      ui.toggleOption(1, 'First')
      await ui.confirmQuestion(1)
      assert.equal(ui.questionPage.value, 0, 'validation returns to the first unanswered question')
      assert.equal(answers.length, 0)
      ui.toggleOption(0, 'Second')
      await ui.confirmQuestion(0)
      await ui.confirmQuestion(1)
      assert.deepEqual(JSON.parse(JSON.stringify(answers)), [[['Second'], ['First']]])
    } finally {
      app.unmount()
    }
  }))

test('final-question submission blocks duplicate Enter and retains answers for a failed-send retry', async () =>
  withChat(async (chat) => {
    const reply = deferred<boolean>()
    let sends = 0
    chat.replyQuestion = async () => {
      sends++
      return reply.promise
    }
    const { app, ui, option } = mountActionInteraction(chat, 'ask_user')
    try {
      ui.toggleOption(0, 'Second')
      const first = ui.confirmCurrentOption(option('First'))
      await nextTick()
      await ui.confirmCurrentOption(option('First'))
      assert.equal(sends, 1)
      reply.resolve(false)
      await first
      assert.equal(ui.busy.value, false)
      assert.deepEqual(JSON.parse(JSON.stringify(ui.selected.value)), [['Second']])
      await ui.confirmCurrentOption(option('First'))
      assert.equal(sends, 2)
    } finally {
      app.unmount()
    }
  }))

test('custom editor keeps arrows, Space, Shift+Enter and IME native; plain Enter submits the final answer', async () =>
  withChat(async (chat) => {
    const answers: string[][][] = []
    chat.replyQuestion = async (_session, _id, value) => {
      answers.push(value)
      return true
    }
    const { app, ui } = mountActionInteraction(chat, 'ask_user')
    try {
      ui.openCustomEditor(0)
      ui.customDrafts.value[0] = '中文补充\n第二行'
      for (const key of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'h', 'j', 'k', 'l', ' ']) {
        assert.equal((await pressInteractionKey(ui, key, 'textarea')).defaultPrevented, false)
      }
      for (const overrides of [{ shiftKey: true }, { isComposing: true }, { keyCode: 229 }]) {
        assert.equal((await pressInteractionKey(ui, 'Enter', 'textarea', '', 0, overrides)).defaultPrevented, false)
      }
      assert.equal(answers.length, 0)
      await pressInteractionKey(ui, 'Enter', 'textarea')
      assert.deepEqual(JSON.parse(JSON.stringify(answers)), [[['中文补充\n第二行']]])
    } finally {
      app.unmount()
    }
  }))

for (const multiple of [false, true]) {
  test(`a custom-only ${multiple ? 'multi' : 'single'} choice uses native option selection and sends its text`, async () =>
    withChat(async (chat) => {
      const answers: string[][][] = []
      chat.replyQuestion = async (_session, _id, value) => {
        answers.push(value)
        return true
      }
      const { app, ui, props } = mountActionInteraction(chat, 'ask_user', multiple)
      Object.assign(props.interaction.questions[0], {
        allowCustom: false,
        options: [{ label: 'Write an answer', description: 'Use your own words', custom: true }],
      })
      await nextTick()
      try {
        assert.equal(ui.customAllowed(0), true)
        assert.equal(ui.questionOptions(0).length, 1)
        await pressInteractionKey(ui, 'Enter', 'custom')
        assert.deepEqual(JSON.parse(JSON.stringify(ui.selected.value)), [[]])
        assert.equal(answers.length, 0)
        await pressInteractionKey(ui, ' ', 'custom')
        assert.equal(ui.isCustomSelected(0), true)
        assert.equal(ui.customOpen.value[0], true)
        assert.equal(ui.canSubmit.value, false, 'selected custom options require text')
        ui.customDrafts.value[0] = '中文, answer\n第二行'
        await pressInteractionKey(ui, 'Escape', 'textarea')
        assert.equal(ui.isCustomSelected(0), true, 'closing the editor keeps the chosen option')
        assert.equal(ui.canSubmit.value, true)
        await pressInteractionKey(ui, 'Enter', 'custom')
        assert.deepEqual(JSON.parse(JSON.stringify(answers)), [[['中文, answer\n第二行']]])
      } finally {
        app.unmount()
      }
    }))
}

test('custom single choice is exclusive and switching choices preserves the editable draft', async () =>
  withChat(async (chat) => {
    const { app, ui, props } = mountActionInteraction(chat, 'ask_user')
    Object.assign(props.interaction.questions[0], {
      allowCustom: false,
      options: [
        { label: 'Fixed', description: '' },
        { label: 'Other', description: '', custom: true },
      ],
    })
    await nextTick()
    try {
      ui.toggleOption(0, 'Other')
      ui.customDrafts.value[0] = 'My answer'
      ui.toggleOption(0, 'Fixed')
      assert.equal(ui.isCustomSelected(0), false)
      assert.deepEqual(ui.answerFor(0), ['Fixed'])
      assert.equal(ui.customDrafts.value[0], 'My answer')
      ui.toggleOption(0, 'Other')
      assert.deepEqual(ui.answerFor(0), ['My answer'])
      assert.equal(ui.isOptionChecked(0, 'Fixed'), false)
    } finally {
      app.unmount()
    }
  }))

test('custom multi choice can be combined or deselected and an empty selected custom option blocks confirmation', async () =>
  withChat(async (chat) => {
    let sends = 0
    chat.replyQuestion = async () => {
      sends++
      return true
    }
    const { app, ui, props } = mountActionInteraction(chat, 'ask_user', true)
    Object.assign(props.interaction.questions[0], {
      allowCustom: false,
      options: [
        { label: 'Fixed', description: '' },
        { label: 'Other', description: '', custom: true },
      ],
    })
    await nextTick()
    try {
      ui.toggleOption(0, 'Fixed')
      ui.toggleOption(0, 'Other')
      assert.equal(ui.canSubmit.value, false)
      await ui.confirmQuestion()
      assert.equal(sends, 0)
      ui.customDrafts.value[0] = 'My answer'
      assert.deepEqual(ui.answerFor(0), ['Fixed', 'My answer'])
      ui.toggleOption(0, 'Other')
      assert.deepEqual(ui.answerFor(0), ['Fixed'])
      assert.equal(ui.customDrafts.value[0], 'My answer')
      assert.equal(ui.canSubmit.value, true)
      ui.toggleOption(0, 'Other')
      assert.deepEqual(ui.answerFor(0), ['Fixed', 'My answer'])
    } finally {
      app.unmount()
    }
  }))

test('custom-only requests render one radio option with its own label instead of a separate editing button', async () =>
  withChat(async (chat, pinia) => {
    chat.getPendingInteraction = () => ({ requestId }) as any
    const props = actionInteractionProps('ask_user')
    Object.assign(props.interaction.questions[0], {
      allowCustom: false,
      options: [{ label: 'Write an answer', description: 'Use your own words', custom: true }],
    })
    const app = createSSRApp(interactionComponent, props)
    app.use(pinia)
    app.use(
      createI18n({
        legacy: false,
        locale: 'en-US',
        messages: { 'en-US': {} },
        missingWarn: false,
        fallbackWarn: false,
      }),
    )
    const html = await renderToString(app)
    assert.equal((html.match(/type="radio"/g) || []).length, 1)
    assert.match(html, /<input[^>]*data-interaction-control-kind="custom"/)
    assert.match(html, /Write an answer/)
    assert.match(html, /Use your own words/)
    assert.doesNotMatch(html, /<button[^>]*data-interaction-control-kind="custom"/)
  }))

test('a custom editor follows its own option when mixed with fixed choices and the legacy flag adds no duplicate', async () =>
  withChat(async (chat, pinia) => {
    chat.getPendingInteraction = () => ({ requestId }) as any
    const props = actionInteractionProps('ask_user')
    Object.assign(props.interaction.questions[0], {
      allowCustom: true,
      options: [
        { label: 'Other', description: 'Use your own words', custom: true },
        { label: 'Fixed', description: '' },
        { label: 'Custom', description: 'A fixed value with a reserved legacy name' },
      ],
    })
    const draft = chat.getInteractionDraft('7', requestId)
    draft.selected = [['Other']]
    draft.customOpen = [true]
    draft.customDrafts = ['My answer']
    const app = createSSRApp(interactionComponent, props)
    app.use(pinia)
    app.use(
      createI18n({
        legacy: false,
        locale: 'en-US',
        messages: { 'en-US': {} },
        missingWarn: false,
        fallbackWarn: false,
      }),
    )
    const html = await renderToString(app)
    assert.equal((html.match(/type="radio"/g) || []).length, 3)
    assert.equal((html.match(/data-interaction-control-kind="custom"/g) || []).length, 1)
    assert.match(html, /data-interaction-control-key="question:0:option:Custom"/)
    const custom = html.indexOf('data-interaction-control-key="question:0:custom"')
    const editor = html.indexOf('<textarea')
    const fixed = html.indexOf('data-interaction-control-key="question:0:option:Fixed"')
    assert.ok(custom >= 0 && custom < editor && editor < fixed)
    assert.match(html, /My answer<\/textarea>/)
  }))
