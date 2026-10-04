import { expect, test } from 'bun:test'
import { createRenderer, defineComponent, nextTick, ref } from 'vue'
import { useVisibleResource } from '../src/pages/chat/useVisibleResource'
import { mergeActivityLog, type ActivityLog } from '../src/types/activity'

test('session reads cancel on navigation, reject late results, coalesce clicks and pause hidden polling', async () => {
  const original = {
    document: globalThis.document,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    now: Date.now,
  }
  let now = 100_000
  let serial = 0
  const timers = new Map<number, { at: number; callback: () => void }>()
  const doc = Object.assign(new EventTarget(), { hidden: false })
  Object.assign(globalThis, {
    document: doc,
    setTimeout: (callback: () => void, delay: number) => {
      const id = ++serial
      timers.set(id, { at: now + delay, callback })
      return id
    },
    clearTimeout: (id: number) => timers.delete(id),
  })
  Date.now = () => now
  const settle = async () => {
    for (let i = 0; i < 6; i++) await nextTick()
  }
  const advance = async (ms: number) => {
    const end = now + ms
    while (true) {
      const next = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
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
  const renderer = createRenderer<Record<string, never>, Record<string, never>>({
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
  const key = ref('7/task_a')
  const calls: { key: string; signal: AbortSignal; resolve: (value: string) => void }[] = []
  let state!: ReturnType<typeof useVisibleResource<string>>
  const app = renderer.createApp(
    defineComponent({
      setup() {
        state = useVisibleResource({
          key,
          interval: () => 5000,
          load: (key, signal) => new Promise((resolve) => calls.push({ key, signal, resolve })),
        })
        return () => null
      },
    }),
  )
  try {
    app.mount({})
    expect(calls.length).toBe(1)
    void state.refresh()
    void state.refresh()
    expect(calls.length).toBe(1)
    key.value = '8/task_b'
    expect(calls[0]!.signal.aborted).toBe(true)
    calls[0]!.resolve('late A')
    await settle()
    expect(state.data.value).toBe(null)
    await advance(750)
    expect(calls.length).toBe(2)
    calls[1]!.resolve('B')
    await settle()
    expect(state.data.value).toBe('B')
    void state.refresh()
    void state.refresh()
    await advance(749)
    expect(calls.length).toBe(2)
    await advance(1)
    expect(calls.length).toBe(3)
    doc.hidden = true
    doc.dispatchEvent(new Event('visibilitychange'))
    expect(calls[2]!.signal.aborted).toBe(true)
    calls[2]!.resolve('hidden late')
    await advance(60_000)
    expect(state.data.value).toBe('B')
    expect(calls.length).toBe(3)
    doc.hidden = false
    doc.dispatchEvent(new Event('visibilitychange'))
    expect(calls.length).toBe(4)
    key.value = ''
    expect(calls[3]!.signal.aborted).toBe(true)
    expect(state.data.value).toBe(null)
    await advance(60_000)
    expect(calls.length).toBe(4)
  } finally {
    app.unmount()
    Object.assign(globalThis, {
      document: original.document,
      setTimeout: original.setTimeout,
      clearTimeout: original.clearTimeout,
    })
    Date.now = original.now
  }
})

test('activity logs deduplicate cursor overlap, bound memory, and isolate activities', () => {
  const page = (id: string, start: number, end: number): ActivityLog => ({
    activity_id: id,
    status: 'running',
    last_seq: end,
    has_more: false,
    dropped_lines: 0,
    lines: Array.from({ length: end - start + 1 }, (_, i) => ({
      seq: start + i,
      stream: 'stdout',
      text: String(start + i),
    })),
  })
  const logs = mergeActivityLog(page('a', 1, 190), page('a', 180, 310))
  expect(logs.lines.length).toBe(200)
  expect(logs.lines[0]!.seq).toBe(111)
  expect(logs.lines.at(-1)!.seq).toBe(310)
  expect(mergeActivityLog(logs, page('b', 1, 3)).lines.length).toBe(3)
  const huge = page('large', 1, 1)
  huge.lines[0]!.text = '文🙂'.repeat(80_000)
  const bounded = mergeActivityLog(null, huge)
  const tail = bounded.lines[0]!.text
  expect(new TextEncoder().encode(tail).length).toBeLessThanOrEqual(128 * 1024)
  expect(tail.startsWith('…')).toBe(true)
  expect(tail).not.toContain('\uFFFD')
})
