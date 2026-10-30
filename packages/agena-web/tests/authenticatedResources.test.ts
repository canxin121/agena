import { afterEach, describe, expect, test } from 'bun:test'
import { effectScope, nextTick } from 'vue'
import { isBackendUrl } from '../src/lib/backend'
import { browserResourceUrl, isAuthenticatedResourceUrl, isPassiveResourceMime } from '../src/lib/resourceUrls'
import { apiBlob, apiFetch, apiText } from '../src/lib/api'
import { downloadResource, loadBrowserResource, openResource } from '../src/lib/authenticatedResources'
import { clearServerAuthTokenForBaseUrl, writeServerAuthTokenForBaseUrl } from '../src/lib/serverAuthToken'
import { useAuthenticatedResource } from '../src/lib/useAuthenticatedResource'

const originalFetch = globalThis.fetch
const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
const originalDocument = Object.getOwnPropertyDescriptor(globalThis, 'document')
const originalCreate = URL.createObjectURL
const originalRevoke = URL.revokeObjectURL

function installBrowser() {
  const entries = new Map<string, string>()
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { location: { origin: 'https://agena.test' } },
  })
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => entries.get(key) ?? null,
      setItem: (key: string, value: string) => entries.set(key, value),
      removeItem: (key: string) => entries.delete(key),
    },
  })
  writeServerAuthTokenForBaseUrl('https://agena.test', 'fixture-token')
}

afterEach(() => {
  globalThis.fetch = originalFetch
  URL.createObjectURL = originalCreate
  URL.revokeObjectURL = originalRevoke
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow)
  else Reflect.deleteProperty(globalThis, 'window')
  if (originalStorage) Object.defineProperty(globalThis, 'localStorage', originalStorage)
  else Reflect.deleteProperty(globalThis, 'localStorage')
  if (originalDocument) Object.defineProperty(globalThis, 'document', originalDocument)
  else Reflect.deleteProperty(globalThis, 'document')
})

function pendingResponseBody() {
  let signal: AbortSignal | null | undefined
  let startBody: () => void = () => {}
  let finishBody: (blob: Blob) => void = () => {}
  const started = new Promise<void>((resolve) => {
    startBody = resolve
  })
  const body = new Promise<Blob>((resolve) => {
    finishBody = resolve
  })
  globalThis.fetch = (async (_, init) => {
    signal = init?.signal
    const response = new Response('', { headers: { 'content-type': 'image/png' } })
    response.blob = () => {
      startBody()
      return body
    }
    return response
  }) as typeof fetch
  return { started, finish: () => finishBody(new Blob(['private image'], { type: 'image/png' })), signal: () => signal }
}

describe('authenticated resource boundaries', () => {
  test('only the active backend API is authenticated, including absolute same-origin links', () => {
    const base = 'https://agena.test'
    for (const value of ['/api/v1/sessions/46/media?path=x', 'https://agena.test/api/v1/workbench/fs/raw?path=x']) {
      expect(isAuthenticatedResourceUrl(value, base)).toBe(true)
    }
    for (const value of [
      'https://elsewhere.test/api/v1/sessions/46/media',
      '//elsewhere.test/api/x',
      'https://agena.test.evil.test/api/x',
      'https://name:secret@agena.test/api/x',
      'data:image/png;base64,eA==',
      'blob:https://agena.test/id',
      '/outside-api.png',
    ]) {
      expect(isAuthenticatedResourceUrl(value, base)).toBe(false)
    }
    expect(isBackendUrl('https://elsewhere.test/api/x', base)).toBe(false)
  })

  test('navigation refuses executable schemes and control-character URL tricks', () => {
    for (const value of [
      'javascript:alert(1)',
      'java\nscript:alert(1)',
      'file:///private',
      '\\evil.test/a',
      '//evil.test/a',
    ]) {
      expect(browserResourceUrl(value)).toBe('')
    }
    expect(browserResourceUrl('/api/v1/sessions/46/media?path=%E5%9B%BE.png')).toContain('/api/')
  })

  test('API helpers never send the active bearer token or cookies to external hosts', async () => {
    installBrowser()
    const requests: Array<{ url: string; authorization: string | null; credentials?: RequestCredentials }> = []
    globalThis.fetch = (async (url, init) => {
      requests.push({
        url: String(url),
        authorization: new Headers(init?.headers).get('authorization'),
        credentials: init?.credentials,
      })
      return new Response('ok')
    }) as typeof fetch
    for (const request of [apiFetch, apiText, apiBlob]) {
      await request('/api/v1/test')
      await request('https://elsewhere.test/file')
    }
    expect(
      requests
        .filter((request) => request.url.startsWith('https://agena.test'))
        .every((request) => request.authorization === 'Bearer fixture-token'),
    ).toBe(true)
    expect(
      requests
        .filter((request) => request.url.startsWith('https://elsewhere.test'))
        .every((request) => request.authorization === null && request.credentials === 'omit'),
    ).toBe(true)
  })

  test('protected media is fetched with a header and creates a releasable browser URL', async () => {
    installBrowser()
    let created: Blob | undefined
    const revoked: string[] = []
    URL.createObjectURL = (blob) => {
      if (blob instanceof Blob) created = blob
      return 'blob:https://agena.test/preview'
    }
    URL.revokeObjectURL = (url) => {
      revoked.push(url)
    }
    globalThis.fetch = (async (url, init) => {
      expect(String(url)).toBe('https://agena.test/api/v1/sessions/46/media?path=image.png')
      expect(String(url)).not.toContain('fixture-token')
      expect(new Headers(init?.headers).get('authorization')).toBe('Bearer fixture-token')
      expect(init?.redirect).toBe('error')
      return new Response('image-bytes', { headers: { 'content-type': 'image/png' } })
    }) as typeof fetch
    const resource = await loadBrowserResource('/api/v1/sessions/46/media?path=image.png')
    expect(resource.url).toStartWith('blob:')
    expect(await created?.text()).toBe('image-bytes')
    resource.release()
    expect(revoked).toEqual([resource.url])
  })

  test('external media remains browser-native and cannot trigger an authenticated fetch', async () => {
    installBrowser()
    globalThis.fetch = (() => {
      throw new Error('External preview must not use authenticated fetch')
    }) as typeof fetch
    const resource = await loadBrowserResource('https://elsewhere.test/video.mp4')
    expect(resource.url).toBe('https://elsewhere.test/video.mp4')
    resource.release()
  })

  test('an aborted or superseded preview never allocates a late object URL', async () => {
    installBrowser()
    const controller = new AbortController()
    let complete: (response: Response) => void = () => {}
    globalThis.fetch = (() =>
      new Promise<Response>((resolve) => {
        complete = resolve
      })) as typeof fetch
    URL.createObjectURL = () => {
      throw new Error('No URL may be allocated after cancellation')
    }
    const loaded = loadBrowserResource('/api/v1/sessions/46/media?path=image.png', controller.signal)
    controller.abort()
    complete(new Response('image-bytes', { headers: { 'content-type': 'image/png' } }))
    await expect(loaded).rejects.toMatchObject({ name: 'AbortError' })
  })

  test('logout aborts a successful response whose private body is still being read', async () => {
    installBrowser()
    const pending = pendingResponseBody()
    let allocations = 0
    URL.createObjectURL = () => {
      allocations += 1
      return 'blob:https://agena.test/stale'
    }
    const loaded = loadBrowserResource('/api/v1/sessions/46/media?path=image.png')
    await pending.started
    clearServerAuthTokenForBaseUrl('https://agena.test')
    expect(pending.signal()?.aborted).toBe(true)
    pending.finish()
    await expect(loaded).rejects.toMatchObject({ name: 'AbortError' })
    expect(allocations).toBe(0)
  })

  for (const action of ['open', 'download'] as const) {
    test(`${action} cannot expose a body completed after a different login`, async () => {
      installBrowser()
      const pending = pendingResponseBody()
      let allocations = 0
      let navigations = 0
      let downloads = 0
      let closed = 0
      URL.createObjectURL = () => {
        allocations += 1
        return 'blob:https://agena.test/stale'
      }
      Object.assign(window, {
        open: () => ({
          opener: null,
          location: {
            replace: () => {
              navigations += 1
            },
          },
          close: () => {
            closed += 1
          },
        }),
      })
      Object.defineProperty(globalThis, 'document', {
        configurable: true,
        value: {
          createElement: () => ({
            click: () => {
              downloads += 1
            },
            remove() {},
          }),
          body: { append() {} },
        },
      })
      const request =
        action === 'open'
          ? openResource('/api/v1/sessions/46/media?path=image.png')
          : downloadResource('/api/v1/sessions/46/media?path=image.png', 'image.png')
      await pending.started
      writeServerAuthTokenForBaseUrl('https://agena.test', 'replacement-login-token')
      expect(pending.signal()?.aborted).toBe(true)
      pending.finish()
      await expect(request).rejects.toMatchObject({ name: 'AbortError' })
      expect([allocations, navigations, downloads]).toEqual([0, 0, 0])
      expect(closed).toBe(action === 'open' ? 1 : 0)
    })
  }

  test('already-created private URLs are revoked exactly once when authentication changes', async () => {
    installBrowser()
    const revoked: string[] = []
    URL.createObjectURL = () => 'blob:https://agena.test/current'
    URL.revokeObjectURL = (url) => {
      revoked.push(url)
    }
    globalThis.fetch = (async () =>
      new Response('private image', { headers: { 'content-type': 'image/png' } })) as typeof fetch
    const loaded = await loadBrowserResource('/api/v1/sessions/46/media?path=image.png')
    writeServerAuthTokenForBaseUrl('https://agena.test', 'fixture-token')
    expect(revoked).toEqual([])
    clearServerAuthTokenForBaseUrl('https://agena.test')
    expect(revoked).toEqual([loaded.url])
    loaded.release()
    expect(revoked).toEqual([loaded.url])
  })

  test('a mounted preview clears synchronously and reloads an unchanged source under the new token', async () => {
    installBrowser()
    let allocations = 0
    const revoked: string[] = []
    const authorizations: Array<string | null> = []
    URL.createObjectURL = () => `blob:https://agena.test/preview-${++allocations}`
    URL.revokeObjectURL = (url) => {
      revoked.push(url)
    }
    globalThis.fetch = (async (_, init) => {
      authorizations.push(new Headers(init?.headers).get('authorization'))
      const response = new Response('')
      response.blob = async () => new Blob(['private image'], { type: 'image/png' })
      return response
    }) as typeof fetch
    const scope = effectScope()
    try {
      const preview = scope.run(() => useAuthenticatedResource('/api/v1/sessions/46/media?path=image.png'))!
      await new Promise<void>((resolve) => setTimeout(resolve, 0))
      expect(preview.url.value).toBe('blob:https://agena.test/preview-1')
      writeServerAuthTokenForBaseUrl('https://agena.test', 'replacement-login-token')
      expect(preview.url.value).toBe('')
      expect(revoked).toEqual(['blob:https://agena.test/preview-1'])
      await nextTick()
      await new Promise<void>((resolve) => setTimeout(resolve, 0))
      expect(preview.url.value).toBe('blob:https://agena.test/preview-2')
      expect(authorizations).toEqual(['Bearer fixture-token', 'Bearer replacement-login-token'])
    } finally {
      scope.stop()
    }
    expect(revoked).toEqual(['blob:https://agena.test/preview-1', 'blob:https://agena.test/preview-2'])
  })

  test('active and unknown documents cannot be navigated in the application blob origin', () => {
    for (const mime of ['text/html', 'image/svg+xml', 'application/xhtml+xml', 'application/octet-stream', ''])
      expect(isPassiveResourceMime(mime)).toBe(false)
    for (const mime of ['image/png', 'audio/wav', 'video/webm', 'application/pdf', 'text/plain;charset=utf-8'])
      expect(isPassiveResourceMime(mime)).toBe(true)
  })
})
