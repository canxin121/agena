import { describe, expect, test } from 'bun:test'
import { createContentSubscriptions } from '../src/lib/contentSubscriptions'
import type { ContentPage, ContentRef } from '../src/lib/content'

const epoch = '11111111-1111-4111-8111-111111111111'
const resource: ContentRef = { resource_id: 'reply', kind: 'text' }

function page(text: string, end: number, state: 'active' | 'complete' | 'interrupted'): ContentPage {
  const cursor = { epoch, sequence: end }
  return {
    resource: {
      resource_id: 'reply',
      kind: 'text',
      owner_session_id: 6,
      part_id: 8,
      state,
      cursor: { epoch, sequence: 10 },
      committed_cursor: { epoch, sequence: 10 },
      dropped_bytes: 0,
      retained_ranges: [],
      capture_error: null,
      total_bytes: 10,
    },
    chunks: [{ cursor, captured_at_ms: 0, payload: { type: 'text', text } }],
    next_cursor: cursor,
    has_more: end < 10,
    gap: false,
  }
}

function frames() {
  let id = 0
  const cancelled = new Set<number>()
  return {
    request(callback: FrameRequestCallback) {
      const next = ++id
      queueMicrotask(() => {
        if (!cancelled.has(next)) callback(0)
      })
      return next
    },
    cancel(frame: number) {
      cancelled.add(frame)
    },
  }
}

async function until(predicate: () => boolean, timeout = 1500) {
  const end = Date.now() + timeout
  while (!predicate() && Date.now() < end) await new Promise((resolve) => setTimeout(resolve, 10))
}

describe('content cursor convergence', () => {
  test('visibility resume replaces a half-open stream and retains its accepted cursor', async () => {
    const previous = Object.getOwnPropertyDescriptor(globalThis, 'document')
    const doc = Object.assign(new EventTarget(), { visibilityState: 'visible' })
    Object.defineProperty(globalThis, 'document', { configurable: true, value: doc })
    const cursors: Array<number | null> = []
    let text = ''
    const observe = createContentSubscriptions(async (_session, _id, after, signal, emit) => {
      cursors.push(after?.sequence ?? null)
      if (!after) {
        emit(page('hello', 5, 'active'))
        await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }))
      } else emit(page('world', 10, 'complete'))
    }, frames())
    const stop = observe('6', resource, (frame) => {
      text = frame.text
    })
    try {
      await until(() => text === 'hello')
      doc.visibilityState = 'hidden'
      doc.dispatchEvent(new Event('visibilitychange'))
      expect(cursors).toEqual([null])
      doc.visibilityState = 'visible'
      doc.dispatchEvent(new Event('visibilitychange'))
      await until(() => text === 'helloworld', 200)
      expect(cursors).toEqual([null, 5])
      expect(text).toBe('helloworld')
    } finally {
      stop()
      if (previous) Object.defineProperty(globalThis, 'document', previous)
      else Reflect.deleteProperty(globalThis, 'document')
    }
  })

  for (const state of ['complete', 'interrupted'] as const) {
    test(`resumes the unread tail of a ${state} source after disconnect`, async () => {
      const cursors: Array<number | null> = []
      let text = ''
      const observe = createContentSubscriptions(async (_session, _id, after, _signal, emit) => {
        cursors.push(after?.sequence ?? null)
        if (!after) emit(page('hello', 5, state))
        else emit(page('world', 10, state))
      }, frames())
      const stop = observe('6', resource, (frame) => {
        text = frame.text
      })
      try {
        await until(() => text === 'helloworld')
        expect(cursors).toEqual([null, 5])
        expect(text).toBe('helloworld')
      } finally {
        stop()
      }
    })
  }

  test('shares one source connection and does not reconnect a fully consumed terminal page', async () => {
    let calls = 0
    const observe = createContentSubscriptions(async (_session, _id, _after, _signal, emit) => {
      calls++
      emit(page('helloworld', 10, 'complete'))
    }, frames())
    const first: string[] = [],
      second: string[] = []
    const stopFirst = observe('6', resource, (frame) => first.push(frame.text))
    const stopSecond = observe('6', resource, (frame) => second.push(frame.text))
    try {
      await until(() => first.includes('helloworld') && second.includes('helloworld'))
      expect(calls).toBe(1)
      expect(second).toContain('helloworld')
    } finally {
      stopFirst()
      stopSecond()
    }
  })
})
