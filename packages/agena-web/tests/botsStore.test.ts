import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia'
import { useBotsStore } from '../src/stores/bots'
import type { BotProfile } from '../src/lib/bots'
import { writeServerAuthTokenForBaseUrl } from '../src/lib/serverAuthToken'
const originalFetch = globalThis.fetch
let pinia: Pinia
const bot = (revision = 1, archived = false): BotProfile =>
  ({
    id: '11111111-1111-4111-8111-111111111111',
    name: `Name ${revision}`,
    revision,
    archived,
    persona: { instructions: '' },
    session_id: 1,
  }) as BotProfile
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}
beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
})
afterEach(() => {
  globalThis.fetch = originalFetch
  disposePinia(pinia)
  setActivePinia(undefined)
})
describe('shared Bot catalog races', () => {
  test('a post-rename authoritative absence removes the row and stale applies cannot bring it back', async () => {
    const store = useBotsStore()
    store.apply(bot(2))
    globalThis.fetch = (async () => Response.json({ items: [] })) as typeof fetch
    await store.refresh()
    expect(store.profiles).toEqual([])
    expect(store.authoritative).toBe(1)
    store.apply(bot(2))
    expect(store.profiles).toEqual([])
  })
  test('an older list and delayed profile cannot undo a local archive', async () => {
    const response = deferred<Response>()
    globalThis.fetch = (() => response.promise) as typeof fetch
    const store = useBotsStore()
    const pending = store.refresh()
    store.apply(bot(3, true))
    response.resolve(Response.json({ items: [bot(1)] }))
    await pending
    store.apply(bot(2))
    expect(store.profiles).toEqual([])
    expect(store.authoritative).toBe(1)
  })
  test('failed refresh does not assert authoritative absence or discard existing catalog', async () => {
    const store = useBotsStore()
    store.apply(bot())
    globalThis.fetch = (async () => {
      throw new Error('offline')
    }) as typeof fetch
    await store.refresh()
    expect(store.profiles).toHaveLength(1)
    expect(store.authoritative).toBe(0)
    expect(store.error).toContain('offline')
  })
  test('auth reset closes requests and ignores a pre-reset list response', async () => {
    const originalWindow = globalThis.window
    const fakeWindow = new EventTarget() as Window & typeof globalThis
    globalThis.window = fakeWindow
    const old = deferred<Response>()
    const fresh = deferred<Response>()
    let calls = 0
    globalThis.fetch = (() => (++calls === 1 ? old.promise : fresh.promise)) as typeof fetch
    const store = useBotsStore()
    try {
      const stop = store.start()
      store.open('edit', bot().id)
      writeServerAuthTokenForBaseUrl('https://test.invalid', 'new-token')
      expect(store.request).toBeNull()
      expect(store.version).toBe(1)
      old.resolve(Response.json({ items: [bot()] }))
      fresh.resolve(Response.json({ items: [] }))
      await Promise.resolve()
      await store.refresh()
      expect(store.profiles).toEqual([])
      stop()
    } finally {
      globalThis.window = originalWindow
    }
  })
})
