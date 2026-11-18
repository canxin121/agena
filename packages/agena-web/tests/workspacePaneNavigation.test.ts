import { afterEach, expect, mock, spyOn, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import * as vue from 'vue'
import * as vueRouter from 'vue-router'
import * as mainTabs from '../src/app/navigation/mainTabs'
import * as paneContext from '../src/app/workspace/workspacePaneContext'
import * as resourceDirectory from '../src/app/workspace/resourceDirectory'
import { useUiStore } from '../src/stores/ui'

const source = readFileSync(new URL('../src/layout/WorkspacePaneView.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const compiled = transformSync(compileScript(descriptor, { id: 'workspace-pane-navigation-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup()
  mock.restore()
  setActivePinia(undefined)
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

async function flushPaneRoute() {
  await vue.nextTick()
  await Promise.resolve()
  await vue.nextTick()
}

async function workspace() {
  const storage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: undefined })
  cleanups.push(() => {
    if (storage) Object.defineProperty(globalThis, 'localStorage', storage)
    else Reflect.deleteProperty(globalThis, 'localStorage')
  })

  const pinia = createPinia()
  setActivePinia(pinia)
  const ui = useUiStore(pinia)
  const first = ui.openWorkspaceWindow('settings', {
    path: '/settings/permissions',
    query: { scope: 'session', targetSessionId: '7' },
  })
  const second = ui.openWorkspaceWindow('chat', { path: '/chat', query: { sessionId: '9' } })
  ui.setFocusedWorkspaceWindow(first)
  const router = vueRouter.createRouter({
    history: vueRouter.createMemoryHistory(),
    routes: ['/settings/:page?', '/chat', '/bot', '/assistant', '/integrations'].map((path) => ({
      path,
      component: { render: () => null },
    })),
  })
  router.onError(() => {})
  await router.replace({
    path: '/settings/permissions',
    query: { scope: 'session', targetSessionId: '7', windowId: first },
  })

  const scope = vue.effectScope()
  const unmount: Array<() => void> = []
  // This is the shell's MainLayout route commit. The pane must reach it only
  // after Vue Router has accepted the navigation, including async guards.
  scope.run(() =>
    vue.watch(
      router.currentRoute,
      (route) => {
        if (mainTabs.isWorkspaceMainTabPath(route.path)) {
          ui.syncActiveWorkspaceWindowFromRoute(mainTabs.mainTabFromPath(route.path), route.query, route)
        }
      },
      { immediate: true, flush: 'sync' },
    ),
  )
  cleanups.push(() => {
    for (const stop of unmount) stop()
    scope.stop()
    disposePinia(pinia)
  })

  function mount(id: string) {
    const provided = new Map<unknown, unknown>()
    const modules: Record<string, unknown> = {
      vue: {
        ...vue,
        provide: (key: unknown, value: unknown) => provided.set(key, value),
        onBeforeUnmount: (fn: () => void) => unmount.push(fn),
      },
      'vue-router': { ...vueRouter, useRouter: () => router },
      '@/app/navigation/mainTabs': mainTabs,
      '@/app/workspace/workspacePaneContext': paneContext,
      '@/app/workspace/resourceDirectory': resourceDirectory,
      '@/stores/ui': { useUiStore: () => ui },
      '@/stores/directory': { useDirectoryStoreUnscoped: () => ({}) },
      '@/lib/afterPaint': {
        afterPaint: (fn: () => void) => {
          fn()
          return () => {}
        },
      },
    }
    const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => unknown } } }
    new Function('require', 'module', 'exports', compiled)(
      (name: string) => {
        if (!(name in modules)) throw new Error(`Unexpected dependency: ${name}`)
        return modules[name]
      },
      module,
      module.exports,
    )
    scope.run(() => module.exports.default!.setup(vue.reactive({ windowId: id, visible: true }), { expose() {} }))
    return {
      context: provided.get(paneContext.workspacePaneContextKey) as paneContext.WorkspacePaneContext,
      router: provided.get(vueRouter.routerKey) as vueRouter.Router,
    }
  }
  const pane = mount(first)
  await flushPaneRoute()
  const target = (id: string) => ({
    path: '/settings/permissions',
    query: { scope: 'session', targetSessionId: id },
  })
  return { ui, router, first, second, pane, mount, target }
}

test('a dirty target guard observes the old pane and cancellation preserves its route and props', async () => {
  const { ui, router, first, pane, target } = await workspace()
  const entered = deferred<void>()
  const decision = deferred<boolean>()
  const fromPath = router.currentRoute.value.fullPath
  router.beforeEach(() => {
    expect(pane.context.route.value.query.targetSessionId).toBe('7')
    expect(ui.getWorkspaceWindowById(first)?.routeQuery.targetSessionId).toBe('7')
    entered.resolve()
    return decision.promise
  })
  const pending = pane.router.replace(target('8'))
  await entered.promise
  await flushPaneRoute()
  expect(pane.context.route.value.query.targetSessionId).toBe('7')
  expect(router.currentRoute.value.fullPath).toBe(fromPath)
  decision.resolve(false)
  expect(vueRouter.isNavigationFailure(await pending, vueRouter.NavigationFailureType.aborted)).toBe(true)
  await flushPaneRoute()
  expect(ui.getWorkspaceWindowById(first)?.routeQuery.targetSessionId).toBe('7')
  expect(pane.context.route.value.query.targetSessionId).toBe('7')
  expect(router.currentRoute.value.fullPath).toBe(fromPath)
})

test('an approved target change commits once after the guard and coalesces duplicate requests', async () => {
  const { ui, router, first, second, pane, target } = await workspace()
  const entered = deferred<void>()
  const decision = deferred<boolean>()
  const replace = spyOn(router, 'replace')
  let guards = 0
  router.beforeEach(() => {
    guards++
    entered.resolve()
    return decision.promise
  })
  const pending = pane.router.replace(target('8'))
  const duplicate = pane.router.replace(target('8'))
  await entered.promise
  await flushPaneRoute()
  expect(ui.getWorkspaceWindowById(first)?.routeQuery.targetSessionId).toBe('7')
  expect(pane.context.route.value.query.targetSessionId).toBe('7')
  decision.resolve(true)
  await Promise.all([pending, duplicate])
  await flushPaneRoute()
  expect(replace).toHaveBeenCalledTimes(1)
  expect(guards).toBe(1)
  expect(ui.getWorkspaceWindowById(first)?.routeQuery.targetSessionId).toBe('8')
  expect(pane.context.route.value.query.targetSessionId).toBe('8')
  expect(ui.getWorkspaceWindowById(second)?.routeQuery.sessionId).toBe('9')
  expect(router.currentRoute.value.query.windowId).toBe(first)
})

test('a failed navigation preserves the pane and clears its pending entry so it can retry', async () => {
  const { ui, router, first, pane, target } = await workspace()
  let fail = true
  router.beforeEach(() => {
    if (fail) throw new Error('guard failed')
  })
  await expect(pane.router.replace(target('8'))).rejects.toThrow('guard failed')
  await flushPaneRoute()
  expect(ui.getWorkspaceWindowById(first)?.routeQuery.targetSessionId).toBe('7')
  expect(pane.context.route.value.query.targetSessionId).toBe('7')
  fail = false
  await pane.router.replace(target('8'))
  await flushPaneRoute()
  expect(pane.context.route.value.query.targetSessionId).toBe('8')
})

test('a newer focused window navigation cancels the old request without changing either pane', async () => {
  const { ui, router, first, second, pane, target } = await workspace()
  const entered = deferred<void>()
  const decision = deferred<boolean>()
  router.beforeEach((to) => {
    if (to.query.targetSessionId === '8') {
      entered.resolve()
      return decision.promise
    }
  })
  const pending = pane.router.replace(target('8'))
  await entered.promise
  ui.setFocusedWorkspaceWindow(second)
  // MainLayout synchronizes a changed focus to that window's stored location.
  await router.replace({ path: '/chat', query: { sessionId: '9', windowId: second } })
  decision.resolve(true)
  expect(vueRouter.isNavigationFailure(await pending, vueRouter.NavigationFailureType.cancelled)).toBe(true)
  await flushPaneRoute()
  expect(ui.focusedWorkspaceWindowId).toBe(second)
  expect(ui.getWorkspaceWindowById(first)?.routeQuery.targetSessionId).toBe('7')
  expect(pane.context.route.value.query.targetSessionId).toBe('7')
  expect(ui.getWorkspaceWindowById(second)?.routeQuery.sessionId).toBe('9')
  expect(router.currentRoute.value.query.windowId).toBe(second)
})

test('an inactive pane keeps local Bot navigation without changing the focused shell', async () => {
  const { ui, router, first, second, mount } = await workspace()
  const inactive = mount(second)
  const shellPath = router.currentRoute.value.fullPath
  const push = spyOn(router, 'push')
  await inactive.router.push({
    path: '/bot',
    query: { bot: 'worker', sessionId: '42', windowId: first, agenaEmbed: '1' },
    hash: '#activity',
  })
  await flushPaneRoute()
  expect(push).not.toHaveBeenCalled()
  expect(router.currentRoute.value.fullPath).toBe(shellPath)
  expect(ui.focusedWorkspaceWindowId).toBe(first)
  expect(ui.getWorkspaceWindowById(second)?.mainTab).toBe('bot')
  expect(ui.getWorkspaceWindowById(second)?.routeQuery).toEqual({ bot: 'worker', sessionId: '42' })
  expect(inactive.context.route.value.path).toBe('/bot')
  expect(inactive.context.route.value.hash).toBe('#activity')
  await inactive.router.push('/assistant')
  expect(router.currentRoute.value.fullPath).toBe(shellPath)
  expect(inactive.context.route.value.path).toBe('/bot')
})

test('focused chat and Bot navigation retain the origin window and conversation settings retain its pane', async () => {
  const { ui, router, first, second, pane } = await workspace()
  await pane.router.push({ path: '/bot', query: { bot: 'worker', sessionId: '42', windowId: second } })
  await flushPaneRoute()
  expect(router.currentRoute.value.query.windowId).toBe(first)
  expect(ui.getWorkspaceWindowById(first)?.mainTab).toBe('bot')
  expect(pane.context.route.value.query.sessionId).toBe('42')
  expect(ui.getWorkspaceWindowById(second)?.routeQuery.sessionId).toBe('9')
  await pane.router.replace({ path: '/chat', query: { sessionId: '8' } })
  await flushPaneRoute()
  expect(ui.getWorkspaceWindowById(first)?.mainTab).toBe('chat')
  expect(pane.context.route.value.query.sessionId).toBe('8')
  const panePath = pane.context.route.value.fullPath
  await pane.router.push({ path: '/assistant', query: { sessionId: '8' } })
  await flushPaneRoute()
  expect(router.currentRoute.value.path).toBe('/assistant')
  expect(pane.context.route.value.fullPath).toBe(panePath)
  expect(ui.getWorkspaceWindowById(first)?.routePath).toBe('/chat')
})
