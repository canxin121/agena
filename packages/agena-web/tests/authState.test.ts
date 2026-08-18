import assert from 'node:assert/strict'
import test from 'node:test'
import { createPinia, setActivePinia } from 'pinia'

import { ensureBrowserTestRuntime } from './testRuntime'
import { useAuthStore } from '../src/stores/auth'
import { localStorageKeys } from '../src/lib/persistence/storageKeys'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}

function jsonResponse(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

test('late pre-login auth refresh cannot relock a successful login', async () => {
  const storage = ensureBrowserTestRuntime()
  storage.removeItem(localStorageKeys.auth.uiTokenByBaseUrl)
  const windowRecord = globalThis.window as Record<string, unknown>
  windowRecord.location = { origin: 'http://agena.test' }

  const originalFetch = globalThis.fetch
  const pendingRefresh = deferred<Response>()
  globalThis.fetch = (async (_input, init) => {
    if (init?.method === 'POST') {
      return jsonResponse({ authenticated: true, disabled: false, token: 'fresh-token' })
    }
    return await pendingRefresh.promise
  }) as typeof fetch

  try {
    setActivePinia(createPinia())
    const auth = useAuthStore()
    const refreshPromise = auth.refresh()

    await auth.login('correct password')
    assert.equal(auth.authenticated, true)
    assert.equal(auth.needsLogin, false)

    pendingRefresh.resolve(jsonResponse({ authenticated: false, locked: true }, 401))
    await refreshPromise

    assert.equal(auth.authenticated, true)
    assert.equal(auth.needsLogin, false)
    assert.equal(auth.loginInFlight, false)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test('a successful auth status response can still require password login', async () => {
  const storage = ensureBrowserTestRuntime()
  storage.removeItem(localStorageKeys.auth.uiTokenByBaseUrl)

  const originalFetch = globalThis.fetch
  globalThis.fetch = (async () => jsonResponse({ authenticated: false, locked: true })) as typeof fetch

  try {
    setActivePinia(createPinia())
    const auth = useAuthStore()
    await auth.refresh()

    assert.equal(auth.checked, true)
    assert.equal(auth.authenticated, false)
    assert.equal(auth.needsLogin, true)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test('auth-required handling cannot interrupt an in-flight password submission', async () => {
  const storage = ensureBrowserTestRuntime()
  storage.removeItem(localStorageKeys.auth.uiTokenByBaseUrl)

  const originalFetch = globalThis.fetch
  const pendingLogin = deferred<Response>()
  globalThis.fetch = (async (_input, init) => {
    if (init?.method === 'POST') return await pendingLogin.promise
    return jsonResponse({ authenticated: true, disabled: false })
  }) as typeof fetch

  try {
    setActivePinia(createPinia())
    const auth = useAuthStore()
    const loginPromise = auth.login('correct password')

    assert.equal(auth.loginInFlight, true)
    assert.equal(auth.requireLogin(), false)
    assert.equal(auth.loginInFlight, true)

    pendingLogin.resolve(jsonResponse({ authenticated: true, disabled: false, token: 'fresh-token' }))
    await loginPromise

    assert.equal(auth.authenticated, true)
    assert.equal(auth.needsLogin, false)
  } finally {
    globalThis.fetch = originalFetch
  }
})
