import assert from 'node:assert/strict'
import test from 'node:test'

import { connectSse } from '../src/lib/sse.ts'

// Use actual Response/ReadableStream objects so decoding sees the same byte
// boundaries as a network response. EOF flushes the queue before onError runs.
async function consumeChunks(chunks) {
  const originalFetch = globalThis.fetch
  const originalWindow = globalThis.window
  if (!globalThis.window) globalThis.window = globalThis
  globalThis.fetch = async () =>
    new Response(
      new ReadableStream({
        start(controller) {
          for (const chunk of chunks) controller.enqueue(chunk)
          controller.close()
        },
      }),
      { headers: { 'content-type': 'text/event-stream' } },
    )

  const events = []
  let finish
  const done = new Promise((resolve) => {
    finish = resolve
  })
  const client = connectSse({
    endpoint: '/fake',
    autoReconnect: false,
    onEvent: (event) => events.push(event),
    onError: () => finish(),
  })
  try {
    await done
    return events
  } finally {
    client.close()
    globalThis.fetch = originalFetch
    if (originalWindow === undefined) delete globalThis.window
  }
}

function notification(change) {
  return { kind: 'session_changed', data: { subscription: {}, change: { session_id: 1, ...change } } }
}

function consumeNotifications(notifications) {
  const stream = notifications.map((value, index) => `id: ${index + 1}\ndata: ${JSON.stringify(value)}\n\n`).join('')
  return consumeChunks([new TextEncoder().encode(stream)])
}

test('SSE preserves multiline JSON, UTF-8, and cursors across split CRLF bytes', async () => {
  const stream =
    'id: 1\r\ndata: {"kind":"runtime_signal",\r\ndata: "data":{"signal":{"kind":"plugin","payload":{"label":"中文"}}}}\r\n\r\n'
  const bytes = new TextEncoder().encode(stream)
  const events = await consumeChunks(Array.from(bytes, (byte) => new Uint8Array([byte])))
  assert.equal(events.length, 1)
  assert.equal(events[0].lastEventId, '1')
  assert.equal(events[0].properties.payload.label, '中文')
})

test('SSE metadata notifications retain both enabled and cleared favorite/pinned flags', async () => {
  const events = await consumeNotifications([
    notification({
      kind: 'session_meta_updated',
      title: 'First',
      version: 1,
      favorite: true,
      pinned: true,
      updated_at_ms: 1,
    }),
    notification({
      kind: 'session_meta_updated',
      session_id: 2,
      title: 'Second',
      version: 2,
      favorite: false,
      pinned: false,
      updated_at_ms: 2,
    }),
  ])
  assert.equal(events.length, 2)
  assert.equal(events[0].properties.favorite, true)
  assert.equal(events[0].properties.pinned, true)
  assert.equal(events[1].properties.favorite, false)
  assert.equal(events[1].properties.pinned, false)
})

test('SSE coalesces complete part snapshots without retaining removed optional fields', async () => {
  const latest = { part_id: 10, kind: 'tool_call', state: 'running', content: {}, revision: 2 }
  const events = await consumeNotifications([
    notification({
      kind: 'part_updated',
      part: { ...latest, revision: 1, summary: 'Old summary', presentation: { title: 'Old tool' } },
    }),
    notification({ kind: 'part_updated', part: latest }),
  ])
  assert.equal(events.length, 1)
  assert.deepEqual(events[0].properties.part, latest)
  assert.equal(events[0].lastEventId, '2')
})

test('SSE preserves independent runtime signals, including array and scalar payloads', async () => {
  const signals = [
    { kind: 'activity', session_id: 1, payload: { count: 1 } },
    { kind: 'plugin', session_id: 1, payload: ['loaded'] },
    { kind: 'plugin', session_id: 1, payload: 'ready' },
  ]
  const events = await consumeNotifications(signals.map((signal) => ({ kind: 'runtime_signal', data: { signal } })))
  assert.deepEqual(
    events.map((event) => event.properties),
    signals,
  )
})

test('SSE does not move a later part snapshot across a removal notification', async () => {
  const changes = [
    { kind: 'part_updated', part: { part_id: 10, revision: 1 } },
    { kind: 'part_removed', part_id: 10 },
    { kind: 'part_added', part: { part_id: 10, revision: 2 } },
  ]
  const events = await consumeNotifications(changes.map(notification))
  assert.deepEqual(
    events.map((event) => event.properties.kind),
    changes.map((change) => change.kind),
  )
  assert.deepEqual(
    events.map((event) => event.lastEventId),
    ['1', '2', '3'],
  )
})

test('SSE keeps emitted cursors in arrival order when snapshots for multiple parts interleave', async () => {
  const events = await consumeNotifications([
    notification({ kind: 'part_updated', part: { part_id: 10, revision: 1 } }),
    notification({ kind: 'part_updated', part: { part_id: 20, revision: 1 } }),
    notification({ kind: 'part_updated', part: { part_id: 10, revision: 2 } }),
  ])
  assert.deepEqual(
    events.map((event) => event.lastEventId),
    ['2', '3'],
  )
  assert.deepEqual(
    events.map((event) => event.properties.part.part_id),
    [20, 10],
  )
})

test('same-frame coalescing never replaces a newer checkpoint with an older revision or timestamp', async () => {
  const latest = { part_id: 10, revision: 4, updated_at_ms: 12, content: { text: 'latest' } }
  const events = await consumeNotifications([
    notification({ kind: 'part_updated', part: latest }),
    notification({ kind: 'part_updated', part: { ...latest, revision: 3 } }),
    notification({ kind: 'part_updated', part: { ...latest, updated_at_ms: 11 } }),
  ])
  assert.equal(events.length, 1)
  assert.deepEqual(events[0].properties.part, latest)
})

test('SSE establishes each subscription before onOpen and closing suppresses queued data and errors', async () => {
  const originalFetch = globalThis.fetch
  const originalWindow = globalThis.window
  if (!globalThis.window) globalThis.window = globalThis
  const events = []
  const errors = []
  let opens = 0
  let connected
  const opened = new Promise((resolve) => {
    connected = resolve
  })
  globalThis.fetch = async (_, init) =>
    new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(
            new TextEncoder().encode(
              `data: ${JSON.stringify(notification({ kind: 'part_updated', part: { part_id: 1 } }))}\n\n`,
            ),
          )
          init.signal.addEventListener('abort', () => controller.error(new Error('aborted')), { once: true })
        },
      }),
      { headers: { 'content-type': 'text/event-stream' } },
    )
  const client = connectSse({
    endpoint: '/fake',
    onOpen: () => {
      opens++
      connected()
    },
    onEvent: (event) => events.push(event),
    onError: (error) => errors.push(error),
  })
  try {
    await opened
    client.close()
    await new Promise((resolve) => setTimeout(resolve, 30))
    assert.equal(opens, 1)
    assert.equal(events.length, 0)
    assert.equal(errors.length, 0)
  } finally {
    client.close()
    globalThis.fetch = originalFetch
    if (originalWindow === undefined) delete globalThis.window
  }
})

test('reconnecting a stream with no event IDs still calls onOpen again for authoritative recovery', async () => {
  const originalFetch = globalThis.fetch
  const originalWindow = globalThis.window
  if (!globalThis.window) globalThis.window = globalThis
  let opens = 0
  let finish
  const recovered = new Promise((resolve) => {
    finish = resolve
  })
  globalThis.fetch = async () =>
    new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode('retry: 10\n\n'))
          controller.close()
        },
      }),
    )
  const client = connectSse({
    endpoint: '/fake',
    onEvent() {},
    onOpen() {
      if (++opens === 2) finish()
    },
  })
  let deadline
  try {
    await Promise.race([
      recovered,
      new Promise((_, reject) => {
        deadline = setTimeout(() => reject(new Error('reconnect missing')), 3000)
      }),
    ])
    assert.equal(opens, 2)
    assert.equal(client.getStats().lastCursor, null)
  } finally {
    clearTimeout(deadline)
    client.close()
    globalThis.fetch = originalFetch
    if (originalWindow === undefined) delete globalThis.window
  }
})
