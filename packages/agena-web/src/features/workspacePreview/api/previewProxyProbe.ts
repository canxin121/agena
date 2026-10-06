import { apiUrl } from '../../../lib/api'
import { buildActiveUiAuthHeaders } from '../../../lib/uiAuthToken'

/** Check headers first, so an iframe does not download and rewrite its HTML twice. */
export async function probePreviewProxyResponse(src: string, signal: AbortSignal) {
  const authHeaders = buildActiveUiAuthHeaders()
  const headers = {
    accept: 'text/html,application/json;q=0.9,*/*;q=0.8',
    ...authHeaders,
  }
  const requestSignal = AbortSignal.any([signal, AbortSignal.timeout(10_000)])
  const request = (method: string) =>
    fetch(apiUrl(src), {
      method,
      headers,
      signal: requestSignal,
      credentials: authHeaders.authorization ? 'omit' : 'include',
    })
  let response = await request('HEAD')
  if (!response.ok) {
    await response.body?.cancel()
    // Some development servers do not implement HEAD. GET also supplies the
    // useful proxy diagnostic that an empty HEAD error cannot contain.
    response = await request('GET')
  }
  if (response.ok) {
    await response.body?.cancel()
    return { status: response.status, ok: true, body: '', contentType: '' }
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
  return { status: response.status, ok: false, body, contentType: response.headers.get('content-type') || '' }
}
