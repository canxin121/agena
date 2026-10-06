import { afterEach, beforeEach, expect, test } from 'bun:test'
import { computed, createRenderer, defineComponent, effectScope, ref } from 'vue'
import type * as Monaco from 'monaco-editor'
import { workspacePaneContextKey, type WorkspacePaneContext } from '../src/app/workspace/workspacePaneContext'
import { usePaneVisibility } from '../src/composables/usePaneVisibility'
import { useVisibleSubscription } from '../src/composables/useVisibleSubscription'
import { useMonacoFindSession } from '../src/components/editor/useMonacoFindSession'
import { createDisplayLineResolver } from '../src/components/editor/displayLineResolver'
import { createRevalidator } from '../src/lib/revalidation'
import { readBatchedRuntimeSettingSources, retireRuntimeSettingReads } from '../src/lib/runtimeSettingReads'
import { writeUiAuthTokenForBaseUrl, clearUiAuthTokenForBaseUrl } from '../src/lib/uiAuthToken'
import { probePreviewProxyResponse } from '../src/features/workspacePreview/api/previewProxyProbe'
import { ensureBrowserTestRuntime } from './testRuntime'

const originalFetch = globalThis.fetch
let browserGlobals: { window: Window & typeof globalThis; document: Document; localStorage: Storage }
const cleanups: Array<() => void> = []
const settle = async () => {
  for (let i = 0; i < 30; i++) await Promise.resolve()
}
const deferred = <T>() => {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
const bundle = (path: string) =>
  Object.fromEntries(
    ['effective', 'file', 'global', 'workspace'].map((source) => [
      source,
      { path, source: source === 'effective' ? 'effective' : 'file', value: path, config_found: true },
    ]),
  )
const batchResponse = (input: RequestInfo | URL) => {
  const url = new URL(String(input), 'http://agena.test')
  const paths: string[] = JSON.parse(url.searchParams.get('paths')!)
  return Response.json(Object.fromEntries(paths.map((path) => [path, bundle(path)])))
}
beforeEach(() => {
  browserGlobals = { window: globalThis.window, document: globalThis.document, localStorage: globalThis.localStorage }
  ensureBrowserTestRuntime()
  retireRuntimeSettingReads()
})
afterEach(() => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup()
  globalThis.fetch = originalFetch
  Object.assign(globalThis, browserGlobals)
})

test('many panes share one document listener, keep unfocused splits live, and release hidden subscriptions', () => {
  const events = new EventTarget()
  let added = 0,
    removed = 0
  const doc = Object.assign(events, {
    hidden: false,
    visibilityState: 'visible',
    addEventListener(type: string, listener: EventListener) {
      added++
      events.addEventListener(type, listener)
    },
    removeEventListener(type: string, listener: EventListener) {
      removed++
      events.removeEventListener(type, listener)
    },
  })
  // Keep the native methods: assigning the wrapper onto events would recurse.
  doc.addEventListener = (type, listener) => {
    added++
    EventTarget.prototype.addEventListener.call(events, type, listener)
  }
  doc.removeEventListener = (type, listener) => {
    removed++
    EventTarget.prototype.removeEventListener.call(events, type, listener)
  }
  globalThis.document = doc as unknown as Document
  const renderer = createRenderer<object, object>({
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
  let owners = 0
  const panes = Array.from({ length: 30 }, (_, index) => {
    const shown = ref(index < 2)
    const focused = ref(false)
    let visibility!: ReturnType<typeof usePaneVisibility>
    const app = renderer.createApp(
      defineComponent({
        setup() {
          visibility = usePaneVisibility()
          useVisibleSubscription(() => {
            owners++
            return () => {
              owners--
            }
          }, visibility)
          return () => null
        },
      }),
    )
    app.provide(workspacePaneContextKey, {
      isVisible: computed(() => shown.value),
      isFocused: computed(() => focused.value),
    } as WorkspacePaneContext)
    app.mount({})
    cleanups.push(() => app.unmount())
    return { app, shown, focused, visibility }
  })
  expect(added).toBe(1)
  expect(owners).toBe(2)
  panes[0]!.shown.value = false
  panes[2]!.shown.value = true
  expect(owners).toBe(2)
  panes[2]!.focused.value = true
  expect(owners).toBe(2)
  doc.hidden = true
  doc.visibilityState = 'hidden'
  events.dispatchEvent(new Event('visibilitychange'))
  expect(owners).toBe(0)
  expect(panes.every((pane) => !pane.visibility.value)).toBe(true)
  doc.hidden = false
  doc.visibilityState = 'visible'
  events.dispatchEvent(new Event('visibilitychange'))
  expect(owners).toBe(2)
  for (const cleanup of cleanups.splice(0)) cleanup()
  expect(removed).toBe(1)
  expect(owners).toBe(0)
})

test('a visible subscription has exactly one lease and unmount releases it even while hidden', () => {
  const scope = effectScope(),
    shown = ref(false)
  let retains = 0,
    releases = 0
  scope.run(() =>
    useVisibleSubscription(() => {
      retains++
      return () => {
        releases++
      }
    }, shown),
  )
  expect(retains).toBe(0)
  shown.value = true
  shown.value = true
  expect(retains).toBe(1)
  shown.value = false
  expect(releases).toBe(1)
  shown.value = true
  scope.stop()
  expect(retains).toBe(2)
  expect(releases).toBe(2)
  shown.value = true
  expect(retains).toBe(2)
})

test('hiding between refresh and its dispatch microtask reads nothing and preserves one read for resume', async () => {
  let shown = true,
    reads = 0
  const queue = createRevalidator(
    async () => {
      reads++
    },
    { enabled: () => shown, intervalMs: 0 },
  )
  cleanups.push(() => queue.dispose())
  const flight = queue.refresh()
  shown = false
  queue.pause()
  await flight
  expect(reads).toBe(0)
  shown = true
  queue.resume()
  await new Promise((resolve) => setTimeout(resolve, 10))
  expect(reads).toBe(1)
  queue.resume()
  await settle()
  expect(reads).toBe(1)
})

test('200 setting fields use four compact bounded requests and preserve all four source records', async () => {
  const requests: URL[] = []
  let active = 0,
    maximum = 0
  globalThis.fetch = (async (input) => {
    requests.push(new URL(String(input), 'http://agena.test'))
    maximum = Math.max(maximum, ++active)
    await Promise.resolve()
    active--
    return batchResponse(input)
  }) as typeof fetch
  const values = await Promise.all(
    Array.from({ length: 200 }, (_, i) => readBatchedRuntimeSettingSources(`ui.field_${i}`)),
  )
  expect(requests).toHaveLength(4)
  expect(maximum).toBeLessThanOrEqual(4)
  expect(values).toHaveLength(200)
  for (const [index, value] of values.entries()) {
    for (const source of ['effective', 'file', 'global', 'workspace'] as const)
      expect(value[source].value).toBe(`ui.field_${index}`)
  }
  for (const url of requests) {
    expect(JSON.parse(url.searchParams.get('paths')!).length).toBeLessThanOrEqual(64)
    expect(url.search.length).toBeLessThanOrEqual(6_001)
  }
  await readBatchedRuntimeSettingSources('ui.field_0')
  expect(requests).toHaveLength(5) // Finished settings are not kept in a stale indefinite cache.
})

test('same setting consumers share a batch and cancelling one does not abort the other panes', async () => {
  const pending = deferred<Response>(),
    controller = new AbortController()
  let requests = 0,
    transport!: AbortSignal,
    url!: RequestInfo | URL
  globalThis.fetch = ((input, init) => {
    requests++
    url = input
    transport = init!.signal!
    return pending.promise
  }) as typeof fetch
  const cancelled = readBatchedRuntimeSettingSources('ui.shared', controller.signal).catch(() => 'cancelled')
  const others = Array.from({ length: 25 }, () => readBatchedRuntimeSettingSources('ui.shared'))
  await settle()
  expect(requests).toBe(1)
  controller.abort()
  expect(await cancelled).toBe('cancelled')
  expect(transport.aborted).toBe(false)
  pending.resolve(batchResponse(url))
  const values = await Promise.all(others)
  expect(values.every((value) => value.effective.value === 'ui.shared')).toBe(true)
})

test('cancelled queued fields never dispatch and a batch transport stops when its last field leaves', async () => {
  let requests = 0,
    transport!: AbortSignal
  globalThis.fetch = ((_input, init) => {
    requests++
    transport = init!.signal!
    return new Promise((_resolve, reject) =>
      transport.addEventListener('abort', () => reject(transport.reason), { once: true }),
    )
  }) as typeof fetch
  const before = new AbortController()
  const queued = readBatchedRuntimeSettingSources('ui.never_read', before.signal).catch(() => 'cancelled')
  before.abort()
  expect(await queued).toBe('cancelled')
  await settle()
  expect(requests).toBe(0)
  const a = new AbortController(),
    b = new AbortController()
  const first = readBatchedRuntimeSettingSources('ui.one', a.signal).catch(() => 'cancelled')
  const second = readBatchedRuntimeSettingSources('ui.two', b.signal).catch(() => 'cancelled')
  await settle()
  expect(requests).toBe(1)
  a.abort()
  expect(transport.aborted).toBe(false)
  b.abort()
  expect(transport.aborted).toBe(true)
  expect(await Promise.all([first, second])).toEqual(['cancelled', 'cancelled'])
  await settle()
})

test('writes and auth changes prevent a new settings reader joining an earlier in-flight batch', async () => {
  const requests: Array<{ pending: ReturnType<typeof deferred<Response>>; input: RequestInfo | URL }> = []
  globalThis.fetch = ((input) => {
    const pending = deferred<Response>()
    requests.push({ pending, input })
    return pending.promise
  }) as typeof fetch
  const before = readBatchedRuntimeSettingSources('ui.epoch')
  await settle()
  retireRuntimeSettingReads()
  const during = readBatchedRuntimeSettingSources('ui.epoch')
  await settle()
  retireRuntimeSettingReads()
  const after = readBatchedRuntimeSettingSources('ui.epoch')
  await settle()
  const base = 'http://agena-performance-followup.test'
  writeUiAuthTokenForBaseUrl(base, 'test-only-token')
  cleanups.push(() => clearUiAuthTokenForBaseUrl(base))
  const authChanged = readBatchedRuntimeSettingSources('ui.epoch')
  await settle()
  expect(requests).toHaveLength(4)
  for (const { input, pending } of requests) pending.resolve(batchResponse(input))
  await Promise.all([before, during, after, authChanged])
})

test('long quoted Unicode paths are split by encoded URL size as well as count', async () => {
  const requests: URL[] = []
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'http://agena.test')
    requests.push(url)
    expect(url.search.length).toBeLessThanOrEqual(6_001)
    expect(new TextEncoder().encode(url.searchParams.get('paths')!).length).toBeLessThanOrEqual(16_384)
    return batchResponse(input)
  }) as typeof fetch
  const paths = Array.from({ length: 130 }, (_, i) => `ui."${'配置'.repeat(60)}_${i}"`)
  const values = await Promise.all(paths.map((path) => readBatchedRuntimeSettingSources(path)))
  expect(requests.length).toBeGreaterThan(3)
  expect(values.map((value) => value.effective.value)).toEqual(paths)
})

test('the advanced root editor shares its four legacy document reads among concurrent consumers', async () => {
  let requests = 0
  globalThis.fetch = (async () => {
    requests++
    return Response.json({ value: { root: true } })
  }) as typeof fetch
  const values = await Promise.all(Array.from({ length: 15 }, () => readBatchedRuntimeSettingSources('')))
  expect(requests).toBe(4)
  expect(values.every((value) => value.workspace.value && typeof value.workspace.value === 'object')).toBe(true)
})

test('Monaco find skips unchanged full scans and full decorations when only the active match moves', () => {
  let scans = 0,
    version = 1,
    selection: Monaco.Selection | null = null
  const collections: Array<{ updates: unknown[][] }> = []
  const query = ref('a'),
    caseSensitive = ref(false),
    regex = ref(false),
    wholeWord = ref(false)
  const model = {
    getVersionId: () => version,
    findMatches: () => {
      scans++
      return [1, 3, 5].map((column) => ({
        range: {
          startLineNumber: 1,
          endLineNumber: 1,
          startColumn: column,
          endColumn: column + 1,
        },
      }))
    },
  }
  let currentModel = model
  const editor = {
    getModel: () => currentModel,
    getSelection: () => selection,
    createDecorationsCollection() {
      const collection = {
        updates: [] as unknown[][],
        set(items: unknown[]) {
          this.updates.push(items)
        },
      }
      collections.push(collection)
      return collection
    },
    setSelection(range: Monaco.IRange) {
      selection = {
        ...range,
        getStartPosition: () => ({ lineNumber: 1, column: range.startColumn }),
      } as Monaco.Selection
    },
    revealRangeInCenterIfOutsideViewport() {},
    focus() {},
  } as unknown as Monaco.editor.IStandaloneCodeEditor
  const find = useMonacoFindSession(() => editor, { query, caseSensitive, regex, wholeWord })
  find.refresh()
  for (let i = 0; i < 40; i++) find.refresh()
  expect(scans).toBe(1)
  expect(collections[0]!.updates).toHaveLength(1)
  expect(collections[1]!.updates).toHaveLength(1)
  find.move(1)
  find.move(1)
  expect(find.currentMatch.value).toBe(3)
  expect(scans).toBe(1)
  expect(collections[0]!.updates).toHaveLength(1)
  expect(collections[1]!.updates).toHaveLength(3)
  version++
  find.refresh()
  expect(scans).toBe(2)
  query.value = 'b'
  find.refresh()
  caseSensitive.value = true
  find.refresh()
  regex.value = true
  find.refresh()
  wholeWord.value = true
  find.refresh()
  expect(scans).toBe(6)
  currentModel = { ...model }
  find.refresh()
  expect(scans).toBe(7)
  find.dispose()
  expect(collections.every((collection) => collection.updates.at(-1)!.length === 0)).toBe(true)
})

test('indexed diff line lookup preserves sparse and non-monotonic mappings and handles large files', () => {
  const maps = [[null, 5, null, 20, 10, 30, null, 25], [8, null, 2], []]
  for (const map of maps) {
    const resolve = createDisplayLineResolver(20, map, 4)
    for (let target = 1; target < 40; target++) {
      const first = map.findIndex((line) => line !== null && line >= target)
      let last = -1
      for (const [index, value] of map.entries()) if (value !== null) last = index
      const expected = first >= 0 ? first + 1 : last >= 0 ? last + 1 : Math.max(1, Math.min(20, target - 3))
      expect(resolve(target)).toBe(expected)
    }
  }
  const large = Array.from({ length: 100_000 }, (_, i) => (i % 3 === 0 ? null : 1_000_000 + i))
  const resolve = createDisplayLineResolver(large.length, large, null)
  for (let i = 0; i < 4_000; i++) expect(resolve(1_000_001 + 3 * i)).toBe(2 + 3 * i)
  expect(resolve(Number.MAX_SAFE_INTEGER)).toBe(99_999)
  expect(createDisplayLineResolver(10, null, 100)(105)).toBe(6)
})

test('a successful preview probe uses HEAD and cancels the response body', async () => {
  const methods: string[] = []
  let cancelled = 0
  globalThis.fetch = (async (_url, init) => {
    methods.push(String(init?.method))
    return new Response(
      new ReadableStream({
        cancel() {
          cancelled++
        },
      }),
    )
  }) as typeof fetch
  const result = await probePreviewProxyResponse('/preview/test', new AbortController().signal)
  expect(methods).toEqual(['HEAD'])
  expect(result.ok).toBe(true)
  expect(cancelled).toBe(1)
})

test('preview servers without HEAD fall back to GET, and error bodies have a fixed read budget', async () => {
  const methods: string[] = []
  let cancelled = 0
  globalThis.fetch = (async (_url, init) => {
    methods.push(String(init?.method))
    if (init?.method === 'HEAD') return new Response(null, { status: 405 })
    return new Response(
      new ReadableStream({
        pull(controller) {
          controller.enqueue(new TextEncoder().encode('x'.repeat(50_000)))
        },
        cancel() {
          cancelled++
        },
      }),
      { status: 502, headers: { 'content-type': 'text/plain' } },
    )
  }) as typeof fetch
  const result = await probePreviewProxyResponse('/preview/test', new AbortController().signal)
  expect(methods).toEqual(['HEAD', 'GET'])
  expect(result.ok).toBe(false)
  expect(result.body).toHaveLength(16_384)
  expect(cancelled).toBe(1)
})

test('hiding a preview aborts its probe transport', async () => {
  const controller = new AbortController()
  let transport!: AbortSignal
  globalThis.fetch = ((_url, init) => {
    transport = init!.signal!
    return new Promise((_resolve, reject) =>
      transport.addEventListener('abort', () => reject(transport.reason), { once: true }),
    )
  }) as typeof fetch
  const result = probePreviewProxyResponse('/preview/test', controller.signal).catch((reason) => reason.name)
  await settle()
  controller.abort()
  expect(transport.aborted).toBe(true)
  expect(await result).toBe('AbortError')
})
