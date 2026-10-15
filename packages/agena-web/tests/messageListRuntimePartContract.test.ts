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

test('provider retry and run loading render accessible status without synthetic transcript parts', async () => {
  ensureBrowserTestRuntime()
  const { default: MessageList } = await vite.ssrLoadModule('/src/components/chat/MessageList.vue')
  const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
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
    assert.match(retry, /role="status"/)
    assert.ok(retry.includes('Retry attempt 2'))
    assert.ok(retry.includes('Provider temporarily unavailable · 1.5s'))
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
  } finally {
    i18n.global.locale.value = locale
  }
})
