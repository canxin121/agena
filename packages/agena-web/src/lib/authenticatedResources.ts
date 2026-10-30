import { apiResponse } from './api'
import { browserResourceUrl, isAuthenticatedResourceUrl, isPassiveResourceMime } from './resourceUrls'
import { readServerAuthTokenVersion, subscribeServerAuthTokenVersion } from './serverAuthToken'

function assertResourceCurrent(version: number, signal?: AbortSignal) {
  if (signal?.aborted || version !== readServerAuthTokenVersion()) {
    throw new DOMException('The resource request was cancelled or authentication changed', 'AbortError')
  }
}

async function readAuthenticatedResource(url: string, signal?: AbortSignal, version = readServerAuthTokenVersion()) {
  const controller = new AbortController()
  const abort = () => controller.abort()
  signal?.addEventListener('abort', abort, { once: true })
  const unsubscribe = subscribeServerAuthTokenVersion(abort)
  try {
    assertResourceCurrent(version, signal)
    const response = await authenticatedResourceResponse(url, controller.signal)
    assertResourceCurrent(version, controller.signal)
    const blob = await response.blob()
    // Headers may have arrived before logout while a streamed body finishes
    // afterwards. A successful HTTP response alone is not a current read.
    assertResourceCurrent(version, controller.signal)
    return { blob, disposition: response.headers.get('content-disposition') || '', version }
  } catch (error) {
    assertResourceCurrent(version, controller.signal)
    throw error
  } finally {
    unsubscribe()
    signal?.removeEventListener('abort', abort)
  }
}

function createAuthenticatedObjectUrl(blob: Blob, version: number, signal?: AbortSignal) {
  assertResourceCurrent(version, signal)
  const url = URL.createObjectURL(blob)
  let released = false
  let unsubscribe = () => {}
  const release = () => {
    if (released) return
    released = true
    unsubscribe()
    signal?.removeEventListener('abort', release)
    URL.revokeObjectURL(url)
  }
  unsubscribe = subscribeServerAuthTokenVersion(release)
  signal?.addEventListener('abort', release, { once: true })
  try {
    assertResourceCurrent(version, signal)
    return { url, release }
  } catch (error) {
    release()
    throw error
  }
}

async function authenticatedResourceResponse(url: string, signal?: AbortSignal): Promise<Response> {
  if (!isAuthenticatedResourceUrl(url)) throw new Error('Not an Agena resource URL')
  return apiResponse(url, {
    headers: { accept: '*/*' },
    // Protected resources are served directly. Do not follow a redirect with
    // server credentials, even if a plugin produced the URL.
    redirect: 'error',
    signal,
  })
}

export async function authenticatedResourceBlob(url: string, signal?: AbortSignal): Promise<Blob> {
  const resource = await readAuthenticatedResource(url, signal)
  assertResourceCurrent(resource.version, signal)
  return resource.blob
}

/** Fetch only protected resources; public/data/blob URLs stay browser-native. */
export async function loadBrowserResource(
  url: string,
  signal?: AbortSignal,
): Promise<{ url: string; release: () => void }> {
  const destination = browserResourceUrl(url)
  if (!destination || !isAuthenticatedResourceUrl(destination)) return { url: destination, release() {} }
  const resource = await readAuthenticatedResource(destination, signal)
  return createAuthenticatedObjectUrl(resource.blob, resource.version, signal)
}

export async function downloadResource(url: string, filename: string): Promise<void> {
  const destination = browserResourceUrl(url)
  if (!destination) return
  const protectedResource = isAuthenticatedResourceUrl(destination)
  const version = readServerAuthTokenVersion()
  const loaded = await loadBrowserResource(destination)
  try {
    if (protectedResource) assertResourceCurrent(version)
    triggerDownload(loaded.url, filename, !protectedResource)
  } catch (error) {
    loaded.release()
    throw error
  }
  // Let the browser consume the download before revoking its object URL.
  setTimeout(loaded.release, 60_000)
}

function triggerDownload(url: string, filename: string, external = false) {
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.rel = 'noopener noreferrer'
  if (external) link.target = '_blank'
  document.body.append(link)
  link.click()
  link.remove()
}

function responseFilename(disposition: string, fallback: string): string {
  const encoded = /filename\*=UTF-8''([^;]+)/i.exec(disposition)?.[1]
  if (encoded) {
    try {
      return decodeURIComponent(encoded.trim())
    } catch {
      /* use the plain filename */
    }
  }
  return /filename="([^"]+)"/i.exec(disposition)?.[1] || /filename=([^;]+)/i.exec(disposition)?.[1]?.trim() || fallback
}

export async function openResource(url: string, filename = 'attachment'): Promise<void> {
  const destination = browserResourceUrl(url)
  if (!destination) return
  if (!isAuthenticatedResourceUrl(destination)) {
    window.open(destination, '_blank', 'noopener,noreferrer')
    return
  }
  const version = readServerAuthTokenVersion()
  // Reserve the tab during the user's click so asynchronous fetch is not
  // rejected by popup blockers. Never put credentials into this URL.
  const tab = window.open('about:blank', '_blank')
  if (tab) tab.opener = null
  try {
    const { blob, disposition } = await readAuthenticatedResource(destination, undefined, version)
    const resource = createAuthenticatedObjectUrl(blob, version)
    const objectUrl = resource.url
    // Blob navigation loses the response's CSP. Keep the server's download
    // decision and never execute HTML, SVG or unknown binary content here.
    if (/^attachment(?:;|$)/i.test(disposition.trim()) || !isPassiveResourceMime(blob.type)) {
      tab?.close()
      try {
        assertResourceCurrent(version)
        triggerDownload(objectUrl, responseFilename(disposition, filename))
      } catch (error) {
        resource.release()
        throw error
      }
      setTimeout(resource.release, 60_000)
      return
    }
    if (tab) {
      try {
        assertResourceCurrent(version)
        tab.location.replace(objectUrl)
      } catch (error) {
        resource.release()
        throw error
      }
    } else {
      resource.release()
      throw new Error('Allow opening a new tab to view this file.')
    }
    setTimeout(resource.release, 60_000)
  } catch (error) {
    tab?.close()
    throw error
  }
}
