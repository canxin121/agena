import { expect, test } from 'bun:test'
import { createRenderer, defineComponent, nextTick, ref } from 'vue'
import { useSessionPlan } from '../src/pages/chat/useSessionPlan'
import type { JsonValue } from '../src/types/json'

// Exercise real Vue mount/disposal and scheduled reads without a browser or
// wall-clock waits. Transport completions remain under the test's control.
test('inline plans coalesce refreshes, pause hidden polling, and retain only their own session state', async () => {
  const original = {
    document: globalThis.document,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    setInterval: globalThis.setInterval,
    clearInterval: globalThis.clearInterval,
    now: Date.now,
  }
  let now = 1_000_000
  let serial = 0
  const timers = new Map<number, { at: number; interval: number; callback: () => void }>()
  const schedule = (callback: () => void, delay = 0, interval = 0) => {
    const id = ++serial
    timers.set(id, { at: now + delay, interval, callback })
    return id
  }
  const doc = Object.assign(new EventTarget(), { visibilityState: 'visible' })
  Object.assign(globalThis, {
    document: doc,
    setTimeout: (callback: () => void, delay: number) => schedule(callback, delay),
    setInterval: (callback: () => void, delay: number) => schedule(callback, delay, delay),
    clearTimeout: (id: number) => timers.delete(id),
    clearInterval: (id: number) => timers.delete(id),
  })
  Date.now = () => now
  const settle = async () => {
    for (let i = 0; i < 6; i++) await nextTick()
  }
  const advance = async (ms: number) => {
    const end = now + ms
    while (true) {
      const next = [...timers.entries()].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
      if (!next) break
      const [id, timer] = next
      now = timer.at
      if (timer.interval) timer.at += timer.interval
      else timers.delete(id)
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
  const session = ref<string | null>('7')
  const busy = ref(false)
  const change = ref(0)
  const calls: Array<{
    session: string | null
    signal: AbortSignal
    resolve: (response: Record<string, JsonValue>) => void
  }> = []
  let state!: ReturnType<typeof useSessionPlan>
  const app = renderer.createApp(
    defineComponent({
      setup() {
        state = useSessionPlan(
          () => session.value,
          () => busy.value,
          () => change.value,
          (session, _tool, _input, signal) => new Promise((resolve) => calls.push({ session, signal, resolve })),
        )
        return () => null
      },
    }),
  )
  const response = (title: string) => ({
    output_text: `Revision: v1\n# ${title}`,
    payload: { plan: { title, autorun: false } },
  })
  try {
    app.mount({})
    expect(calls.length).toBe(1)
    state.expanded.value = true
    calls[0]!.resolve(response('First'))
    await settle()
    await advance(1000)
    expect(calls.length).toBe(1) // opening an already fresh plan does not re-read
    state.expanded.value = false
    await settle()
    await advance(28_000)
    expect(calls.length).toBe(1)
    await advance(1200)
    expect(calls.length).toBe(2) // an idle collapsed plan backs off to 30 seconds

    for (let i = 0; i < 20; i++) change.value++
    await settle()
    await advance(2000)
    expect(calls.length).toBe(2) // one request in flight, one pending invalidation
    calls[1]!.resolve(response('Updated'))
    await settle()
    await advance(750)
    expect(calls.length).toBe(3)
    calls[2]!.resolve(response('Fresh'))
    await settle()
    state.expanded.value = true
    doc.visibilityState = 'hidden'
    await advance(61_000)
    expect(calls.length).toBe(3)
    doc.visibilityState = 'visible'
    doc.dispatchEvent(new Event('visibilitychange'))
    await advance(180)
    expect(calls.length).toBe(4)

    session.value = '9'
    expect(calls[3]!.signal.aborted).toBe(true)
    expect(state.viewer.snapshot.value).toBeNull()
    expect(state.expanded.value).toBe(false)
    calls[3]!.resolve(response('Late first session'))
    calls[4]!.resolve(response('Second session'))
    await settle()
    expect(state.viewer.snapshot.value?.title).toBe('Second session')
    session.value = '7'
    expect(state.expanded.value).toBe(true)
    expect(state.viewer.markdown.value).toBe('')
    app.unmount()
    expect(calls[5]!.signal.aborted).toBe(true)
    calls[5]!.resolve(response('Disposed'))
    await settle()
    expect(state.viewer.markdown.value).toBe('')
    expect(timers.size).toBe(0)
  } finally {
    app.unmount()
    Date.now = original.now
    Object.assign(globalThis, {
      document: original.document,
      setTimeout: original.setTimeout,
      clearTimeout: original.clearTimeout,
      setInterval: original.setInterval,
      clearInterval: original.clearInterval,
    })
  }
})
