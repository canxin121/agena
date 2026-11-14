import { afterEach, expect, test } from 'bun:test'
import { streamContent, type ContentPage } from '../src/lib/content'

const originalFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = originalFetch
})
const encoder = new TextEncoder()
const cursor = { epoch: '11111111-1111-4111-8111-111111111111', sequence: 4 }
const completed: ContentPage = {
  resource: {
    resource_id: 'reply',
    owner_session_id: 6,
    part_id: 8,
    kind: 'text',
    state: 'complete',
    cursor,
    committed_cursor: cursor,
    retained_ranges: [],
    dropped_bytes: 0,
    total_bytes: 4,
    capture_error: null,
  },
  chunks: [{ cursor, captured_at_ms: 0, payload: { type: 'text', text: 'done' } }],
  next_cursor: cursor,
  has_more: false,
  gap: false,
}
const frame = () => encoder.encode(`event: content\ndata: ${JSON.stringify(completed)}\n\n`)

function response(
  signal: AbortSignal,
  start?: (stream: ReadableStreamDefaultController<Uint8Array>) => void,
  cancel?: () => void,
) {
  return new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        signal.addEventListener(
          'abort',
          () => {
            try {
              controller.error(signal.reason)
            } catch {}
          },
          { once: true },
        )
        start?.(controller)
      },
      cancel,
    }),
    { headers: { 'content-type': 'text/event-stream' } },
  )
}

test('a stalled content handshake aborts instead of leaving a reply loading indefinitely', async () => {
  let requestSignal: AbortSignal | undefined
  globalThis.fetch = (async (_url, init) =>
    new Promise<Response>((_resolve, reject) => {
      requestSignal = init!.signal!
      requestSignal.addEventListener('abort', () => reject(requestSignal!.reason), { once: true })
    })) as typeof fetch
  await expect(
    streamContent('6', 'reply', null, new AbortController().signal, () => {}, { handshakeMs: 30 }),
  ).rejects.toThrow()
  expect(requestSignal?.aborted).toBe(true)
})

test('a half-open content body aborts so its owner can reconnect at the accepted cursor', async () => {
  let requestSignal: AbortSignal | undefined
  globalThis.fetch = (async (_url, init) => {
    requestSignal = init!.signal!
    return response(requestSignal)
  }) as typeof fetch
  await expect(
    streamContent('6', 'reply', cursor, new AbortController().signal, () => {}, { idleMs: 30 }),
  ).rejects.toThrow()
  expect(requestSignal?.aborted).toBe(true)
})

test('heartbeats extend the idle deadline and a final page completes without waiting for EOF', async () => {
  let heartbeats: ReturnType<typeof setInterval> | undefined
  let finish: ReturnType<typeof setTimeout> | undefined
  globalThis.fetch = (async (_url, init) =>
    response(init!.signal!, (controller) => {
      heartbeats = setInterval(() => controller.enqueue(encoder.encode(': heartbeat\n\n')), 10)
      finish = setTimeout(() => {
        clearInterval(heartbeats)
        controller.enqueue(frame())
      }, 90)
    })) as typeof fetch
  const pages: ContentPage[] = []
  try {
    await streamContent('6', 'reply', null, new AbortController().signal, (page) => pages.push(page), { idleMs: 40 })
    expect(pages).toEqual([completed])
  } finally {
    clearInterval(heartbeats)
    clearTimeout(finish)
  }
})
