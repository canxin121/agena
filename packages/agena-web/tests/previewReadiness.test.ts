import { describe, expect, test } from 'bun:test'
import {
  probePreviewProxyResponse,
  waitForPreviewProxyReady,
} from '../src/features/workspacePreview/api/previewProxyProbe'

const src = '/api/v1/workbench/preview/s/delayed-app/'
const response = (status: number) => ({
  status,
  ok: status >= 200 && status < 300,
  upstreamResponded: false,
  body: status === 502 ? 'upstream is not listening yet' : '',
  contentType: 'text/plain',
})

describe('preview startup readiness', () => {
  test('a delayed server becomes visible without another run identity or manual refresh', async () => {
    const statuses = [502, 503, 504, 200]
    let attempts = 0
    const result = await waitForPreviewProxyReady(src, new AbortController().signal, {
      timeoutMs: 1_000,
      retryIntervalMs: 1,
      probe: async (path, signal) => {
        expect(path).toBe(src)
        expect(signal.aborted).toBe(false)
        return response(statuses[attempts++]!)
      },
    })
    expect(result.ok).toBe(true)
    expect(attempts).toBe(4)
  })

  test('terminal HTTP errors are returned after one probe', async () => {
    for (const status of [400, 401, 403, 404, 500]) {
      let attempts = 0
      const result = await waitForPreviewProxyReady(src, new AbortController().signal, {
        timeoutMs: 1_000,
        retryIntervalMs: 1,
        probe: async () => {
          attempts++
          return response(status)
        },
      })
      expect(result.status).toBe(status)
      expect(attempts).toBe(1)
    }
  })

  test('app login and error responses are distinguished from control and gateway failures', async () => {
    for (const status of [401, 403, 404, 500]) {
      for (const reached of [true, false]) {
        const result = await waitForPreviewProxyReady(src, new AbortController().signal, {
          probe: (path, signal) =>
            probePreviewProxyResponse(
              path,
              signal,
              async () =>
                new Response('app response', {
                  status,
                  headers: reached ? { 'x-agena-preview-upstream': '1' } : {},
                }),
            ),
        })
        expect(result.status).toBe(status)
        expect(result.upstreamResponded).toBe(reached)
      }
    }
  })

  test('a bounded startup wait preserves an upstream 503 so its app page remains renderable', async () => {
    let attempts = 0
    const result = await waitForPreviewProxyReady(src, new AbortController().signal, {
      timeoutMs: 20,
      retryIntervalMs: 5,
      probe: async () => {
        attempts++
        return { ...response(503), upstreamResponded: true, body: 'Application maintenance page' }
      },
    })
    expect(attempts).toBeGreaterThan(1)
    expect(result.status).toBe(503)
    expect(result.upstreamResponded).toBe(true)
  })

  test('login redirects are ready without following them or forwarding the control token', async () => {
    const opaque = new Response(null)
    Object.defineProperties(opaque, {
      type: { value: 'opaqueredirect' },
      status: { value: 0 },
      ok: { value: false },
    })
    const login = () => new Response(null, { status: 302, headers: { location: 'https://login.example.com/sign-in' } })
    for (const replies of [[login()], [opaque], [new Response(null, { status: 405 }), login()]]) {
      const methods: string[] = []
      const result = await waitForPreviewProxyReady(src, new AbortController().signal, {
        timeoutMs: 1_000,
        retryIntervalMs: 1,
        probe: (path, signal) =>
          probePreviewProxyResponse(path, signal, async (url, init) => {
            expect(url).toBe(src)
            expect(init.redirect).toBe('manual')
            methods.push(init.method!)
            const reply = replies[methods.length - 1]
            if (!reply) throw new Error('The probe unexpectedly retried a login redirect')
            return reply
          }),
      })
      expect(result.ok).toBe(true)
      expect(methods).toEqual(replies.length === 1 ? ['HEAD'] : ['HEAD', 'GET'])
    }
  })

  test('a transient connection failure is retried, but validation failures are not', async () => {
    let attempts = 0
    const result = await waitForPreviewProxyReady(src, new AbortController().signal, {
      timeoutMs: 1_000,
      retryIntervalMs: 1,
      probe: async () => {
        if (++attempts === 1) throw new TypeError('Failed to fetch')
        return response(200)
      },
    })
    expect(result.ok).toBe(true)
    expect(attempts).toBe(2)
    attempts = 0
    await expect(
      waitForPreviewProxyReady(src, new AbortController().signal, {
        probe: async () => {
          attempts++
          throw new Error('invalid request')
        },
      }),
    ).rejects.toThrow('invalid request')
    expect(attempts).toBe(1)
    await expect(
      waitForPreviewProxyReady('https://p-other.preview.example.com/', new AbortController().signal, {
        probe: async () => {
          attempts++
          return response(200)
        },
      }),
    ).rejects.toThrow('control API proxy path')
    expect(attempts).toBe(1)
  })

  test('selection cancellation aborts an in-flight probe and leaves no scheduled retry', async () => {
    const controller = new AbortController()
    let attempts = 0
    let probeAborted = false
    const ready = waitForPreviewProxyReady(src, controller.signal, {
      timeoutMs: 1_000,
      retryIntervalMs: 1,
      probe: async (_path, signal) => {
        attempts++
        return new Promise<ReturnType<typeof response>>((_, reject) => {
          signal.addEventListener(
            'abort',
            () => {
              probeAborted = true
              reject(signal.reason)
            },
            { once: true },
          )
        })
      },
    })
    controller.abort(new DOMException('selection changed', 'AbortError'))
    await expect(ready).rejects.toThrow('selection changed')
    await new Promise((resolve) => setTimeout(resolve, 10))
    expect(probeAborted).toBe(true)
    expect(attempts).toBe(1)
  })

  test('hiding during backoff cancels the retry timer', async () => {
    const controller = new AbortController()
    let attempts = 0
    const ready = waitForPreviewProxyReady(src, controller.signal, {
      timeoutMs: 1_000,
      retryIntervalMs: 30,
      probe: async () => {
        attempts++
        return response(502)
      },
    })
    // Let the first response reach its retry delay before hiding the panel.
    await Promise.resolve()
    controller.abort(new DOMException('preview hidden', 'AbortError'))
    await expect(ready).rejects.toThrow('preview hidden')
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(attempts).toBe(1)
  })

  test('a server that stays unavailable has a bounded wait and retains its diagnostic', async () => {
    let attempts = 0
    const result = await waitForPreviewProxyReady(src, new AbortController().signal, {
      timeoutMs: 20,
      retryIntervalMs: 5,
      probe: async () => {
        attempts++
        return response(502)
      },
    })
    expect(result.status).toBe(502)
    expect(result.body).toBe('upstream is not listening yet')
    const completedAttempts = attempts
    await new Promise((resolve) => setTimeout(resolve, 15))
    expect(attempts).toBe(completedAttempts)
  })
})
