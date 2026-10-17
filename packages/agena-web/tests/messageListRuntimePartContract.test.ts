import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createSSRApp } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { createPinia, disposePinia } from 'pinia'
import { ensureBrowserTestRuntime } from './testRuntime'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  optimizeDeps: { noDiscovery: true, include: [] },
  plugins: [
    {
      name: 'runtime-status-ssr-router',
      enforce: 'pre',
      transform(code, id) {
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
      },
    },
  ],
})
after(() => vite.close())

test('runtime loading stays accessible and execution metadata does not invent error Parts', async () => {
  ensureBrowserTestRuntime()
  const { default: MessageList } = await vite.ssrLoadModule('/src/components/chat/MessageList.vue')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
  const { normalizeAgenaPart } = await vite.ssrLoadModule('/src/stores/chat/api.ts')
  const { projectTranscriptBlocks } = await vite.ssrLoadModule('/src/pages/chat/transcriptProjection.ts')
  const locale = i18n.global.locale.value
  i18n.global.locale.value = 'en-US'
  const render = async (extra: Record<string, unknown> = {}) => {
    const pinia = createPinia()
    const app = createSSRApp(MessageList, {
      selectedSessionId: '1',
      renderBlocks: [],
      isCompactLayout: false,
      isCompactTouch: false,
      messagesLoading: false,
      messagesError: null,
      pendingInitialScrollSessionId: null,
      loadingOlder: false,
      activityPageSize: 10,
      showTimestamps: false,
      formatTime: () => '',
      copiedMessageId: '',
      revertBusyMessageId: '',
      isStreamingAssistantMessage: () => false,
      showRunLoading: false,
      sessionEnded: false,
      retryStatus: null,
      currentPhase: 'busy',
      activityCollapseSignal: 0,
      isPartExpanded: () => false,
      ...extra,
    })
      .use(pinia)
      .use(i18n)
    try {
      return await renderToString(app)
    } finally {
      disposePinia(pinia)
    }
  }
  try {
    const retry = await render({
      retryStatus: { type: 'retry', attempt: 2, message: 'Provider temporarily unavailable', next: 5000 },
      retryCountdown: '1.5s',
    })
    assert.ok(!retry.includes('Retry attempt 2'))
    assert.ok(!retry.includes('Provider temporarily unavailable'))
    assert.ok(!retry.includes('data-part-loading'))

    const loading = await render({ showRunLoading: true })
    assert.match(loading, /role="status"/)
    assert.ok(loading.includes('data-part-loading'))
    assert.ok(loading.includes('aria-live="polite"'))

    const idle = await render({ currentPhase: 'idle', sessionEnded: true })
    assert.ok(!idle.includes('data-part-loading'))
    assert.ok(!idle.includes('role="status"'))
    for (const html of [retry, loading, idle]) {
      assert.ok(!html.includes('<article'), 'runtime state does not invent a transcript message')
      assert.ok(!html.includes('data-part-kind='), 'only durable parts become transcript rows')
    }

    const problem = {
      code: 'provider.timeout',
      category: 'timeout',
      user: { fallback: 'The provider request timed out.' },
      retry: 'backoff',
    }
    const durable = normalizeAgenaPart('102', '1', '101', {
      part_id: 102,
      run_id: 101,
      kind: 'error',
      state: 'failed',
      role: 'assistant',
      content: { problem, retry: { attempt: 3, max_retries: 3, outcome: 'recovered' } },
    })
    const recovered = await render({
      renderBlocks: projectTranscriptBlocks(
        [
          {
            info: {
              id: '101',
              role: 'assistant',
              runState: 'completed',
              error: { message: 'client metadata must stay invisible' },
            },
            parts: [durable],
          },
        ],
        { showReasoning: true },
      ),
      isPartExpanded: () => true,
      sessionError: { error: { message: 'legacy session event must stay invisible' } },
    })
    assert.equal((recovered.match(/data-part-kind="error"/g) || []).length, 1)
    assert.ok(recovered.includes('data-part-id="102"'))
    assert.ok(recovered.includes('data-toggleable="true"'))
    assert.ok(recovered.includes('The provider request timed out.'))
    assert.ok(recovered.includes('3/3 · Completed'))
    assert.ok(!recovered.includes('client metadata must stay invisible'))
    assert.ok(!recovered.includes('legacy session event must stay invisible'))

    for (const [progress, title, statusText] of [
      [
        { attempt: 2, max_retries: 10, outcome: 'retrying', phase: 'backoff', next_at_ms: Date.now() + 60_000 },
        'Waiting to retry',
        'Retry in',
      ],
      [
        { attempt: 2, max_retries: 10, outcome: 'retrying', phase: 'requesting', started_at_ms: Date.now() - 12_000 },
        'Retrying',
        'Waiting for response',
      ],
      [
        { attempt: 2, max_retries: 10, outcome: 'retrying', phase: 'streaming', started_at_ms: Date.now() - 15_000 },
        'Retrying',
        'Receiving response',
      ],
      [{ attempt: 10, max_retries: 10, outcome: 'failed' }, 'Retry failed', 'All 10 retries used'],
      [{ attempt: 2, max_retries: 10, outcome: 'cancelled' }, 'Retry cancelled', 'Automatic retry stopped'],
      [{ attempt: 2, max_retries: 10, outcome: 'recovered' }, 'Recovered', 'Recovered after 2 retries'],
    ] as const) {
      const error = normalizeAgenaPart('102', '1', '101', {
        part_id: 102,
        run_id: 101,
        kind: 'error',
        state: 'failed',
        role: 'assistant',
        content: { problem, retry: progress },
      })
      const html = await render({
        renderBlocks: projectTranscriptBlocks([{ info: { id: '101', role: 'assistant' }, parts: [error] }], {
          showReasoning: true,
        }),
      })
      assert.equal((html.match(/data-part-kind="error"/g) || []).length, 1)
      assert.ok(html.includes('data-part-id="102"'))
      assert.ok(html.includes('data-transcript-vim-toggle="true"'))
      assert.ok(html.includes('aria-expanded="false"'))
      assert.ok(html.includes('data-error-part-headline'))
      assert.ok(html.includes('data-error-part-progress'))
      assert.ok(html.includes(title), title)
      assert.ok(html.includes(statusText), statusText)
      assert.ok(html.includes('The provider request timed out.'))
      assert.ok(!html.includes('<dt'), 'details stay folded while progress remains visible')
    }
  } finally {
    i18n.global.locale.value = locale
  }
})
