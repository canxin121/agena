import { afterAll, expect, test } from 'bun:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createRenderer, h, nextTick, reactive, ssrContextKey } from 'vue'
import { createI18n } from 'vue-i18n'
import { createPinia } from 'pinia'
import { ensureBrowserTestRuntime } from './testRuntime'

ensureBrowserTestRuntime()
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())
const { default: PathPicker } = await vite.ssrLoadModule('/src/components/ui/PathPicker.vue')
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
const settle = async () => {
  for (let i = 0; i < 8; i++) await nextTick()
}

test('late folder listings preserve typed paths and cannot replace newer browser navigation', async () => {
  const originalFetch = globalThis.fetch
  const requests: Array<{ path: string; resolve: (value: Response) => void }> = []
  globalThis.fetch = ((url: string) =>
    new Promise<Response>((resolve) => {
      requests.push({ path: new URL(url, 'http://test.local').searchParams.get('path') || '', resolve })
    })) as typeof fetch
  const props = reactive({ modelValue: '/a', view: 'browser', mode: 'directory', resolveToAbsolute: true })
  let state: any
  const subject = {
    ...PathPicker,
    setup(props: object, context: object) {
      state = PathPicker.setup(props, context)
      return () => null
    },
  }
  const app = renderer.createApp({
    render: () =>
      h(subject, {
        ...props,
        'onUpdate:modelValue': (path: string) => {
          props.modelValue = path
        },
      }),
  })
  app.provide(ssrContextKey, {})
  app.use(createPinia())
  app.use(createI18n({ legacy: false, locale: 'en', messages: { en: {} }, missingWarn: false, fallbackWarn: false }))
  const complete = (index: number) =>
    requests[index]!.resolve(Response.json({ path: requests[index]!.path, entries: [] }))
  try {
    app.mount({})
    expect(requests[0]!.path).toBe('/a')
    state.value.value = '/typed-destination'
    await nextTick()
    complete(0)
    await settle()
    expect(props.modelValue).toBe('/typed-destination')

    const older = state.loadBrowser('/older')
    const newer = state.loadBrowser('/newer')
    complete(2)
    await newer
    complete(1)
    await older
    expect(state.browserPath.value).toBe('/newer')
    expect(props.modelValue).toBe('/newer')
    expect(state.browserLoading.value).toBe(false)

    const retired = state.loadBrowser('/closed-dialog')
    app.unmount()
    complete(3)
    await retired
    expect(props.modelValue).toBe('/newer')
  } finally {
    if (app._instance) app.unmount()
    globalThis.fetch = originalFetch
  }
})
