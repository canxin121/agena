// Production reply projection, part components and Vim cursor, driven by
// deterministic transcript revisions and the canonical content SSE format.
import { computed, createApp, h, reactive, ref } from 'vue'
import { createPinia } from 'pinia'
import { Terminal } from '@xterm/xterm'
import { i18n } from '../../src/i18n'
import MessageItem from '../../src/components/chat/MessageItem.vue'
import { useChatRenderBlocks } from '../../src/pages/chat/useChatRenderBlocks'
import { useChatTranscriptVim } from '../../src/pages/chat/useChatTranscriptVim'
import '../../src/style.css'
import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-mono/400.css'

i18n.global.locale.value = 'en-US'
const epoch = '00000000-0000-0000-0000-000000000001'
const reference = { resource_id: 'live-shell', kind: 'log' }
const resource = {
  ...reference,
  owner_session_id: 1,
  part_id: 8,
  state: 'active',
  cursor: { epoch, sequence: 0 },
  committed_cursor: { epoch, sequence: 0 },
  total_bytes: 0,
  dropped_bytes: 0,
  retained_ranges: [],
}
const chunks = []
const streams = new Set()
const encoder = new TextEncoder()
const fixture = (window.partControlsFixture = {
  streams,
  terminals: [],
  errors: [],
  detailRequests: [],
  contentRequests: [],
  holdContent: false,
  holdDetails: false,
  failNextDetails: false,
})
const nativeOpen = Terminal.prototype.open
Terminal.prototype.open = function (element) {
  fixture.terminals.push(this)
  return nativeOpen.call(this, element)
}
const nativeFetch = window.fetch.bind(window)
function contentStreamResponse(page, signal, live = false) {
  let active
  const body = new ReadableStream({
    start(controller) {
      active = controller
      if (live) streams.add(controller)
    },
    cancel() {
      streams.delete(active)
    },
  })
  const request = {
    signal,
    resourceId: page.resource.resource_id,
    resolve() {
      if (signal?.aborted) return
      active.enqueue(encoder.encode(`event: content\ndata: ${JSON.stringify(page)}\n\n`))
      if (!live) active.close()
    },
  }
  fixture.contentRequests.push(request)
  signal?.addEventListener(
    'abort',
    () => {
      try {
        active.close()
      } catch {
        /* already closed */
      }
      streams.delete(active)
    },
    { once: true },
  )
  if (!fixture.holdContent) request.resolve()
  return new Response(body, { headers: { 'Content-Type': 'text/event-stream' } })
}
window.fetch = async (input, options) => {
  const url = new URL(String(input), location.href)
  const detail = /\/parts\/8\/tool-sections\/(input|output|metadata)$/.exec(url.pathname)
  if (detail) {
    const shell = fixture.message.parts.find((part) => part.id === '8')
    const body = {
      part_id: 8,
      section: detail[1],
      revision: shell.revision,
      updated_at_ms: shell.updatedAt,
      part_state: shell.partState,
      value: `tool detail revision ${shell.revision}`,
    }
    const request = { body, signal: options?.signal, resolve: () => {} }
    fixture.detailRequests.push(request)
    if (fixture.failNextDetails) {
      fixture.failNextDetails = false
      return Response.json({ message: 'Temporary detail failure' }, { status: 500 })
    }
    if (!fixture.holdDetails) return Response.json(body)
    return new Promise((resolve, reject) => {
      request.resolve = () => resolve(Response.json(body))
      options?.signal?.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), {
        once: true,
      })
    })
  }
  if (url.pathname.endsWith('/content/live-shell/stream')) {
    const after = Number(url.searchParams.get('after') || 0)
    return contentStreamResponse(
      {
        resource: structuredClone(resource),
        chunks: chunks.filter((chunk) => chunk.cursor.sequence > after),
        next_cursor: resource.cursor,
        has_more: false,
        gap: false,
      },
      options?.signal,
      true,
    )
  }
  if (url.pathname.endsWith('/content/live-shell')) {
    const after = Number(url.searchParams.get('after') || 0)
    return Response.json({
      resource,
      chunks: chunks.filter((chunk) => chunk.cursor.sequence > after),
      next_cursor: resource.cursor,
      has_more: false,
      gap: false,
    })
  }
  if (url.pathname.endsWith('/content/loading-text/stream')) {
    const cursor = { epoch, sequence: 1 }
    return contentStreamResponse(
      {
        resource: {
          ...resource,
          resource_id: 'loading-text',
          kind: 'text',
          state: 'complete',
          cursor,
          committed_cursor: cursor,
        },
        chunks: [{ cursor, captured_at_ms: Date.now(), payload: { type: 'text', text: 'Loaded answer content' } }],
        next_cursor: cursor,
        has_more: false,
        gap: false,
      },
      options?.signal,
    )
  }
  if (url.pathname.startsWith('/api/')) return Response.json({})
  return nativeFetch(input, options)
}

function operation(id) {
  return {
    id: String(id),
    type: 'tool',
    agenaKind: 'tool_call',
    agenaRole: 'assistant',
    partState: 'in_progress',
    revision: 1,
    updatedAt: 1,
    agenaContent: {
      name: 'shell.exec',
      call_id: id,
      input: { command: `echo part-${id}` },
      lifecycle: { start_ms: 1 },
    },
    agenaPresentation: {
      title: `Shell ${id}`,
      summary: '',
      blocks:
        id === 8
          ? [
              { type: 'command', id: 'command', command: 'streaming shell' },
              { type: 'content', id: 'output', resource: reference },
            ]
          : [{ type: 'text', id: 'output', text: `Output ${id}` }],
    },
  }
}
const message = reactive({
  info: { id: 'reply', role: 'assistant', runState: 'in_progress' },
  parts: Array.from({ length: 12 }, (_, i) => operation(i + 1)),
})
const chat = reactive({ messages: [message] })
fixture.message = message
fixture.emit = (text) => {
  const chunk = {
    cursor: { epoch, sequence: resource.cursor.sequence + 1 },
    captured_at_ms: Date.now(),
    payload: { type: 'log', stream: 'stdout', text },
  }
  chunks.push(chunk)
  resource.cursor = resource.committed_cursor = chunk.cursor
  resource.total_bytes += encoder.encode(text).length
  resource.retained_ranges = [{ first: 1, last: chunk.cursor.sequence }]
  const page = { resource, chunks: [chunk], next_cursor: chunk.cursor, has_more: false, gap: false }
  for (const stream of streams) stream.enqueue(encoder.encode(`event: content\ndata: ${JSON.stringify(page)}\n\n`))
}
fixture.revise = () => {
  const shell = message.parts.find((part) => part.id === '8')
  shell.revision++
  shell.updatedAt++
  shell.agenaPresentation = { ...shell.agenaPresentation, summary: `revision ${shell.revision}` }
}
fixture.append = (count) => {
  for (let i = 0; i < count; i++) message.parts.push(operation(message.parts.length + 1))
}
fixture.interaction = () => {
  const part = message.parts[2]
  part.agenaContent.user_input = {
    requests: [
      {
        request: {
          request_id: 'ask-3',
          title: 'Recovery choice',
          input_kind: 'ask_user',
          questions: [{ question: 'Resume the operation?', options: [{ label: 'Resume' }], allow_custom: true }],
        },
        reply: null,
      },
    ],
  }
  part.revision++
  part.agenaPresentation = { title: 'Recovery choice', summary: '', blocks: [] }
}
fixture.finish = () => {
  message.info.runState = 'completed'
  const shell = message.parts.find((part) => part.id === '8')
  shell.partState = 'completed'
  shell.revision++
  resource.state = 'complete'
  fixture.emit('Build completed\n')
  fixture.render.collapseAllActivities()
}
fixture.text = () => {
  const id = String(message.parts.length + 1)
  message.parts.push({
    id,
    agenaKind: 'text',
    agenaRole: 'assistant',
    partState: 'completed',
    agenaContent: { resources: [{ resource_id: 'loading-text', kind: 'text' }] },
  })
  return id
}

createApp({
  setup() {
    const pageRef = ref(null),
      scrollEl = ref(null)
    const render = useChatRenderBlocks({ chat, settings: {}, showThinking: computed(() => true), formatTime: () => '' })
    fixture.render = render
    const isPartExpanded = render.transcriptPartExpanded
    const vim = useChatTranscriptVim({
      enabled: ref(true),
      pageRef,
      scrollEl,
      composerRef: ref(null),
      searchInputRef: ref(null),
      selectedSessionId: computed(() => '1'),
      renderBlocks: render.renderBlocks,
      draft: ref(''),
      clearComposer() {},
      canAbort: computed(() => false),
      abortRun() {},
      toggleHelp() {},
      openPlan() {},
      isPartExpanded,
      togglePart: (part, expanded) => render.setActivityExpanded(part.key, expanded),
      toasts: { push() {} },
    })
    const pageSize = ref(5)
    const visibility = reactive({ ids: [] })
    fixture.visibility = visibility
    return () =>
      h('main', { ref: pageRef, class: 'mx-auto max-w-4xl p-6', style: 'font-family:var(--font-sans)' }, [
        h('h1', { class: 'mb-3 text-xl font-semibold' }, 'Part interaction verification'),
        h('div', { ref: scrollEl, class: 'h-[720px] overflow-auto border p-2', 'data-fixture': 'transcript' }, [
          h(
            'div',
            { 'data-transcript-root': 'true' },
            render.renderBlocks.value.map((block) =>
              h(MessageItem, {
                key: block.key,
                message: block.message,
                displayParts: block.displayParts,
                showTimestamps: false,
                formatTime: () => '',
                copiedMessageId: '',
                revertBusyMessageId: '',
                isStreaming: message.info.runState === 'in_progress',
                collapseSignal: render.activityCollapseSignal.value,
                activityPageSize: pageSize.value,
                activityVisibility: visibility,
                isCompactTouch: false,
                isPartExpanded,
                isNodeSelected: vim.isNodeSelected,
                isNodeSearchMatch: vim.isNodeSearchMatch,
                sessionId: '1',
                onPartToggle: (part, expanded) => render.setActivityExpanded(part.key, expanded),
                onNodeSelect: vim.selectNode,
                onSetActivityPageSize: (size) => {
                  pageSize.value = size
                },
              }),
            ),
          ),
        ]),
      ])
  },
})
  .use(createPinia())
  .use(i18n)
  .mount('#app')
