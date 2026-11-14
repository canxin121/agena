import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia'
import {
  consumeAuthRequiredFromStorage,
  emitAuthRequired,
  OC_AUTH_REQUIRED_STORAGE_KEY,
  readAuthRequiredFromStorage,
} from '../src/lib/authEvents'
import { localStorageKeys } from '../src/lib/persistence/storageKeys'
import { readActiveServerAuthToken } from '../src/lib/serverAuthToken'
import { useAuthStore } from '../src/stores/auth'

const browserGlobals = ['window', 'document', 'localStorage', 'sessionStorage'] as const
const originals = new Map(browserGlobals.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]))
const originalFetch = globalThis.fetch
const origin = 'https://agena.test'
let pinia: Pinia

function memoryStorage() {
  const values = new Map<string, string>()
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  }
}

beforeEach(() => {
  const values = {
    window: Object.assign(new EventTarget(), { location: { origin } }),
    document: undefined,
    localStorage: memoryStorage(),
    sessionStorage: memoryStorage(),
  }
  for (const name of browserGlobals)
    Object.defineProperty(globalThis, name, { configurable: true, value: values[name] })
  pinia = createPinia()
  setActivePinia(pinia)
})

afterEach(() => {
  disposePinia(pinia)
  globalThis.fetch = originalFetch
  for (const name of browserGlobals) {
    const descriptor = originals.get(name)
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

describe('authentication across page reloads', () => {
  test('a same-document early denial is consumed exactly once', () => {
    const detail = { status: 401, code: 'auth.required', authTokenVersion: 0 }
    emitAuthRequired(detail)
    expect(consumeAuthRequiredFromStorage()?.detail).toEqual(detail)
    expect(consumeAuthRequiredFromStorage()).toBeNull()
    expect(sessionStorage.getItem(OC_AUTH_REQUIRED_STORAGE_KEY)).toBeNull()
  })

  test('a successful session check clears a pending denial without rotating the saved token', async () => {
    localStorage.setItem(localStorageKeys.auth.serverTokenByBaseUrl, JSON.stringify({ [origin]: 'persisted-login' }))
    emitAuthRequired({ status: 401, code: 'auth.required', authTokenVersion: 0 })
    globalThis.fetch = (async () =>
      new Response(JSON.stringify({ authenticated: true }), {
        headers: { 'content-type': 'application/json' },
      })) as typeof fetch
    const auth = useAuthStore()
    await auth.refresh()
    expect(auth.authenticated).toBe(true)
    expect(readActiveServerAuthToken()).toBe('persisted-login')
    expect(sessionStorage.getItem(OC_AUTH_REQUIRED_STORAGE_KEY)).toBeNull()
  })

  test('an earlier document cannot replay a version-zero denial over a persisted login', async () => {
    localStorage.setItem(localStorageKeys.auth.serverTokenByBaseUrl, JSON.stringify({ [origin]: 'persisted-login' }))
    sessionStorage.setItem(
      OC_AUTH_REQUIRED_STORAGE_KEY,
      JSON.stringify({
        at: Date.now(),
        documentId: 'previous-document',
        detail: { status: 401, code: 'auth.required', authTokenVersion: 0 },
      }),
    )
    // The request version is process-local and starts at zero after a reload.
    // A previous document's version zero must not become a fresh login denial.
    expect(consumeAuthRequiredFromStorage()).toBeNull()
    expect(sessionStorage.getItem(OC_AUTH_REQUIRED_STORAGE_KEY)).toBeNull()
    globalThis.fetch = (async (_, init) => {
      expect(new Headers(init?.headers).get('authorization')).toBe('Bearer persisted-login')
      return new Response(JSON.stringify({ authenticated: true }), { headers: { 'content-type': 'application/json' } })
    }) as typeof fetch
    const auth = useAuthStore()
    await auth.refresh()
    expect(auth.authenticated).toBe(true)
    expect(auth.needsLogin).toBe(false)
    expect(readActiveServerAuthToken()).toBe('persisted-login')
  })

  test('successful password login clears a pending pre-login denial', async () => {
    emitAuthRequired({ status: 401, code: 'auth.required', authTokenVersion: 0 })
    expect(readAuthRequiredFromStorage()).not.toBeNull()
    globalThis.fetch = (async () =>
      new Response(JSON.stringify({ authenticated: true, token: 'new-login' }), {
        headers: { 'content-type': 'application/json' },
      })) as typeof fetch
    const auth = useAuthStore()
    await auth.login('test-password')
    expect(auth.authenticated).toBe(true)
    expect(readActiveServerAuthToken()).toBe('new-login')
    expect(readAuthRequiredFromStorage()).toBeNull()
    expect(sessionStorage.getItem(OC_AUTH_REQUIRED_STORAGE_KEY)).toBeNull()
  })
})
