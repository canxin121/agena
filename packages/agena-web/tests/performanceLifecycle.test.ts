import { describe, expect, test } from 'bun:test'
import { createRenderer, defineComponent, effectScope, nextTick, ref, shallowRef } from 'vue'
import { acquireModel } from '../src/lib/monaco-editor/utils'
import type { MonacoEditor } from '../src/lib/monaco-editor/types'
import { TranscriptCacheIndex } from '../src/stores/chat/transcriptCache'
import { composerDomSelection } from '../src/pages/chat/composerDomSelection'
import { createBlobUrlRegistry } from '../src/lib/blobUrlRegistry'
import { createAttachmentIngestor, type StagedAttachment } from '../src/pages/chat/attachmentIngestion'
import { useNearViewport } from '../src/composables/useNearViewport'
import { usePinnedScroll } from '../src/composables/chat/usePinnedScroll'
import { transcriptSearchHighlightRanges } from '../src/pages/chat/transcriptSearchHighlights'
import type { TranscriptSearchMatch } from '../src/pages/chat/transcriptSearch'
import { loadExpandedTree } from '../src/features/sessions/model/expandedTree'
import { mergeActivityLog, type ActivityLog } from '../src/types/activity'
import {
  MAX_PROMPT_HISTORY_CHARACTERS,
  persistPromptHistory,
  useComposerPromptHistory,
} from '../src/pages/chat/composerPromptHistory'

describe('bounded resource lifetimes', () => {
  test('hidden transcripts pause scrolling and restore reading anchors or bottom following after DOM remount', async () => {
    const originalWindow = globalThis.window
    const originalObserver = globalThis.ResizeObserver
    let sequence = 0
    const frames = new Map<number, FrameRequestCallback>()
    globalThis.window = {
      ...(originalWindow ?? {}),
      requestAnimationFrame: (callback: FrameRequestCallback) => {
        const id = ++sequence
        frames.set(id, callback)
        return id
      },
      cancelAnimationFrame: (id: number) => {
        frames.delete(id)
      },
    } as Window & typeof globalThis
    const observers: Array<{ disconnected: boolean; callback: ResizeObserverCallback }> = []
    globalThis.ResizeObserver = class {
      disconnected = false
      constructor(public callback: ResizeObserverCallback) {
        observers.push(this)
      }
      observe() {}
      disconnect() {
        this.disconnected = true
      }
    } as unknown as typeof ResizeObserver
    const visible = ref(true)
    const session = ref('one')
    let olderReads = 0
    let state!: ReturnType<typeof usePinnedScroll>
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
    const app = renderer.createApp(
      defineComponent({
        setup() {
          state = usePinnedScroll({
            isVisible: () => visible.value,
            sessionId: () => session.value,
            canLoadOlder: () => true,
            loadOlder: async () => {
              olderReads++
              return false
            },
          })
          return () => null
        },
      }),
    )
    let inserted = 0
    let rows: HTMLElement[] = []
    const scroller = {
      scrollTop: 300,
      scrollHeight: 1800,
      clientHeight: 600,
      getBoundingClientRect: () => ({ top: 40 }),
      querySelectorAll: () => rows,
      contains: (element: HTMLElement) => rows.includes(element),
    }
    const leaf = () =>
      ({
        dataset: { transcriptKey: 'part:2' },
        querySelector: () => null,
        getBoundingClientRect: () => ({
          top: 390 + inserted - scroller.scrollTop,
          bottom: 590 + inserted - scroller.scrollTop,
        }),
      }) as unknown as HTMLElement
    const bottom = {
      scrollIntoView() {
        scroller.scrollTop = scroller.scrollHeight - scroller.clientHeight
      },
    }
    const settle = async () => {
      for (let i = 0; i < 4; i++) await nextTick()
    }
    const paint = async () => {
      const pending = [...frames.values()]
      frames.clear()
      for (const callback of pending) callback(performance.now())
      await settle()
    }
    try {
      app.mount({})
      rows = [leaf()]
      state.scrollEl.value = scroller as unknown as HTMLDivElement
      state.contentEl.value = {} as HTMLDivElement
      state.bottomEl.value = bottom as HTMLDivElement
      await settle()
      state.handleScroll()
      expect(state.isAtBottom.value).toBe(false)
      visible.value = false
      expect(observers.at(-1)!.disconnected).toBe(true)
      rows = []
      scroller.scrollHeight = 600
      scroller.scrollTop = 0
      state.handleScroll()
      state.handleWheel({ deltaY: -100 } as WheelEvent)
      state.scheduleScrollToBottom()
      expect(olderReads).toBe(0)
      expect(frames.size).toBe(0)
      expect(state.isAtBottom.value).toBe(false)

      inserted = 70
      rows = [leaf()] // a new element with the same durable row identity
      scroller.scrollHeight = 1900
      visible.value = true
      await settle()
      expect(scroller.scrollTop).toBe(370)
      expect(state.isAtBottom.value).toBe(false)
      expect(observers.at(-1)!.disconnected).toBe(false)

      scroller.scrollTop = 1300
      state.handleScroll()
      expect(state.isAtBottom.value).toBe(true)
      visible.value = false
      rows = []
      scroller.scrollTop = 0
      scroller.scrollHeight = 600
      visible.value = true
      rows = [leaf()]
      scroller.scrollHeight = 2400
      await settle()
      expect(scroller.scrollTop).toBe(1800)
      expect(state.isAtBottom.value).toBe(true)

      visible.value = false
      session.value = 'two'
      state.requestInitialScroll('two')
      scroller.scrollTop = 0
      scroller.scrollHeight = 2000
      visible.value = true
      await settle()
      await paint()
      expect(scroller.scrollTop).toBe(1400)
      expect(state.pendingInitialScrollSessionId.value).toBeNull()
      app.unmount()
      expect(observers.every((observer) => observer.disconnected)).toBe(true)
    } finally {
      app.unmount()
      globalThis.window = originalWindow
      globalThis.ResizeObserver = originalObserver
    }
  })
  test('expanded trees preserve sibling order, bound request fanout, and handle very deep paths', async () => {
    let inFlight = 0
    let maximum = 0
    const rows = await loadExpandedTree(
      [0, 1],
      async (id, depth, root) => {
        maximum = Math.max(maximum, ++inFlight)
        await new Promise((resolve) => setTimeout(resolve, id === 0 ? 2 : 0))
        inFlight--
        return { row: { id, depth, root }, children: depth === 0 ? [id * 10 + 2, id * 10 + 3, id * 10 + 4] : [] }
      },
      undefined,
      2,
    )
    expect(maximum).toBe(2)
    expect(rows.map((row) => row.id)).toEqual([0, 2, 3, 4, 1, 12, 13, 14])
    expect(rows[4]).toEqual({ id: 1, depth: 0, root: 1 })
    const deep = await loadExpandedTree([0], async (id, depth) => ({ row: depth, children: id < 4000 ? [id + 1] : [] }))
    expect(deep).toHaveLength(4001)
    expect(deep.at(-1)).toBe(4000)
    const controller = new AbortController()
    let started = 0
    await expect(
      loadExpandedTree(
        [0],
        async () => {
          started++
          controller.abort()
          return { row: 0, children: [1] }
        },
        controller.signal,
      ),
    ).rejects.toThrow()
    expect(started).toBe(1)
  })
  test('named editor models survive another pane releasing them and are disposed at the last release', () => {
    const models = new Map<string, { isDisposed: () => boolean; dispose: () => void }>()
    let disposed = 0
    const monaco = {
      Uri: { parse: (path: string) => path },
      editor: {
        getModel: (path: string) => models.get(path) ?? null,
        createModel: (_value: string, _language: string, path: string) => {
          let released = false
          const model = {
            isDisposed: () => released,
            dispose: () => {
              released = true
              disposed++
              models.delete(path)
            },
          }
          models.set(path, model)
          return model
        },
      },
    } as unknown as MonacoEditor
    const first = acquireModel(monaco, 'a', 'text', 'file:a')
    const second = acquireModel(monaco, 'a', 'text', 'file:a')
    first.release()
    first.release()
    expect(disposed).toBe(0)
    expect(second.model).toBe(first.model)
    second.release()
    expect(disposed).toBe(1)
    for (let i = 0; i < 1000; i++) acquireModel(monaco, 'body', 'text', `file:${i}`).release()
    expect(models.size).toBe(0)
    const external = {
      isDisposed: () => false,
      dispose: () => {
        throw new Error('external model disposed')
      },
    }
    models.set('external', external)
    acquireModel(monaco, '', '', 'external').release()
    expect(models.get('external')).toBe(external)
  })

  test('transcript eviction protects visible sessions and removes shared-part reverse memberships', () => {
    const cache = new TranscriptCacheIndex()
    cache.set('one', [
      ['shared', 'x'.repeat(64)],
      ['removed', 'old'],
    ])
    cache.set('two', [['shared', 'body']])
    cache.set('three', [['tail', 'body']])
    expect([...cache.members('shared')]).toEqual(['one', 'two'])
    cache.touch('one')
    expect(cache.evictions(new Set(['three']), 2)).toEqual(['two'])
    expect(cache.evictions(new Set(['one', 'three']), 1, 1)).toEqual(['two'])
    cache.removePart('one', 'removed')
    expect([...cache.members('removed')]).toEqual([])
    cache.remove('one')
    expect([...cache.members('shared')]).toEqual(['two'])
    cache.set('two', [['replacement', 'new']])
    expect([...cache.members('shared')]).toEqual([])
    cache.clear()
    expect(cache.evictions(new Set(), 0, 0)).toEqual([])
  })

  test('blob budgets are shared across panes and release only unreferenced URLs', () => {
    let sequence = 0
    const revoked: string[] = []
    const options = { maxBytes: 6, create: () => `blob:test-${++sequence}`, revoke: (url: string) => revoked.push(url) }
    const first = createBlobUrlRegistry(options)
    const second = createBlobUrlRegistry(options)
    try {
      const draft = first.create(new Blob(['12345']))
      expect(second.available()).toBe(1)
      expect(() => second.create(new Blob(['12']))).toThrow(/budget/)
      first.releaseUnreferenced(new Set([draft]))
      expect(revoked).toEqual([])
      first.releaseUnreferenced(new Set())
      first.release(draft)
      expect(revoked).toEqual([draft])
      second.create(new Blob(['123456']))
      expect(first.available()).toBe(0)
    } finally {
      first.dispose()
      second.dispose()
    }
    expect(createBlobUrlRegistry(options).available()).toBe(6)
  })

  test('binary attachment staging deduplicates contents and frees cancelled/rejected URLs', async () => {
    let current: StagedAttachment[] = []
    let sequence = 0
    const released: string[] = []
    const ingestion = createAttachmentIngestor({
      get: () => current,
      set: (files) => {
        current = files
      },
      read: async () => `blob:attachment-${++sequence}`,
      release: (url) => released.push(url),
      onError: () => {},
      onBusy: () => {},
    })
    await ingestion.stage([new File(['AAA'], 'same.png'), new File(['BBB'], 'same.png'), new File(['AAA'], 'same.png')])
    expect(current).toHaveLength(2)
    expect(current.every((file) => file.blob instanceof Blob)).toBe(true)
    expect(released).toEqual(['blob:attachment-3'])
    let finish!: (value: string) => void
    const late = createAttachmentIngestor({
      get: () => [],
      set: (files) => {
        if (files.length) throw new Error('cancelled read was published')
      },
      read: () =>
        new Promise((resolve) => {
          finish = resolve
        }),
      release: (url) => released.push(url),
      onError: () => {},
      onBusy: () => {},
    })
    const pending = late.stage([new File(['a'], 'late.txt')])
    await Promise.resolve()
    late.clear()
    finish('blob:cancelled')
    await pending
    expect(released.at(-1)).toBe('blob:cancelled')
  })

  test('a duplicate removed during hashing does not discard the new attachment', async () => {
    const original = new File(['same contents'], 'same.png')
    const replacement = new File(['same contents'], 'same.png')
    let current: StagedAttachment[] = [
      {
        id: 'original',
        filename: 'same.png',
        size: original.size,
        mime: '',
        url: 'blob:original',
        blob: original,
        state: 'ready',
      },
    ]
    const originalRead = original.arrayBuffer.bind(original)
    original.arrayBuffer = () => {
      current = []
      return originalRead()
    }
    const ingestion = createAttachmentIngestor({
      get: () => current,
      set: (files) => {
        current = files
      },
      read: async () => 'blob:replacement',
      onError() {},
      onBusy() {},
    })
    await ingestion.stage([replacement])
    expect(current).toHaveLength(1)
    expect(current[0]!.blob).toBe(replacement)
  })

  test('decorative search ranges preserve cross-node offsets and always include the active occurrence', () => {
    const first = {} as Text
    const second = {} as Text
    const segments = [
      { node: first, nodeStart: 3, nodeEnd: 8, start: 0, end: 5 },
      { node: second, nodeStart: 0, nodeEnd: 5, start: 6, end: 11 },
    ]
    const match = (start: number, end: number): TranscriptSearchMatch => ({
      key: 'part:1',
      textStart: start,
      textEnd: end,
      globalStart: start,
      globalEnd: end,
    })
    const matches = [match(1, 2), match(4, 8), match(9, 11)]
    expect(transcriptSearchHighlightRanges(segments, matches, matches[1]!, 1)).toEqual([
      { node: first, start: 4, end: 5, active: false },
      { node: first, start: 7, end: 8, active: true },
      { node: second, start: 0, end: 2, active: true },
    ])
    expect(transcriptSearchHighlightRanges(segments, matches, matches[2]!, 0)).toEqual([
      { node: second, start: 3, end: 5, active: true },
    ])
    expect(transcriptSearchHighlightRanges(segments, matches, null, 0)).toEqual([])
  })

  test('all rich bodies share one observer and release it when their scopes close', async () => {
    const original = globalThis.IntersectionObserver
    let creations = 0
    let disconnects = 0
    let callback!: IntersectionObserverCallback
    const observed = new Set<Element>()
    globalThis.IntersectionObserver = class {
      constructor(next: IntersectionObserverCallback) {
        creations++
        callback = next
      }
      observe(element: Element) {
        observed.add(element)
      }
      unobserve(element: Element) {
        observed.delete(element)
      }
      disconnect() {
        disconnects++
        observed.clear()
      }
    } as unknown as typeof IntersectionObserver
    const first = effectScope()
    const second = effectScope()
    const a = {} as HTMLElement
    const b = {} as HTMLElement
    try {
      const elementA = shallowRef<HTMLElement | null>(a)
      const nearA = first.run(() => useNearViewport(elementA))!
      const nearB = second.run(() => useNearViewport(shallowRef(b)))!
      await nextTick()
      expect(creations).toBe(1)
      expect(observed.size).toBe(2)
      expect(nearA.value).toBe(false)
      callback([{ target: a, isIntersecting: true }] as IntersectionObserverEntry[], {} as IntersectionObserver)
      expect(nearA.value).toBe(true)
      expect(nearB.value).toBe(false)
      elementA.value = {} as HTMLElement
      await nextTick()
      expect(nearA.value).toBe(false)
      expect(observed.has(a)).toBe(false)
      expect(observed.has(elementA.value)).toBe(true)
      // Disposal before the post-flush ref watcher runs must still release
      // the previous target, rather than the ref's new (unobserved) value.
      elementA.value = null
      first.stop()
      expect(disconnects).toBe(0)
      expect(observed.size).toBe(1)
      second.stop()
      expect(disconnects).toBe(1)
    } finally {
      first.stop()
      second.stop()
      globalThis.IntersectionObserver = original
    }
  })
})

test('composer offsets traverse wrappers, line breaks, Unicode and attachment chips once', () => {
  const text = (value: string) => ({ nodeType: 3, textContent: value, childNodes: [] }) as unknown as Node
  const element = (tagName: string, children: Node[] = [], dataset = {}) =>
    ({ nodeType: 1, tagName, childNodes: children, dataset }) as unknown as Node
  const first = text('A😀')
  const chip = element('SPAN', [text('very long chip label')], { attachmentId: 'file' })
  const last = text('中文')
  const wrapper = element('SPAN', [last])
  const root = element('DIV', [first, element('BR'), chip, wrapper])
  const range = (startContainer: Node, startOffset: number, endContainer: Node, endOffset: number) =>
    ({ startContainer, startOffset, endContainer, endOffset }) as Range
  expect(composerDomSelection(root, range(first, 1, last, 2))).toEqual([1, 7])
  expect(composerDomSelection(root, range(root, 1, root, 3))).toEqual([3, 5])
  expect(composerDomSelection(root, range(wrapper, 0, wrapper, 1))).toEqual([5, 7])
  expect(composerDomSelection(root, range(last, 1, last, 1))).toEqual([6, 6])
  expect(composerDomSelection(root, range(root, 0, root, 4))).toEqual([0, 7])
})

test('activity logs preserve immutable line identities and UTF-8 tails within the byte budget', () => {
  const snapshot = (lines: ActivityLog['lines']): ActivityLog => ({
    activity_id: 'task_a',
    status: 'running',
    lines,
    last_seq: lines.at(-1)?.seq ?? 0,
    has_more: false,
    dropped_lines: 0,
  })
  const line = { seq: 1, stream: 'message', text: 'aaaa' }
  const before = mergeActivityLog(null, snapshot([line]))
  const unchanged = mergeActivityLog(before, snapshot([]))
  expect(unchanged.lines[0]).toBe(line)
  const body = '中😀'.repeat(300_000) + 'END'
  const result = mergeActivityLog(unchanged, snapshot([{ seq: 2, stream: 'message', text: body }]))
  expect(result.lines.at(-1)!.text.endsWith('END')).toBe(true)
  expect(result.lines.at(-1)!.text.startsWith('…')).toBe(true)
  expect(result.lines.at(-1)!.text).not.toContain('�')
  expect(
    result.lines.reduce((bytes, item) => bytes + new TextEncoder().encode(item.text).length, 0),
  ).toBeLessThanOrEqual(128 * 1024)
  expect(
    mergeActivityLog(result, snapshot([{ seq: 2, stream: 'message', text: 'replacement' }])).lines.at(-1)!.text,
  ).toBe('replacement')
})

test('prompt history is shared across panes without reparsing unchanged storage and obeys the character budget', () => {
  const data = new Map<string, string>()
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => data.set(key, value),
  } as Storage
  const first = useComposerPromptHistory({ storage })
  const second = useComposerPromptHistory({ storage })
  first.record('one')
  expect(second.entries).toBe(first.entries)
  const entries = first.entries.value
  second.reload()
  expect(second.entries.value).toBe(entries)
  persistPromptHistory(['x'.repeat(MAX_PROMPT_HISTORY_CHARACTERS), 'overflow'], storage)
  first.reload()
  expect(first.entries.value.reduce((count, item) => count + item.length, 0)).toBe(MAX_PROMPT_HISTORY_CHARACTERS)
  expect(first.entries.value).toHaveLength(1)
})
