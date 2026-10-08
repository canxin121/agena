import { describe, expect, test } from 'bun:test'
import { createContentSubscriptions, type ContentFrame } from '../src/lib/contentSubscriptions'
import type { ContentPage, streamContent } from '../src/lib/content'

function page(sequence: number, state: 'active' | 'complete' = 'active'): ContentPage {
  const cursor = { epoch: 'one', sequence }
  return { resource: { resource_id: 'source', kind: 'log', state, owner_session_id: 1, part_id: 2, cursor, committed_cursor: cursor, retained_ranges: [{ first: 1, last: sequence }], dropped_bytes: 0, total_bytes: sequence },
    chunks: [{ cursor, captured_at_ms: 1000, payload: { type: 'log', stream: 'stdout', text: String(sequence) } }], next_cursor: cursor, has_more: false, gap: false }
}

function fixture() {
  const callbacks = new Map<number, FrameRequestCallback>()
  let frame = 0
  const calls: { signal: AbortSignal; cursor: unknown; page: (value: ContentPage) => void; resolve: () => void }[] = []
  const transport: typeof streamContent = async (_session, _resource, cursor, signal, onPage) => new Promise<void>((resolve) => {
    calls.push({ signal, cursor, page: onPage, resolve })
    signal.addEventListener('abort', () => resolve(), { once: true })
  })
  const observe = createContentSubscriptions(transport, { request: (callback) => { callbacks.set(++frame, callback); return frame }, cancel: (id) => { callbacks.delete(id) } })
  const flush = () => { const pending = [...callbacks.values()]; callbacks.clear(); pending.forEach((callback) => callback(0)) }
  return { calls, observe, flush }
}

describe('shared content subscriptions', () => {
  const resource = { resource_id: 'source', kind: 'log' as const }

  test('late mounts do not append a pending frame twice, and one widget cannot close another', async () => {
    const { calls, observe, flush } = fixture()
    const first: ContentFrame[] = []
    const second: ContentFrame[] = []
    const closeFirst = observe('1', resource, (frame) => first.push(frame))
    calls[0]!.page(page(1))
    const closeSecond = observe('1', resource, (frame) => second.push(frame))
    expect(calls.length).toBe(1)
    flush()
    expect(second.flatMap((frame) => frame.appended).map((chunk) => chunk.cursor.sequence)).toEqual([1])
    closeFirst()
    expect(calls[0]!.signal.aborted).toBe(false)
    calls[0]!.page(page(2, 'complete'))
    flush()
    expect(second.at(-1)!.buffer.resource!.state).toBe('complete')
    closeSecond()
    expect(calls[0]!.signal.aborted).toBe(true)
    await Promise.resolve()
  })

  test('a broken connection reconnects from its accepted cursor', async () => {
    const { calls, observe, flush } = fixture()
    const close = observe('1', resource, () => {})
    calls[0]!.page(page(1))
    flush()
    calls[0]!.resolve()
    await Bun.sleep(650)
    expect(calls.length).toBe(2)
    expect(calls[1]!.cursor).toEqual({ epoch: 'one', sequence: 1 })
    calls[1]!.page(page(2, 'complete'))
    flush()
    close()
  })

  test('session authorization contexts have independent interests', () => {
    const { calls, observe } = fixture()
    const closeFirst = observe('1', resource, () => {})
    const closeSecond = observe('2', resource, () => {})
    expect(calls.length).toBe(2)
    closeFirst()
    closeSecond()
  })

  test('an interrupted source is sealed and does not start a polling loop', async () => {
    const { calls, observe, flush } = fixture()
    const frames: ContentFrame[] = []
    const close = observe('1', resource, (frame) => frames.push(frame))
    const interrupted = page(1)
    interrupted.resource.state = 'interrupted'
    interrupted.resource.capture_error = 'capture incomplete'
    calls[0]!.page(interrupted)
    calls[0]!.resolve()
    flush()
    await Bun.sleep(650)
    expect(calls.length).toBe(1)
    expect(frames.at(-1)!.buffer.resource!.state).toBe('interrupted')
    expect(frames.at(-1)!.error).toBe('capture incomplete')
    close()
  })
})
