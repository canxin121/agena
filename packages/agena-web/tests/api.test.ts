import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { ApiError, apiBlob, apiFetch, apiResponse, apiText } from '../src/lib/api'
import { OC_AUTH_REQUIRED_EVENT, readAuthRequiredFromStorage, type AuthRequiredDetail } from '../src/lib/authEvents'
import { readServerAuthTokenVersion, writeServerAuthTokenForBaseUrl } from '../src/lib/serverAuthToken'

const originalFetch = globalThis.fetch
const browserGlobals = ['window', 'document', 'localStorage', 'sessionStorage'] as const
const originalGlobals = new Map(browserGlobals.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]))
const origin = 'https://agena.test'
let authEvents: AuthRequiredDetail[] = []

function memoryStorage() {
  const values = new Map<string, string>()
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  }
}

beforeEach(() => {
  authEvents = []
  const target = Object.assign(new EventTarget(), { location: { origin } })
  target.addEventListener(OC_AUTH_REQUIRED_EVENT, (event) => {
    authEvents.push((event as CustomEvent<AuthRequiredDetail>).detail)
  })
  const values = { window: target, document: undefined, localStorage: memoryStorage(), sessionStorage: memoryStorage() }
  for (const name of browserGlobals) {
    Object.defineProperty(globalThis, name, { configurable: true, value: values[name] })
  }
  writeServerAuthTokenForBaseUrl(origin, 'request-token')
})

afterEach(() => {
  globalThis.fetch = originalFetch
  for (const name of browserGlobals) {
    const descriptor = originalGlobals.get(name)
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

async function requestError(request: () => Promise<unknown>): Promise<ApiError> {
  try {
    await request()
  } catch (error) {
    if (error instanceof ApiError) return error
    throw error
  }
  throw new Error('Expected an ApiError')
}

const readers = [
  { name: 'apiResponse', request: apiResponse },
  { name: 'apiText', request: apiText },
  { name: 'apiBlob', request: apiBlob },
] as const

describe('shared HTTP error boundary', () => {
  for (const { name, request } of readers) {
    test(`${name} preserves both public error contracts and endpoint diagnostics`, async () => {
      const fixtures = [
        {
          body: {
            error: ' Git failed ',
            code: ' git.exit ',
            hint: ' Inspect stderr ',
            stdout: 'partial result',
            stderr: 'git diagnostic',
            path: '/repo/file',
            promptMode: 'confirm',
          },
          message: 'Git failed\nInspect stderr',
          code: 'git.exit',
          hint: 'Inspect stderr',
          contentType: 'text/plain',
        },
        {
          body: { problem: { code: 'session.denied', user: { fallback: ' Session denied ' }, hint: ' Ask owner ' } },
          message: 'Session denied\nAsk owner',
          code: 'session.denied',
          hint: 'Ask owner',
          contentType: 'application/json',
        },
      ]
      for (const fixture of fixtures) {
        const bodyText = JSON.stringify(fixture.body)
        const response = new Response(bodyText, { status: 403, headers: { 'content-type': fixture.contentType } })
        const readText = response.text.bind(response)
        let reads = 0
        response.text = async () => {
          reads += 1
          return await readText()
        }
        globalThis.fetch = (async () => response) as typeof fetch
        const error = await requestError(() => request('/api/v1/operation'))
        expect(error).toMatchObject({
          message: fixture.message,
          status: 403,
          bodyText,
          bodyJson: fixture.body,
          code: fixture.code,
          hint: fixture.hint,
        })
        expect(reads).toBe(1)
        expect(authEvents).toEqual([])
      }
    })

    test(`${name} retains useful fallback errors for malformed, empty, or unreadable bodies`, async () => {
      for (const fixture of [
        { body: '{malformed', expected: '{malformed', unreadable: false },
        { body: 'gateway unavailable', expected: 'gateway unavailable', unreadable: false },
        { body: '', expected: 'Request failed (503)', unreadable: false },
        { body: 'unreadable', expected: 'Request failed (503)', unreadable: true },
      ]) {
        const response = new Response(fixture.body, { status: 503, headers: { 'content-type': 'application/json' } })
        if (fixture.unreadable)
          response.text = async () => {
            throw new Error('body stream failed')
          }
        globalThis.fetch = (async () => response) as typeof fetch
        const error = await requestError(() => request('/api/v1/operation'))
        expect(error.message).toBe(fixture.expected)
        expect(error.bodyText).toBe(fixture.unreadable ? '' : fixture.body)
        expect(error.bodyJson).toBeUndefined()
        expect(error.code).toBeUndefined()
        expect(error.hint).toBeUndefined()
      }
    })

    test(`${name} reports the token version that started an auth-required request`, async () => {
      const initialVersion = readServerAuthTokenVersion()
      const url = '/api/v1/operation?scope=session'
      globalThis.fetch = (async (_, init) => {
        expect(new Headers(init?.headers).get('authorization')).toBe('Bearer request-token')
        writeServerAuthTokenForBaseUrl(origin, 'replacement-token')
        return new Response(JSON.stringify({ problem: { code: 'auth.required', user: { fallback: 'Sign in' } } }), {
          status: 401,
        })
      }) as typeof fetch
      await requestError(() => request(url))
      expect(readServerAuthTokenVersion()).toBeGreaterThan(initialVersion)
      expect(authEvents).toEqual([
        { message: 'Sign in', status: 401, code: 'auth.required', url, authTokenVersion: initialVersion },
      ])
      expect(readAuthRequiredFromStorage()?.detail).toEqual(authEvents[0])

      authEvents = []
      for (const [status, code] of [
        [401, 'permission.denied'],
        [403, 'auth.required'],
      ] as const) {
        globalThis.fetch = (async () =>
          new Response(JSON.stringify({ error: 'Denied', code }), { status })) as typeof fetch
        await requestError(() => request(url))
      }
      expect(authEvents).toEqual([])
    })
  }

  test('successful responses retain each reader’s body-consumption policy', async () => {
    const responses: Response[] = []
    globalThis.fetch = (async () => {
      const response = new Response('successful body')
      responses.push(response)
      return response
    }) as typeof fetch
    const response = await apiResponse('/api/v1/operation')
    expect(response).toBe(responses[0])
    expect(response.bodyUsed).toBe(false)
    expect(await response.text()).toBe('successful body')
    expect(await apiText('/api/v1/operation')).toBe('successful body')
    expect(responses[1]?.bodyUsed).toBe(true)
    expect(await (await apiBlob('/api/v1/operation')).text()).toBe('successful body')
    expect(responses[2]?.bodyUsed).toBe(true)
  })

  test('only apiResponse accepts an unread 304 for conditional requests', async () => {
    globalThis.fetch = (async () => new Response(null, { status: 304 })) as typeof fetch
    const response = await apiResponse('/api/v1/operation')
    expect(response.status).toBe(304)
    expect(response.bodyUsed).toBe(false)
    for (const request of [apiText, apiBlob]) {
      const error = await requestError(() => request('/api/v1/operation'))
      expect(error).toMatchObject({ status: 304, message: 'Request failed (304)', bodyText: '' })
    }
    expect(authEvents).toEqual([])
  })
})

describe('shared HTTP request boundary', () => {
  const requests = [{ name: 'apiFetch', request: apiFetch }, ...readers]

  for (const { name, request } of requests) {
    test(`${name} preserves all HeadersInit forms and caller request options`, async () => {
      const entries: [string, string][] = [
        ['Authorization', 'Bearer caller-token'],
        ['Accept', 'text/plain'],
        ['Content-Type', 'text/plain'],
        ['X-Custom', 'kept'],
      ]
      for (const headers of [Object.fromEntries(entries), entries, new Headers(entries)]) {
        const before = Array.from(new Headers(headers).entries())
        const controller = new AbortController()
        globalThis.fetch = (async (url, init) => {
          expect(String(url)).toBe(`${origin}/api/v1/operation`)
          const sent = new Headers(init?.headers)
          expect(sent.get('authorization')).toBe('Bearer caller-token')
          expect(sent.get('accept')).toBe('text/plain')
          expect(sent.get('content-type')).toBe('text/plain')
          expect(sent.get('x-custom')).toBe('kept')
          expect(init?.method).toBe('POST')
          expect(init?.body).toBe('command')
          expect(init?.signal).toBe(controller.signal)
          expect(init?.credentials).toBe('same-origin')
          expect(init?.cache).toBe('no-store')
          expect(init?.redirect).toBe('error')
          return new Response('ok')
        }) as typeof fetch
        await request('/api/v1/operation', {
          method: 'POST',
          headers,
          body: 'command',
          signal: controller.signal,
          credentials: 'same-origin',
          cache: 'no-store',
          redirect: 'error',
        })
        expect(Array.from(new Headers(headers).entries())).toEqual(before)
      }
    })
  }

  test('reader defaults apply only to absent headers and leave caller Headers unchanged', async () => {
    for (const { name, request } of requests) {
      const defaults = name === 'apiFetch' || name === 'apiResponse' ? 'application/json' : null
      for (const [headers, expectedAuth, expectedAccept] of [
        [new Headers(), 'Bearer request-token', defaults],
        [new Headers({ authorization: '', accept: '' }), '', ''],
      ] as const) {
        const before = Array.from(headers.entries())
        globalThis.fetch = (async (_, init) => {
          const sent = new Headers(init?.headers)
          expect(sent.get('authorization')).toBe(expectedAuth)
          expect(sent.get('accept')).toBe(expectedAccept)
          return new Response('ok')
        }) as typeof fetch
        await request('/api/v1/operation', { headers })
        expect(Array.from(headers.entries())).toEqual(before)
      }
    }
  })

  test('credential defaults retain backend trust, token presence, and explicit overrides', async () => {
    const cases = [
      {
        url: '/api/v1/operation',
        token: 'request-token',
        expectedAuth: 'Bearer request-token',
        expectedCredentials: 'omit',
      },
      { url: '/api/v1/operation', token: '', expectedAuth: null, expectedCredentials: 'include' },
      { url: 'https://outside.test/file', token: 'request-token', expectedAuth: null, expectedCredentials: 'omit' },
      {
        url: '/api/v1/operation',
        token: 'request-token',
        credentials: 'same-origin',
        expectedAuth: 'Bearer request-token',
        expectedCredentials: 'same-origin',
      },
    ] as const
    for (const { request } of requests) {
      for (const fixture of cases) {
        writeServerAuthTokenForBaseUrl(origin, fixture.token)
        globalThis.fetch = (async (_, init) => {
          expect(new Headers(init?.headers).get('authorization')).toBe(fixture.expectedAuth)
          expect(init?.credentials).toBe(fixture.expectedCredentials)
          return new Response('ok')
        }) as typeof fetch
        await request(fixture.url, 'credentials' in fixture ? { credentials: fixture.credentials } : undefined)
      }
    }
  })
})
