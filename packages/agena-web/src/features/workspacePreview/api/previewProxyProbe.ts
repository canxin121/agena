import { apiUrl } from '../../../lib/api'
import { buildActiveServerAuthHeaders } from '../../../lib/serverAuthToken'
import { normalizePreviewProxyBasePath } from '../model/previewUrl'

function requireControlPreviewPath(src: string) {
  if (!normalizePreviewProxyBasePath(src)) throw new Error('Preview diagnostics require a control API proxy path.')
}

type PreviewFetch = (url: string, init: RequestInit) => Promise<Response>

function hasResponded(response: Response): boolean {
  // Browsers expose manual redirects as opaque responses with status 0. A
  // login redirect proves the upstream is listening; never follow it with the
  // control token just to establish readiness.
  return response.ok || response.type === 'opaqueredirect' || (response.status >= 300 && response.status < 400)
}

/** Check headers first, so an iframe does not download and rewrite its HTML twice. */
export async function probePreviewProxyResponse(src: string, signal: AbortSignal, fetchRequest: PreviewFetch = fetch) {
  // This helper is only for the control API. An isolated preview URL must never receive its bearer token.
  requireControlPreviewPath(src)
  const authHeaders = buildActiveServerAuthHeaders()
  const headers = {
    accept: 'text/html,application/json;q=0.9,*/*;q=0.8',
    ...authHeaders,
  }
  const requestSignal = AbortSignal.any([signal, AbortSignal.timeout(10_000)])
  const request = (method: string) =>
    fetchRequest(apiUrl(src), {
      method,
      headers,
      signal: requestSignal,
      redirect: 'manual',
      cache: 'no-store',
      credentials: authHeaders.authorization ? 'omit' : 'include',
    })
  let response = await request('HEAD')
  if (!hasResponded(response)) {
    await response.body?.cancel()
    // Some development servers do not implement HEAD. GET also supplies the
    // useful proxy diagnostic that an empty HEAD error cannot contain.
    response = await request('GET')
  }
  if (hasResponded(response)) {
    await response.body?.cancel()
    return {
      status: response.status,
      ok: true,
      upstreamResponded: response.headers.get('x-agena-preview-upstream') === '1',
      body: '',
      contentType: '',
    }
  }
  const reader = response.body?.getReader()
  const decoder = new TextDecoder()
  let body = ''
  let remaining = 16_384
  try {
    while (reader && remaining > 0) {
      const { done, value } = await reader.read()
      if (done) break
      const chunk = value.subarray(0, remaining)
      body += decoder.decode(chunk, { stream: true })
      remaining -= chunk.length
    }
    body += decoder.decode()
  } finally {
    await reader?.cancel()
  }
  return {
    status: response.status,
    ok: false,
    upstreamResponded: response.headers.get('x-agena-preview-upstream') === '1',
    body,
    contentType: response.headers.get('content-type') || '',
  }
}

export type PreviewProxyResponse = Awaited<ReturnType<typeof probePreviewProxyResponse>>
type PreviewReadinessOptions = {
  timeoutMs?: number
  retryIntervalMs?: number
  probe?: typeof probePreviewProxyResponse
}

function waitForRetry(delayMs: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, delayMs)
    function onAbort() {
      clearTimeout(timer)
      signal.removeEventListener('abort', onAbort)
      reject(signal.reason)
    }
    signal.addEventListener('abort', onAbort, { once: true })
    if (signal.aborted) onAbort()
  })
}

/** A spawned process may need time to listen; preserve one bounded, cancellable readiness wait. */
export async function waitForPreviewProxyReady(
  src: string,
  signal: AbortSignal,
  options: PreviewReadinessOptions = {},
): Promise<PreviewProxyResponse> {
  requireControlPreviewPath(src)
  signal.throwIfAborted()
  const controller = new AbortController()
  const onAbort = () => controller.abort(signal.reason)
  signal.addEventListener('abort', onAbort, { once: true })
  const timeoutMs = Math.max(1, Math.min(options.timeoutMs ?? 18_000, 18_000))
  const timer = setTimeout(() => {
    controller.abort(new DOMException('Preview did not become ready before the startup deadline.', 'TimeoutError'))
  }, timeoutMs)
  const retryIntervalMs = Math.max(1, options.retryIntervalMs ?? 300)
  const probe = options.probe ?? probePreviewProxyResponse
  let lastResponse: PreviewProxyResponse | undefined
  try {
    while (true) {
      controller.signal.throwIfAborted()
      try {
        const response = await probe(src, controller.signal)
        controller.signal.throwIfAborted()
        if (response.ok || ![502, 503, 504].includes(response.status)) return response
        lastResponse = response
      } catch (error) {
        controller.signal.throwIfAborted()
        // Fetch reports short-lived connection failures as TypeError. Do not
        // retry validation, authentication, or other application exceptions.
        const transient =
          error instanceof TypeError ||
          (error instanceof DOMException && ['NetworkError', 'TimeoutError'].includes(error.name))
        if (!transient) throw error
      }
      await waitForRetry(retryIntervalMs, controller.signal)
    }
  } catch (error) {
    if (signal.aborted) throw signal.reason
    // Preserve the upstream diagnostic after the bounded startup window.
    if (controller.signal.aborted && lastResponse) return lastResponse
    throw error
  } finally {
    clearTimeout(timer)
    signal.removeEventListener('abort', onAbort)
  }
}
