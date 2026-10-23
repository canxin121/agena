import { afterAll, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { createRenderer, defineComponent, nextTick, ssrContextKey } from 'vue'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { ensureBrowserTestRuntime } from './testRuntime'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  optimizeDeps: { noDiscovery: true, entries: [] },
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())
const { default: component } = await vite.ssrLoadModule('/src/components/chat/QueuedMessageEditor.vue')
const { useChatStore } = await vite.ssrLoadModule('/src/stores/chat.ts')
const { i18n } = await vite.ssrLoadModule('/src/i18n/index.ts')
const renderer = createRenderer<object, object>({
  patchProp() {},
  insert() {},
  remove() {},
  setElementText() {},
  setText() {},
  setComment() {},
  createElement: () => ({}),
  createText: () => ({}),
  createComment: () => ({}),
  parentNode: () => null,
  nextSibling: () => null,
})
const original = [{ type: 'text', text: 'queued text' }]
async function dialog(run: (ui: any, chat: any, events: string[]) => Promise<void>, loadFails = false) {
  ensureBrowserTestRuntime()
  const previousFetch = globalThis.fetch
  const pinia = createPinia()
  setActivePinia(pinia)
  const chat = useChatStore()
  chat.setComposerDraft('7', 'main draft')
  const events: string[] = []
  globalThis.fetch = (async () =>
    loadFails
      ? Response.json({ error: 'message already started' }, { status: 409 })
      : Response.json({ execution_id: 'pending', message_id: 42, revision: 3, document: original })) as typeof fetch
  let ui: any
  const app = renderer.createApp(
    defineComponent({
      setup() {
        ui = component.setup(
          { sessionId: '7', executionId: 'pending' },
          { expose() {}, emit: (event: string) => events.push(event) },
        )
        ui.composer.value = {
          getSegments: () => [{ type: 'text', text: ui.draft.value }],
          restoreSegments() {},
          removeAttachmentNodes() {},
          textareaEl: { focus() {} },
        }
        return () => null
      },
    }),
  )
  app.use(pinia)
  app.use(i18n)
  app.provide(ssrContextKey, { modules: new Set() })
  try {
    app.mount({})
    for (let i = 0; ui.loading.value && i < 40; i++) await new Promise((resolve) => setTimeout(resolve, 1))
    await nextTick()
    await run(ui, chat, events)
    expect(chat.getComposerDraft('7')).toBe('main draft')
  } finally {
    app.unmount()
    disposePinia(pinia)
    globalThis.fetch = previousFetch
  }
}

test('modal cancellation leaves the queued document and the conversation draft intact', async () => {
  await dialog(async (ui, chat, events) => {
    let writes = 0
    chat.editQueuedMessage = async () => {
      writes++
    }
    expect(ui.loaded.value).toBe(true)
    expect(ui.draft.value).toBe('queued text')
    ui.draft.value = 'unsaved modal change'
    ui.close()
    expect(events).toEqual(['close'])
    expect(writes).toBe(0)
    expect(original).toEqual([{ type: 'text', text: 'queued text' }])
  })
})

test('a failed save retains edits and a successful retry uses the original identity and revision', async () => {
  await dialog(async (ui, chat, events) => {
    const writes: unknown[][] = []
    chat.editQueuedMessage = async (...args: unknown[]) => {
      writes.push(args)
      if (writes.length === 1) throw new Error('version conflict')
    }
    ui.draft.value = 'edited modal text'
    await ui.save()
    expect(ui.error.value).toBe('version conflict')
    expect(ui.saving.value).toBe(false)
    expect(ui.draft.value).toBe('edited modal text')
    expect(events).toEqual([])
    await ui.save()
    expect(writes[1]).toEqual(['7', 'pending', 3, [{ type: 'text', text: 'edited modal text' }]])
    expect(events).toEqual(['close'])
  })
})

test('new attachments upload once across a queue save retry and use valid activity identities', async () => {
  await dialog(async (ui, chat) => {
    let uploads = 0
    let writes = 0
    let document: any
    chat.uploadWorkspaceAttachment = async () => {
      uploads++
      return { path: 'uploads/new.png', sha256: 'frozen', size_bytes: 4 }
    }
    chat.editQueuedMessage = async (_sid: string, _execution: string, _revision: number, value: unknown) => {
      document = value
      if (++writes === 1) throw new Error('temporary write failure')
    }
    ui.attachedFiles.value = [
      {
        id: 'pending-local',
        filename: 'new.png',
        mime: 'image/png',
        size: 4,
        blob: new File(['data'], 'new.png', { type: 'image/png' }),
        delivery: 'model_input',
        state: 'ready',
      },
    ]
    await ui.save()
    expect(ui.attachedFiles.value).toHaveLength(1)
    expect(ui.error.value).toBe('temporary write failure')
    await ui.save()
    expect(uploads).toBe(1)
    expect(document[1].activity.id).toMatch(/^[0-9a-f]{8}-[0-9a-f-]{27}$/)
    expect(document[1].activity.payload.reference.path).toBe('uploads/new.png')
    expect(document[1].activity.provenance.content_hash).toBe('frozen')
  })
})

test('a failed load finishes the loading state and cannot save an empty replacement', async () => {
  await dialog(async (ui) => {
    expect(ui.loading.value).toBe(false)
    expect(ui.loaded.value).toBe(false)
    expect(ui.canSave.value).toBe(false)
    expect(ui.error.value).toBe('message already started')
  }, true)
})
