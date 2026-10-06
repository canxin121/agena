import { apiResponse, apiUrl } from './api'
import { readUiAuthTokenVersion } from './uiAuthToken'
import { canReuseResource, captureResourceObservation, noteResourceVersion } from './resourceSync'
import { createSharedRead, limitBackgroundReads } from './backgroundReads'

type Cached = { token: string; etag: string; value: unknown; bytes: number; generation: number }
type Flight = { read: ReturnType<typeof createSharedRead<unknown>>; generation: number; requestedGeneration: number }
const cache = new Map<string, Cached>()
const flights = new Map<string, Flight>()
let cacheBytes = 0
let cacheScope = ''
let forcedSequence = 0

function forget(key: string) {
  const previous = cache.get(key)
  if (previous) cacheBytes -= previous.bytes
  cache.delete(key)
}

function remember(key: string, entry: Cached) {
  forget(key)
  cache.set(key, entry)
  cacheBytes += entry.bytes
  while (cache.size > 80 || cacheBytes > 8 * 1024 * 1024) {
    const oldest = cache.keys().next().value!
    cacheBytes -= cache.get(oldest)!.bytes
    cache.delete(oldest)
  }
}

/** Bounded representations, shared reads, and independent cancellation.
 * A mutation's forced read cannot join a request sent before that mutation. */
export async function conditionalJson<T>(resource: string, url: string, init?: RequestInit, force = false, requiredGeneration?: number): Promise<T> {
  init?.signal?.throwIfAborted()
  const authVersion = readUiAuthTokenVersion()
  const resolvedUrl = apiUrl(url)
  const nextScope = `${authVersion}:${apiUrl('/')}`
  if (nextScope !== cacheScope) {
    cacheScope = nextScope
    for (const flight of flights.values()) flight.read.abort()
    flights.clear()
    cache.clear()
    cacheBytes = 0
  }
  const key = `${resource}:${resolvedUrl}`
  if (force && requiredGeneration === undefined) requiredGeneration = ++forcedSequence
  const previous = cache.get(key)
  if (previous && (!force || previous.generation >= requiredGeneration!) && canReuseResource(resource, previous.token)) {
    cache.delete(key)
    cache.set(key, previous)
    return previous.value as T
  }
  let flight = flights.get(key)
  if (flight?.read.signal.aborted) {
    flights.delete(key)
    flight = undefined
  }
  if (flight) {
    if (force) flight.requestedGeneration = Math.max(flight.requestedGeneration, requiredGeneration!)
    const value = await flight.read.join(init?.signal)
    if (force && flight.generation < requiredGeneration!) return conditionalJson<T>(resource, url, init, true, requiredGeneration)
    return value as T
  }
  const current: Flight = { read: undefined!, generation: forcedSequence, requestedGeneration: forcedSequence }
  current.read = createSharedRead(async (signal) =>
    limitBackgroundReads(async () => {
      // Several callers queued before dispatch can share this latest request.
      current.generation = current.requestedGeneration
      signal.throwIfAborted()
      const observation = captureResourceObservation(resource)
      const cached = cache.get(key)
      const headers = new Headers(init?.headers)
      if (cached) headers.set('if-none-match', cached.etag)
      const response = await apiResponse(resolvedUrl, {
        ...init,
        headers,
        cache: 'no-store',
        signal: AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
      })
      signal.throwIfAborted()
      if (authVersion !== readUiAuthTokenVersion() || resolvedUrl !== apiUrl(url))
        throw new Error('Backend or authentication changed during read')
      if (response.status === 304) {
        if (!cached) throw new Error('A 304 response has no cached representation')
        if (noteResourceVersion(resource, cached.token, observation)) remember(key, { ...cached, generation: current.generation })
        return cached.value
      }
      const text = await response.text()
      signal.throwIfAborted()
      if (authVersion !== readUiAuthTokenVersion() || resolvedUrl !== apiUrl(url))
        throw new Error('Backend or authentication changed during read')
      const value = JSON.parse(text) as T
      const etag = response.headers.get('etag') ?? ''
      const token = /^(?:W\/)?"(.+)"$/.exec(etag)?.[1] ?? ''
      const accepted = token ? noteResourceVersion(resource, token, observation) : false
      const bytes = text.length * 2
      if (accepted && bytes <= 2 * 1024 * 1024) remember(key, { token, etag, value, bytes, generation: current.generation })
      else if (accepted || !token) forget(key)
      // A concurrent event leaves one throttled trailing read with the
      // subscriber. Keep this useful snapshot without caching an old token.
      return value
    }, signal),
  )
  flights.set(key, current)
  void current.read.promise
    .finally(() => {
      if (flights.get(key) === current) flights.delete(key)
    })
    .catch(() => {})
  return (await current.read.join(init?.signal)) as T
}
