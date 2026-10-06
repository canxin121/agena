import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createSharedRead, createRequestLimiter } from '../src/lib/backgroundReads'
import { conditionalJson } from '../src/lib/conditionalJson'
import {
  applyResourceEvent,
  canReuseResource,
  captureResourceObservation,
  checkResourceVersions,
  noteResourceVersion,
  subscribeResource,
} from '../src/lib/resourceSync'
import { createRevalidator } from '../src/lib/revalidation'
import { ensureBrowserTestRuntime } from './testRuntime'

const originalFetch = globalThis.fetch
const originalNow = Date.now
const cleanups: Array<() => void> = []
let browserGlobals: { window: Window & typeof globalThis; document: Document; localStorage: Storage }
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
beforeEach(() => {
  browserGlobals = { window: globalThis.window, document: globalThis.document, localStorage: globalThis.localStorage }
  ensureBrowserTestRuntime()
})
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  globalThis.fetch = originalFetch
  Date.now = originalNow
  Object.assign(globalThis, browserGlobals)
})

test('45 visible resources share one revision request; unchanged bodies send zero requests and one change reads one body', async () => {
  let now = originalNow()
  Date.now = () => now
  const keys = Array.from({ length: 45 }, (_, i) => `session:${1000 + i}:state`)
  const tokens = new Map(keys.map((key) => [key, 'batch-server:1']))
  let revisionReads = 0
  let bodyReads = 0
  let notifications = 0
  for (const key of keys) cleanups.push(subscribeResource(key, () => notifications++))
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'http://agena.test')
    if (url.pathname === '/api/v1/changes/revisions') {
      revisionReads++
      const requested: string[] = JSON.parse(url.searchParams.get('resources')!)
      expect(requested.length).toBe(45)
      return Response.json(Object.fromEntries(requested.map((key) => [key, tokens.get(key)])))
    }
    bodyReads++
    const resource = keys.find((key) => url.pathname.endsWith(key.split(':')[1]!))!
    return Response.json(
      { resource, token: tokens.get(resource) },
      { headers: { etag: `W/"${tokens.get(resource)}"` } },
    )
  }) as typeof fetch
  const readAll = () =>
    Promise.all(keys.map((key) => conditionalJson<{ token: string }>(key, `/resource-test/${key.split(':')[1]}`)))
  await readAll()
  expect(bodyReads).toBe(45)
  bodyReads = notifications = 0
  now += 30_001
  await checkResourceVersions(keys)
  await readAll()
  expect(revisionReads).toBe(1)
  expect(bodyReads).toBe(0)
  expect(notifications).toBe(0)
  tokens.set(keys[7]!, 'batch-server:2')
  now += 30_001
  await checkResourceVersions(keys)
  const current = await readAll()
  expect(revisionReads).toBe(2)
  expect(bodyReads).toBe(1)
  expect(notifications).toBe(1)
  expect(current[7]!.token).toBe('batch-server:2')
})

test('identical reads share one request and cancelling one consumer preserves the others', async () => {
  const pending = deferred<Response>()
  let requests = 0
  let transportSignal!: AbortSignal
  globalThis.fetch = ((_url, init) => {
    requests++
    transportSignal = init!.signal!
    return pending.promise
  }) as typeof fetch
  const controller = new AbortController()
  const cancelled = conditionalJson('session:2001:state', '/resource-test/shared', { signal: controller.signal }).catch(
    () => 'cancelled',
  )
  const consumers = Array.from({ length: 20 }, () => conditionalJson('session:2001:state', '/resource-test/shared'))
  await settle()
  expect(requests).toBe(1)
  controller.abort()
  expect(await cancelled).toBe('cancelled')
  expect(transportSignal.aborted).toBe(false)
  pending.resolve(Response.json({ value: 42 }, { headers: { etag: 'W/"shared-server:1"' } }))
  const values = await Promise.all(consumers)
  expect(values.every((value) => (value as { value: number }).value === 42)).toBe(true)
  await conditionalJson('session:2001:state', '/resource-test/shared')
  expect(requests).toBe(1)
})

test('a forced post-mutation read waits for the old flight, then sends one current conditional request', async () => {
  const requests: Array<ReturnType<typeof deferred<Response>>> = []
  globalThis.fetch = (() => {
    const request = deferred<Response>()
    requests.push(request)
    return request.promise
  }) as typeof fetch
  const old = conditionalJson<{ value: string }>('session:2002:state', '/resource-test/mutation')
  await settle()
  const current = conditionalJson<{ value: string }>('session:2002:state', '/resource-test/mutation', undefined, true)
  expect(requests.length).toBe(1)
  requests[0]!.resolve(Response.json({ value: 'before' }, { headers: { etag: 'W/"mutation-server:1"' } }))
  await settle()
  expect(requests.length).toBe(2)
  requests[1]!.resolve(Response.json({ value: 'after' }, { headers: { etag: 'W/"mutation-server:2"' } }))
  expect((await old).value).toBe('before')
  expect((await current).value).toBe('after')
})

test('304 retains the body; a response from the retired server epoch cannot overwrite a newer observation', async () => {
  const key = 'session:2003:state'
  const url = '/resource-test/restart'
  let requests = 0
  globalThis.fetch = (async (_input, init) => {
    requests++
    if (requests === 1) return Response.json({ value: 'cached' }, { headers: { etag: 'W/"old-server:2"' } })
    expect(new Headers(init?.headers).get('if-none-match')).toBe('W/"old-server:2"')
    return new Response(null, { status: 304 })
  }) as typeof fetch
  await conditionalJson(key, url)
  expect(await conditionalJson(key, url, undefined, true)).toEqual({ value: 'cached' })
  const beforeRestart = captureResourceObservation(key)
  expect(noteResourceVersion(key, 'new-server:1')).toBe(true)
  expect(noteResourceVersion(key, 'old-server:3', beforeRestart)).toBe(false)
  expect(canReuseResource(key, 'new-server:1')).toBe(true)
  expect(noteResourceVersion(key, 'new-server:0')).toBe(false)
})

test('SSE revision tokens wake only the changed resource and do not perform a separate version request', async () => {
  let notified = 0
  const key = 'session:2004:files'
  cleanups.push(subscribeResource(key, () => notified++))
  noteResourceVersion(key, 'event-server:1', captureResourceObservation(key))
  globalThis.fetch = (async () => {
    throw new Error('SSE notification must not make a version request')
  }) as typeof fetch
  applyResourceEvent({
    type: 'session_changed',
    properties: {
      session_id: 2004,
      kind: 'part_updated',
      resource_revisions: { [key]: 'event-server:2', 'session:2999:files': 'event-server:2' },
    },
  })
  expect(notified).toBe(1)
  expect(canReuseResource(key, 'event-server:2')).toBe(true)
  await checkResourceVersions([key])
  expect(notified).toBe(1)
})

test('cancelling the last shared consumer aborts transport and queued work never starts', async () => {
  const limiter = createRequestLimiter(1)
  const blocked = deferred<void>()
  const first = limiter(() => blocked.promise)
  let started = false
  const read = createSharedRead((signal) =>
    limiter(async () => {
      started = true
    }, signal),
  )
  const controller = new AbortController()
  const consumer = read.join(controller.signal).catch(() => {})
  await settle()
  controller.abort()
  await consumer
  expect(read.signal.aborted).toBe(true)
  blocked.resolve()
  await first
  await read.promise.catch(() => {})
  expect(started).toBe(false)
})

test('failure backoff survives event bursts and direct refresh calls; hidden and disposed queues stop work', async () => {
  const originalTimers = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout }
  let now = 1_000_000
  let serial = 0
  const timers = new Map<number, { at: number; callback: () => void }>()
  globalThis.setTimeout = ((callback: () => void, delay: number) => {
    const id = ++serial
    timers.set(id, { at: now + delay, callback })
    return id
  }) as unknown as typeof setTimeout
  globalThis.clearTimeout = ((id: number) => {
    timers.delete(id)
  }) as typeof clearTimeout
  Date.now = () => now
  const advance = async (ms: number) => {
    const end = now + ms
    while (true) {
      const next = [...timers.entries()].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
      if (!next) break
      now = next[1].at
      timers.delete(next[0])
      next[1].callback()
      await settle()
    }
    now = end
    await settle()
  }
  let visible = true
  let calls = 0
  const queue = createRevalidator(
    async () => {
      calls++
      throw new Error('overloaded')
    },
    {
      intervalMs: 1000,
      retryMs: 5000,
      enabled: () => visible,
    },
  )
  try {
    await queue.refresh().catch(() => {})
    for (let i = 0; i < 1000; i++) {
      queue.invalidate(0)
      await queue.refresh()
    }
    await advance(4999)
    expect(calls).toBe(1)
    await advance(1)
    expect(calls).toBe(2)
    visible = false
    queue.pause()
    await advance(60_000)
    expect(calls).toBe(2)
    visible = true
    queue.resume()
    await advance(0)
    expect(calls).toBe(3)
    queue.dispose()
    await advance(60_000)
    expect(calls).toBe(3)
    expect(timers.size).toBe(0)
  } finally {
    queue.dispose()
    Object.assign(globalThis, originalTimers)
  }
})
