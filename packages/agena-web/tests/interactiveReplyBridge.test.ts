import assert from 'node:assert/strict'
import test from 'node:test'
import { bridgeFetch, type AppSnapshot } from '../src/features/interactive/api'
import type { AppManifest } from '../src/features/interactive/model'

const manifest: AppManifest = {
  id: 'tasks',
  title: 'Tasks',
  html: '<main>Tasks</main>',
  services: [{ name: 'api', command: 'python3' }],
}
const snapshot = {
  services: { api: { pid: 123, state: 'ready', proxyBasePath: '/api/v1/workbench/preview/s/app-test/' } },
} as AppSnapshot

test('the app bridge preserves backend error statuses and content types', async () => {
  const originalFetch = globalThis.fetch
  try {
    for (const status of [200, 400, 403, 500]) {
      globalThis.fetch = (async () =>
        new Response('backend response', {
          status,
          headers: { 'content-type': 'text/plain', 'set-cookie': 'private=value', 'x-private': 'private' },
        })) as typeof fetch
      const result = await bridgeFetch(
        snapshot,
        manifest,
        { service: 'api', path: '/tasks' },
        new AbortController().signal,
      )
      assert.equal(result.status, status)
      assert.equal(result.body, 'backend response')
      assert.deepEqual(result.headers, { 'content-type': 'text/plain' })
    }
  } finally {
    globalThis.fetch = originalFetch
  }
})

test('the app bridge cancels oversized successful and error response streams', async () => {
  const originalFetch = globalThis.fetch
  try {
    for (const status of [200, 500]) {
      let cancelled = false
      globalThis.fetch = (async () =>
        new Response(
          new ReadableStream({
            pull(controller) {
              controller.enqueue(new Uint8Array(256 * 1024))
            },
            cancel() {
              cancelled = true
            },
          }),
          { status },
        )) as typeof fetch
      await assert.rejects(
        bridgeFetch(snapshot, manifest, { service: 'api', path: '/tasks' }, new AbortController().signal),
        /exceeds 1 MiB/,
      )
      assert.equal(cancelled, true)
    }
  } finally {
    globalThis.fetch = originalFetch
  }
})

test('multibyte requests are bounded before the bridge calls authenticated fetch', async () => {
  const originalFetch = globalThis.fetch
  let requested = false
  globalThis.fetch = (async () => {
    requested = true
    return new Response('unexpected')
  }) as typeof fetch
  try {
    await assert.rejects(
      bridgeFetch(
        snapshot,
        manifest,
        {
          service: 'api',
          path: '/tasks',
          method: 'POST',
          body: '中'.repeat(400000),
        },
        new AbortController().signal,
      ),
      /at most 1 MiB/,
    )
    assert.equal(requested, false)
  } finally {
    globalThis.fetch = originalFetch
  }
})
