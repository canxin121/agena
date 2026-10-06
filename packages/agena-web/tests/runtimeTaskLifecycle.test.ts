import { afterEach, beforeEach, expect, test } from 'bun:test'
import { waitForRuntimeTask } from '../src/lib/runtimeTask'
import { reloadAgenaRuntime } from '../src/lib/reload'
import { applyResourceEvent } from '../src/lib/resourceSync'
import { ensureBrowserTestRuntime } from './testRuntime'

let original: Pick<typeof globalThis, 'fetch' | 'window' | 'document' | 'localStorage'>
beforeEach(() => {
  original = {
    fetch: globalThis.fetch,
    window: globalThis.window,
    document: globalThis.document,
    localStorage: globalThis.localStorage,
  }
  ensureBrowserTestRuntime()
  Object.assign(globalThis, {
    document: Object.assign(new EventTarget(), { hidden: false, visibilityState: 'visible' }),
  })
})
afterEach(() => Object.assign(globalThis, original))
const settle = () => new Promise((resolve) => setTimeout(resolve, 10))
const task = (suffix: string) => ({ id: `rtask_wait_${suffix}`, kind: 'marketplace_plugin_install', status: 'running' })
function announce(id: string, revision: number, status = 'running') {
  applyResourceEvent({
    type: 'runtime_signal',
    properties: {
      kind: 'activity',
      resource_revisions: { [`activity:${id}`]: `runtime-wait-server:${revision}` },
      payload: { activity: { id, kind: 'runtime', status, message: status === 'succeeded' ? 'installed' : undefined } },
    },
  })
}

test('runtime task waits use one scoped initial read; event bursts and completion read no lists or status bodies', async () => {
  const initial = task('events')
  const calls: string[] = []
  globalThis.fetch = (async (input) => {
    calls.push(new URL(String(input), 'http://agena.test').pathname)
    return Response.json({ ...initial, kind: 'runtime' }, { headers: { etag: 'W/"runtime-wait-server:1"' } })
  }) as typeof fetch
  const cancel = new AbortController()
  const result = waitForRuntimeTask(initial, { signal: cancel.signal })
  try {
    await settle()
    expect(calls).toEqual([`/api/v1/activities/${initial.id}`])
    for (let i = 0; i < 1000; i++) announce(initial.id, i + 2)
    await settle()
    expect(calls.length).toBe(1)
    announce(initial.id, 1002, 'succeeded')
    expect(await result).toMatchObject({ ...initial, status: 'succeeded', message: 'installed' })
    expect(calls.length).toBe(1)
  } finally {
    cancel.abort()
    await result.catch(() => {})
  }
})

test('a task finishing before the first read resolves directly from SSE without a request', async () => {
  let calls = 0
  globalThis.fetch = (async () => {
    calls++
    throw new Error('completion already supplied the result')
  }) as typeof fetch
  const initial = task('instant')
  const result = waitForRuntimeTask(initial)
  announce(initial.id, 1, 'succeeded')
  expect((await result).status).toBe('succeeded')
  await settle()
  expect(calls).toBe(0)
})

test('hidden task waits send no reads and cancelling after resume aborts transport and subscriptions', async () => {
  let calls = 0
  let transport: AbortSignal | undefined
  globalThis.fetch = ((_input, init) => {
    calls++
    transport = init!.signal!
    return new Promise((_resolve, reject) =>
      transport!.addEventListener('abort', () => reject(transport!.reason), { once: true }),
    )
  }) as typeof fetch
  document.hidden = true
  const controller = new AbortController()
  const initial = task('cancel')
  const result = waitForRuntimeTask(initial, { signal: controller.signal }).catch((error) => error)
  try {
    await settle()
    expect(calls).toBe(0)
    document.hidden = false
    document.dispatchEvent(new Event('visibilitychange'))
    await settle()
    expect(calls).toBe(1)
    controller.abort()
    expect((await result).name).toBe('AbortError')
    expect(transport?.aborted).toBe(true)
    announce(initial.id, 2, 'succeeded')
    await settle()
    expect(calls).toBe(1)
  } finally {
    controller.abort()
    await result
  }
})

test('runtime reload waits for its own terminal activity without repeatedly reading full runtime status', async () => {
  const initial = { ...task('reload'), kind: 'runtime_reload' }
  const paths: string[] = []
  globalThis.fetch = (async (input) => {
    const path = new URL(String(input), 'http://agena.test').pathname
    paths.push(path)
    return path === '/api/v1/runtime/reload'
      ? Response.json({ started: true, task: initial })
      : Response.json(
          { ...initial, kind: 'runtime', status: 'succeeded' },
          { headers: { etag: 'W/"runtime-wait-server:1"' } },
        )
  }) as typeof fetch
  const result = await reloadAgenaRuntime()
  expect(result.task.status).toBe('succeeded')
  expect(result.task.kind).toBe('runtime_reload')
  expect(paths).toEqual(['/api/v1/runtime/reload', `/api/v1/activities/${initial.id}`])
})

test('a missed completion before a short deadline uses one final conditional validation', async () => {
  const initial = task('missed')
  let calls = 0
  globalThis.fetch = (async (_input, init) => {
    calls++
    if (calls === 2) expect(new Headers(init?.headers).get('if-none-match')).toBe('W/"runtime-wait-server:1"')
    return Response.json(
      { ...initial, status: calls === 1 ? 'running' : 'succeeded' },
      { headers: { etag: `W/"runtime-wait-server:${calls}"` } },
    )
  }) as typeof fetch
  const result = await waitForRuntimeTask(initial, { timeoutMs: 20 })
  expect(result.status).toBe('succeeded')
  expect(calls).toBe(2)
})
