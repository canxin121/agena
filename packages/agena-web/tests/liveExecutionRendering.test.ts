import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createRenderer, h, nextTick, reactive, ref, ssrContextKey, type Ref } from 'vue'
import { createI18n } from 'vue-i18n'
import { createPinia, disposePinia } from 'pinia'
import { useVisibleResource } from '../src/pages/chat/useVisibleResource'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
  plugins: [
    {
      name: 'ssr-memory-history',
      transform(code, id) {
        if (id.endsWith('/src/router.ts')) return code.replaceAll('createWebHistory', 'createMemoryHistory')
      },
    },
  ],
})
after(() => vite.close())
const { default: operationComponent } = await vite.ssrLoadModule('/src/components/chat/AgenaOperationPart.vue')

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

async function withClock(
  run: (clock: { advance(ms: number): Promise<void>; settle(): Promise<void>; document: Document }) => Promise<void>,
) {
  const original = {
    document: globalThis.document,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    now: Date.now,
  }
  let now = 100_000,
    serial = 0
  const timers = new Map<number, { at: number; callback: () => void }>()
  const document = Object.assign(new EventTarget(), { hidden: false }) as unknown as Document
  Object.assign(globalThis, {
    document,
    setTimeout: (callback: () => void, delay: number) => {
      const id = ++serial
      timers.set(id, { at: now + delay, callback })
      return id
    },
    clearTimeout: (id: number) => timers.delete(id),
  })
  Date.now = () => now
  const settle = async () => {
    for (let i = 0; i < 20; i++) await nextTick()
  }
  const advance = async (ms: number) => {
    const end = now + ms
    while (true) {
      const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
      if (!next) break
      const [id, timer] = next
      now = timer.at
      timers.delete(id)
      timer.callback()
      await settle()
    }
    now = end
    await settle()
  }
  try {
    await run({ advance, settle, document })
  } finally {
    Date.now = original.now
    Object.assign(globalThis, {
      document: original.document,
      setTimeout: original.setTimeout,
      clearTimeout: original.clearTimeout,
    })
  }
}

test('an invalidation burst during a visible read produces one trailing read before the idle heartbeat', async () =>
  withClock(async ({ advance, settle }) => {
    const calls: Array<{ resolve(value: string): void }> = []
    let state!: ReturnType<typeof useVisibleResource<string>>
    const app = renderer.createApp({
      setup() {
        state = useVisibleResource({
          key: ref('shell_a'),
          minInterval: 250,
          interval: () => 30_000,
          load: () => new Promise<string>((resolve) => calls.push({ resolve })),
        })
        return () => null
      },
    })
    try {
      app.mount({})
      for (let i = 0; i < 100; i++) void state.refresh()
      assert.equal(calls.length, 1)
      calls[0]!.resolve('running output')
      await settle()
      assert.equal(state.data.value, 'running output')
      await advance(249)
      assert.equal(calls.length, 1)
      await advance(1)
      assert.equal(calls.length, 2)
      calls[1]!.resolve('final output')
      await settle()
      assert.equal(state.data.value, 'final output')
      await advance(10_000)
      assert.equal(calls.length, 2, 'a burst must settle after one trailing read')
    } finally {
      app.unmount()
    }
  }))

type DetailState = {
  sectionValues: Ref<Record<string, unknown>>
  sectionErrors: Ref<Record<string, string>>
  toggleDetails(): void
  toggleSection(section: string): Promise<void>
}

test('running details retain useful snapshots, catch up under continuous revisions and reject an old value at completion', async () =>
  withClock(async ({ advance, settle }) => {
    const props = reactive({
      part: { id: '4', status: 'in_progress', source: { revision: 1, updatedAt: 1 } },
      expanded: true,
      collapseSignal: 0,
      sessionId: '7',
    })
    let state!: DetailState
    const subject = {
      ...operationComponent,
      setup(props: object, context: object) {
        state = operationComponent.setup(props, context)
        return () => null
      },
    }
    const app = renderer.createApp({ setup: () => () => h(subject, props) })
    app.provide(ssrContextKey, {})
    const pinia = createPinia()
    app.use(pinia)
    app.use(createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': {} } }))
    const requests: Array<{ signal: AbortSignal; resolve(response: Response): void }> = []
    const originalFetch = globalThis.fetch
    globalThis.fetch = ((_, init) =>
      new Promise<Response>((resolve, reject) => {
        requests.push({ signal: init!.signal as AbortSignal, resolve })
        init?.signal?.addEventListener('abort', () => reject(init.signal!.reason), { once: true })
      })) as typeof fetch
    const reply = (index: number, revision: number, value: string) =>
      requests[index]!.resolve(
        Response.json({
          part_id: 4,
          section: 'output',
          revision,
          updated_at_ms: revision,
          value,
        }),
      )
    try {
      app.mount({})
      state.toggleDetails()
      await settle()
      const initial = state.toggleSection('output')
      await settle()
      reply(0, 1, 'first')
      await initial
      props.part.source = { revision: 2, updatedAt: 2 }
      await settle()
      assert.equal(state.sectionValues.value.output, 'first')
      await advance(1000)
      assert.equal(requests.length, 2)
      assert.equal(state.sectionValues.value.output, 'first', 'refreshing does not blank the open section')
      props.part.source = { revision: 10, updatedAt: 10 }
      await settle()
      reply(1, 2, 'intermediate')
      await settle()
      assert.equal(state.sectionValues.value.output, 'intermediate', 'continuous output cannot starve rendering')
      assert.equal(state.sectionErrors.value.output, '')
      await advance(2000)
      assert.equal(requests.length, 3, 'a newer source triggers a catch-up read')
      props.part.status = 'completed'
      props.part.source = { revision: 11, updatedAt: 11 }
      await settle()
      assert.equal(requests[2]!.signal.aborted, true)
      assert.equal(requests.length, 3, 'completion cancels stale output while retaining the read budget')
      await advance(1000)
      assert.equal(requests.length, 4)
      reply(3, 11, 'final result')
      await settle()
      reply(2, 10, 'late running output')
      await settle()
      assert.equal(state.sectionValues.value.output, 'final result')
      assert.equal(state.sectionErrors.value.output, '')
    } finally {
      app.unmount()
      disposePinia(pinia)
      globalThis.fetch = originalFetch
    }
  }))
