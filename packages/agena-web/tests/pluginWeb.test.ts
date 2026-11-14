import { describe, expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { usePluginSessionRefresh } from '../src/features/pluginWeb/usePluginSessionRefresh'
import { createPluginUI, type PluginUIHost, type PluginUIRequest } from '../src/features/pluginWeb/createSdk'
import { PluginPageMount } from '../src/features/pluginWeb/mount'
import { PluginStyleRegistry } from '../src/features/pluginWeb/styles'
import { parsePluginPages, pluginAssetUrl, pluginPagePath, pluginPageSession } from '../src/features/pluginWeb/model'
import { isWorkspaceMainTabPath, mainTabFromPath, normalizeMainTabPath } from '../src/app/navigation/mainTabs'
import type { AgenaPluginUI, PluginPageCatalogItem, PluginUIEnvironment } from '../src/features/pluginWeb/sdk'

const origin = 'http://localhost:3210'
const fixture = (): PluginPageCatalogItem => ({
  plugin_id: 'demo.board',
  page_id: 'dashboard',
  title: 'Board',
  api_version: 1,
  scope: 'instance',
  entry_url: '/plugin-assets/demo.board/revision/main.js',
  style_urls: ['/plugin-assets/demo.board/revision/main.css'],
  commands: ['list', 'update'],
  settings: false,
  revision: 'revision',
  generation: 'runtime-1',
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

function hostFixture(overrides: Partial<PluginUIHost> = {}) {
  const controller = new AbortController()
  const calls: Array<{ path: string; init?: RequestInit }> = []
  const environment: PluginUIEnvironment = Object.freeze({ locale: 'en-US', theme: 'light' })
  const listeners = new Set<(value: PluginUIEnvironment) => void>()
  let refreshes = 0
  const host: PluginUIHost = {
    page: fixture(),
    signal: controller.signal,
    sessionId: null,
    path: '/',
    request: (async (path, init) => {
      calls.push({ path, init })
      return { result: { status: 'succeeded', title: '', summary: '', output: [1] } }
    }) as PluginUIRequest,
    environment: () => environment,
    subscribeEnvironment(listener) {
      listeners.add(listener)
      listener(environment)
      return () => {
        listeners.delete(listener)
      }
    },
    openPage: async () => {},
    setTitle: () => {},
    notify: () => {},
    refresh: () => {
      refreshes++
    },
    messages: { inactive: 'inactive', unavailable: 'unavailable', sessionRequired: 'session required' },
    ...overrides,
  }
  return {
    sdk: createPluginUI(host),
    controller,
    calls,
    listeners,
    get refreshes() {
      return refreshes
    },
  }
}

describe('plugin Web discovery and page identities', () => {
  test('assets stay in the declared plugin revision after URL normalization', () => {
    const page = fixture()
    expect(pluginAssetUrl(page, page.entry_url, origin)).toBe(origin + page.entry_url)
    for (const path of [
      'https://other.example/main.js',
      '//other.example/main.js',
      '/plugin-assets/other/revision/main.js',
      '/plugin-assets/demo.board/old/main.js',
      '/plugin-assets/demo.board/revision/%2e%2e/secret.js',
      '/plugin-assets/demo.board/revision/%252e%252e/secret.js',
      '/plugin-assets/demo.board/revision/a%2fb.js',
      '/plugin-assets/demo.board/revision/main.js?token=secret',
      '/plugin-assets/demo.board/revision/main.js#fragment',
      '/plugin-assets/demo.board/revision/',
    ])
      expect(() => pluginAssetUrl(page, path, origin)).toThrow()
  })

  test('catalog validates every entry and retains omitted empty catalogs', () => {
    expect(parsePluginPages(undefined, origin)).toEqual([])
    const page = fixture()
    const [copy] = parsePluginPages([page], origin)
    expect(copy).toEqual(page)
    expect(copy!.commands).not.toBe(page.commands)
    expect(() => parsePluginPages([page, page], origin)).toThrow()
    expect(() => parsePluginPages([{ ...page, api_version: 2 }], origin)).toThrow()
    expect(() => parsePluginPages([{ ...page, style_urls: ['/settings'] }], origin)).toThrow()
  })

  test('plugin routes survive workspace normalization without matching other path prefixes', () => {
    const path = pluginPagePath('demo.board', 'dashboard')
    expect(path).toBe('/plugins/demo.board/dashboard')
    expect(normalizeMainTabPath('plugins', path + '/detail/42')).toBe(path + '/detail/42')
    expect(mainTabFromPath(path)).toBe('plugins')
    expect(isWorkspaceMainTabPath('/plugins')).toBe(true)
    expect(isWorkspaceMainTabPath('/plugins-unknown')).toBe(false)
    expect(normalizeMainTabPath('chat', path)).toBe('/chat')
    expect(pluginPagePath('demo.board', 'other')).not.toBe(path)
  })

  test('session scope accepts only explicit positive safe integers', () => {
    expect(pluginPageSession('42')).toBe(42)
    for (const value of ['', '0', '-1', '1.5', '1e3', '9007199254740992', ['42'], 42])
      expect(pluginPageSession(value)).toBeNull()
  })
})

describe('plugin Web SDK ownership', () => {
  test('commands bind page, generation and input without exposing an authentication parameter', async () => {
    const { sdk, calls, controller } = hostFixture()
    const result = await sdk.commands.invoke('update', { value: 3 })
    expect(result.status).toBe('succeeded')
    expect(calls).toHaveLength(1)
    expect(calls[0]!.path).toBe('/api/v1/plugins/demo.board/web/dashboard/commands/update')
    expect(JSON.parse(String(calls[0]!.init!.body))).toEqual({
      generation: 'runtime-1',
      input: { value: 3 },
      session_id: null,
    })
    expect(calls[0]!.init!.signal).toBe(controller.signal)
    expect(Object.keys(sdk)).not.toContain('token')
    expect(Object.isFrozen(sdk)).toBe(true)
  })

  test('undeclared operations never reach the request layer', async () => {
    const { sdk, calls } = hostFixture()
    await expect(sdk.commands.invoke('foreign')).rejects.toThrow('unavailable')
    expect(() => sdk.settings.read()).toThrow('unavailable')
    expect(calls).toHaveLength(0)
  })

  test('an instance page does not borrow a globally selected session', async () => {
    const page = { ...fixture(), scope: 'session' as const }
    const { sdk, calls } = hostFixture({ page })
    await expect(sdk.commands.invoke('list')).rejects.toThrow('session required')
    expect(calls).toHaveLength(0)
  })

  test('a response settling after page disposal cannot return data to that page', async () => {
    const response = deferred<{ result: { status: string } }>()
    const { sdk, controller } = hostFixture({ request: (() => response.promise) as PluginUIRequest })
    const pending = sdk.commands.invoke('list')
    controller.abort()
    response.resolve({ result: { status: 'succeeded' } })
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
    await expect(sdk.commands.invoke('list')).rejects.toMatchObject({ name: 'AbortError' })
  })

  test('environment subscriptions are removed on page disposal', () => {
    const { sdk, controller, listeners } = hostFixture()
    const received: string[] = []
    const stop = sdk.subscribeEnvironment((environment) => received.push(environment.locale))
    expect(received).toEqual(['en-US'])
    expect(listeners.size).toBe(1)
    controller.abort()
    expect(listeners.size).toBe(0)
    stop()
    expect(listeners.size).toBe(0)
  })

  test('stale-generation responses trigger a catalog refresh', async () => {
    const failure = Object.assign(new Error('stale'), { status: 409 })
    const fixture = hostFixture({
      request: (async () => {
        throw failure
      }) as PluginUIRequest,
    })
    await expect(fixture.sdk.commands.invoke('list')).rejects.toThrow('stale')
    expect(fixture.refreshes).toBe(1)
  })

  test('settings stay bound to their declared page and refresh after reload', async () => {
    const calls: Array<{ path: string; init?: RequestInit }> = []
    const context = hostFixture({
      page: { ...fixture(), settings: true },
      request: (async (path, init) => {
        calls.push({ path, init })
        return { reload_required: true, settings: {} }
      }) as PluginUIRequest,
    })
    await context.sdk.settings.read()
    await context.sdk.settings.update({ color: 'blue' })
    expect(calls[0]!.path).toEndWith('/web/dashboard/settings?generation=runtime-1')
    expect(JSON.parse(String(calls[1]!.init!.body))).toEqual({ generation: 'runtime-1', value: { color: 'blue' } })
    expect(context.refreshes).toBe(1)
  })
})

class FakeElement extends EventTarget {
  readonly style: Record<string, string> = {}
  readonly dataset: Record<string, string> = {}
  readonly children: FakeElement[] = []
  parent: FakeElement | null = null
  className = ''
  rel = ''
  href = ''
  constructor(readonly ownerDocument: FakeDocument) {
    super()
  }
  append(child: FakeElement) {
    child.parent = this
    this.children.push(child)
  }
  remove() {
    if (this.parent) this.parent.children.splice(this.parent.children.indexOf(this), 1)
    this.parent = null
  }
}
class FakeDocument {
  head = new FakeElement(this)
  createElement() {
    return new FakeElement(this)
  }
}
const element = (value: FakeElement) => value as unknown as HTMLElement

describe('plugin page lifecycle', () => {
  test('navigation during import never mounts the retired module', async () => {
    const document = new FakeDocument()
    const container = new FakeElement(document)
    const loaded = deferred<unknown>()
    let mounts = 0,
      releases = 0
    const lifecycle = new PluginPageMount(() => {})
    const pending = lifecycle.mount({
      container: element(container),
      load: () => loaded.promise,
      styles: async () => () => {
        releases++
      },
      sdk: () => hostFixture().sdk,
    })
    lifecycle.dispose()
    loaded.resolve({
      mount() {
        mounts++
        return { dispose() {} }
      },
    })
    expect(await pending).toBe(false)
    expect(mounts).toBe(0)
    expect(container.children).toHaveLength(0)
    expect(releases).toBe(1)
  })

  test('a late mount is disposed once while a new page remains mounted', async () => {
    const document = new FakeDocument()
    const container = new FakeElement(document)
    const firstHandle = deferred<{ dispose(): void }>()
    const firstStarted = deferred<void>()
    let oldDisposals = 0,
      newDisposals = 0
    let oldSignal: AbortSignal | undefined
    const lifecycle = new PluginPageMount(() => {})
    const first = lifecycle.mount({
      container: element(container),
      load: async () => ({
        mount(_root: HTMLElement, sdk: AgenaPluginUI) {
          oldSignal = sdk.signal
          firstStarted.resolve()
          return firstHandle.promise
        },
      }),
      styles: async () => () => {},
      sdk: (signal) => hostFixture({ signal }).sdk,
    })
    await firstStarted.promise
    await lifecycle.mount({
      container: element(container),
      load: async () => ({
        mount() {
          return {
            dispose() {
              newDisposals++
            },
          }
        },
      }),
      styles: async () => () => {},
      sdk: (signal) => hostFixture({ signal }).sdk,
    })
    expect(oldSignal!.aborted).toBe(true)
    firstHandle.resolve({
      dispose() {
        oldDisposals++
      },
    })
    expect(await first).toBe(false)
    expect(container.children).toHaveLength(1)
    expect(oldDisposals).toBe(1)
    expect(newDisposals).toBe(0)
    lifecycle.dispose()
    lifecycle.dispose()
    expect(newDisposals).toBe(1)
    expect(container.children).toHaveLength(0)
  })

  test('invalid entries retire their DOM and styles', async () => {
    const document = new FakeDocument()
    const container = new FakeElement(document)
    let released = 0
    const lifecycle = new PluginPageMount(() => {})
    await expect(
      lifecycle.mount({
        container: element(container),
        load: async () => ({}),
        styles: async () => () => {
          released++
        },
        sdk: () => hostFixture().sdk,
      }),
    ).rejects.toThrow('mount')
    expect(released).toBe(1)
    expect(container.children).toHaveLength(0)
  })
})

describe('plugin styles across workspace panes', () => {
  test('the last page releases a shared stylesheet', async () => {
    const document = new FakeDocument()
    const registry = new PluginStyleRegistry(document as unknown as Document)
    const first = new AbortController()
    const second = new AbortController()
    const a = registry.acquire(['/shared.css'], first.signal)
    const b = registry.acquire(['/shared.css'], second.signal)
    expect(document.head.children).toHaveLength(1)
    document.head.children[0]!.dispatchEvent(new Event('load'))
    const [releaseA, releaseB] = await Promise.all([a, b])
    first.abort()
    releaseA()
    expect(document.head.children).toHaveLength(1)
    releaseB()
    expect(document.head.children).toHaveLength(0)
  })

  test('a failed stylesheet also releases the other requested styles', async () => {
    const document = new FakeDocument()
    const registry = new PluginStyleRegistry(document as unknown as Document)
    const promise = registry.acquire(['/missing.css', '/other.css'], new AbortController().signal)
    document.head.children[0]!.dispatchEvent(new Event('error'))
    await expect(promise).rejects.toThrow('failed')
    expect(document.head.children).toHaveLength(0)
  })
})

describe('session pages opened from a cached plugin catalog', () => {
  test('the initial session scope loads choices without visiting a chat page first', async () => {
    const scope = effectScope()
    const pageScope = ref<'instance' | 'session'>('session')
    let refreshes = 0
    const error = scope.run(() =>
      usePluginSessionRefresh(
        () => pageScope.value,
        async () => {
          refreshes++
        },
      ),
    )!
    expect(refreshes).toBe(1)
    expect(error.value).toBe('')
    scope.stop()
    pageScope.value = 'instance'
    await nextTick()
    pageScope.value = 'session'
    await nextTick()
    expect(refreshes).toBe(1)
  })

  test('a late session-load error cannot leak into a different plugin page', async () => {
    const scope = effectScope()
    const pageScope = ref<'instance' | 'session'>('session')
    const loading = deferred<void>()
    const error = scope.run(() =>
      usePluginSessionRefresh(
        () => pageScope.value,
        () => loading.promise,
      ),
    )!
    pageScope.value = 'instance'
    await nextTick()
    loading.reject(new Error('session lookup failed'))
    await nextTick()
    expect(error.value).toBe('')
    pageScope.value = 'session'
    await nextTick()
    await nextTick()
    expect(error.value).toBe('session lookup failed')
    scope.stop()
  })
})
