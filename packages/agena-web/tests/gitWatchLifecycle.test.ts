import assert from 'node:assert/strict'
import test, { after } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'

import { ensureBrowserTestRuntime } from './testRuntime'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
after(() => vite.close())
const { useGitWatchSse } = (await vite.ssrLoadModule(
  '/src/composables/git/useGitWatchSse.ts',
)) as typeof import('../src/composables/git/useGitWatchSse')

test('Git watch resume compares the new snapshot with the last visible snapshot and isolates selections', async () => {
  const original = {
    fetch: globalThis.fetch,
    window: globalThis.window,
    document: globalThis.document,
    localStorage: globalThis.localStorage,
  }
  ensureBrowserTestRuntime()
  type Snapshot = { signature: string }
  const streams: Array<{ signal: AbortSignal; controller: ReadableStreamDefaultController<Uint8Array> }> = []
  const received: Array<{ payload: Snapshot; previous: Snapshot | null }> = []
  globalThis.fetch = (async (_input, init) => {
    const signal = init!.signal!
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        streams.push({ signal, controller })
        signal.addEventListener('abort', () => controller.error(signal.reason), { once: true })
      },
    })
    return new Response(body, { headers: { 'content-type': 'text/event-stream' } })
  }) as typeof fetch
  const watch = useGitWatchSse<Snapshot>({
    buildUrl: (directory) => `/git-watch-lifecycle?directory=${encodeURIComponent(directory)}`,
    onPayload: (payload, previous) => received.push({ payload, previous }),
  })
  const announce = async (signature: string) => {
    for (let i = 0; i < 30; i++) await Promise.resolve()
    streams
      .at(-1)!
      .controller.enqueue(
        new TextEncoder().encode(
          `data: ${JSON.stringify({ type: 'git.watch.status', properties: { signature } })}\n\n`,
        ),
      )
    // Exercise the client's actual frame batching and dispatch.
    await new Promise((resolve) => setTimeout(resolve, 30))
  }
  try {
    watch.startWatch('/repo', 'file-a')
    watch.startWatch('/repo', 'file-a')
    await announce('before')
    assert.equal(streams.length, 1, 'repeated start retains one connection')
    assert.equal(received[0]!.previous, null)
    watch.stopWatch()
    assert.equal(streams[0]!.signal.aborted, true)

    watch.startWatch('/repo', 'file-a')
    await announce('changed-while-paused')
    assert.deepEqual(received[1], {
      payload: { signature: 'changed-while-paused' },
      previous: { signature: 'before' },
    })

    watch.startWatch('/repo', 'file-b')
    await announce('other-selection')
    assert.equal(streams[1]!.signal.aborted, true)
    assert.equal(received[2]!.previous, null, 'another selection has an independent baseline')
    watch.startWatch('/repo', 'file-a')
    await announce('after-return')
    assert.deepEqual(received[3]!.previous, { signature: 'changed-while-paused' })
  } finally {
    watch.stopWatch()
    assert.equal(streams.at(-1)?.signal.aborted, true)
    Object.assign(globalThis, original)
  }
})
