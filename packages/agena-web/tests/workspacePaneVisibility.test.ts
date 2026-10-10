import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'
import { createRenderer, ssrContextKey, type Component } from 'vue'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { ensureBrowserTestRuntime } from './testRuntime'

ensureBrowserTestRuntime()
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
after(() => vite.close())
const { default: WorkspacePaneView } = await vite.ssrLoadModule('/src/layout/WorkspacePaneView.vue')
const { workspacePaneContextKey } = await vite.ssrLoadModule('/src/app/workspace/workspacePaneContext.ts')

const renderer = createRenderer({
  createElement: () => ({}),
  createText: () => ({}),
  createComment: () => ({}),
  insert() {},
  remove() {},
  setText() {},
  setElementText() {},
  patchProp() {},
  parentNode: () => null,
  nextSibling: () => null,
})

for (const [name, supplied, expected] of [
  ['primary panes stay visible when the optional Boolean prop is omitted', undefined, true],
  ['inactive desktop panes remain hidden', false, false],
  ['active desktop panes remain visible', true, true],
] as const) {
  test(name, async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: '/:pathMatch(.*)*', component: { render: () => null } }],
    })
    await router.push('/chat')
    const props = supplied === undefined ? { windowId: '' } : { windowId: '', visible: supplied }
    // Run the real SFC setup; its SSR template is outside this visibility test.
    const app = renderer.createApp({ ...WorkspacePaneView, render: () => null } as Component, props)
    app.provide(ssrContextKey, {})
    app.use(createPinia()).use(router)
    app.mount({})
    try {
      // Exercise Vue's absent-Boolean coercion as well as explicit visibility.
      assert.equal(app._instance?.props.visible, expected)
      assert.equal(app._instance?.provides[workspacePaneContextKey].isVisible.value, expected)
    } finally {
      app.unmount()
    }
  })
}
