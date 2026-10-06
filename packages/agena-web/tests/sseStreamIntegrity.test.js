import assert from 'node:assert/strict'
import test from 'node:test'

import { connectSse } from '../src/lib/sse.ts'

// Use actual Response/ReadableStream objects so decoding sees the same byte
// boundaries as a network response. EOF flushes the queue before onError runs.
async function consumeChunks(chunks, onEvent) {
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
    onEvent: (event) => {
      events.push(event)
      onEvent?.(event)
    },
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

function consumeNotifications(notifications, onEvent) {
  const stream = notifications.map((value, index) => `id: ${index + 1}\ndata: ${JSON.stringify(value)}\n\n`).join('')
  return consumeChunks([new TextEncoder().encode(stream)], onEvent)
}

test('SSE drains recovery bursts in order at EOF while allowing other timers to run', async () => {
  let heartbeats = 0
  const timer = setInterval(() => {
    heartbeats++
  }, 1)
  try {
    const payloads = Array.from({ length: 80 }, (_, seq) => ({ type: 'data', seq, data: String(seq) }))
    const events = await consumeNotifications(payloads, () => {
      const deadline = performance.now() + 0.4
      while (performance.now() < deadline) {
        /* model a costly event consumer */
      }
    })
    assert.deepEqual(
      events.map((event) => event.seq),
      payloads.map((event) => event.seq),
    )
    assert.ok(heartbeats > 0, 'large bursts yield before all consumers finish')
  } finally {
    clearInterval(timer)
  }
})

test('closing from an event consumer during the EOF drain suppresses remaining callbacks', async () => {
  const originalFetch = globalThis.fetch
  const originalWindow = globalThis.window
  if (!globalThis.window) globalThis.window = globalThis
  globalThis.fetch = async () =>
    new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(
            new TextEncoder().encode('data: {"type":"data","seq":1}\n\ndata: {"type":"data","seq":2}\n\n'),
          )
          controller.close()
        },
      }),
    )
  const events = []
  const errors = []
  let delivered
  const consumed = new Promise((resolve) => {
    delivered = resolve
  })
  const client = connectSse({
    endpoint: '/fake',
    autoReconnect: false,
    onEvent(event) {
      events.push(event.seq)
      client.close()
      delivered()
    },
    onError(error) {
      errors.push(error)
    },
  })
  try {
    await consumed
    await new Promise((resolve) => setTimeout(resolve, 20))
    assert.deepEqual(events, [1])
    assert.deepEqual(errors, [])
  } finally {
    client.close()
    globalThis.fetch = originalFetch
    if (originalWindow === undefined) delete globalThis.window
  }
})

test('SSE preserves multiline JSON, UTF-8, and cursors across split CRLF bytes', async () => {
  const stream =
    'id: 1\r\ndata: {"kind":"runtime_signal",\r\ndata: "data":{"signal":{"kind":"plugin","payload":{"label":"中文"}}}}\r\n\r\n'
  const bytes = new TextEncoder().encode(stream)
  const events = await consumeChunks(Array.from(bytes, (byte) => new Uint8Array([byte])))
  assert.equal(events.length, 1)
  assert.equal(events[0].lastEventId, '1')
  assert.equal(events[0].properties.payload.label, '中文')
})

test('SSE dispatches current workbench and filesystem event payloads intact', async () => {
  const payloads = [
    { type: 'preview.sessions.changed', revision: 'preview-epoch:2' },
    { type: 'terminal-ui-state.snapshot', state: { version: 7, sessionIds: ['term_1'] }, seq: 0 },
    {
      type: 'terminal-ui-state.patch',
      properties: { ops: [{ type: 'state.replace', state: { version: 8 } }] },
      seq: 1,
    },
    { type: 'connected', seq: 0 },
    { type: 'data', data: '中文\n', seq: 1 },
    { type: 'resync', seq: 2 },
    { type: 'exit', seq: 3 },
    { type: 'git.watch.status', properties: { worktreeSignature: 'changed', totalFiles: 1 } },
    { type: 'agena:fs-changed', properties: { paths: ['src/a.rs'] } },
  ]
  const events = await consumeNotifications(payloads)
  assert.deepEqual(
    events.map(({ lastEventId, ...payload }) => payload),
    payloads,
  )
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
