import { i18n } from '@/i18n'
import { apiResponse, apiUrl } from './api'
import { readServerAuthTokenVersion } from './serverAuthToken'
import { canReuseResource, captureResourceObservation, noteResourceVersion } from './resourceSync'
import { createSharedRead, limitBackgroundReads } from './backgroundReads'

type Cached = { token: string; etag: string; value: unknown; bytes: number; generation: number }
type Observation = ReturnType<typeof captureResourceObservation>
type Observed<T> = { value: T; observation: Observation }
type Flight = {
  read: ReturnType<typeof createSharedRead<Observed<unknown>>>
  generation: number
  requestedGeneration: number
}
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
export async function conditionalJson<T>(
  resource: string,
  url: string,
  init?: RequestInit,
  force = false,
  requiredGeneration?: number,
): Promise<T> {
  return (await conditionalJsonObserved<T>(resource, url, init, force, requiredGeneration)).value
}

/** Associate a locally retained value with the token of its actual response,
 * including an older response that raced a newer SSE announcement. */
export async function conditionalJsonObserved<T>(
  resource: string,
  url: string,
  init?: RequestInit,
  force = false,
  requiredGeneration?: number,
  forceFresh = false,
): Promise<Observed<T>> {
  init?.signal?.throwIfAborted()
  const authVersion = readServerAuthTokenVersion()
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
  if (
    previous &&
    !forceFresh &&
    (!force || previous.generation >= requiredGeneration!) &&
    canReuseResource(resource, previous.token)
  ) {
    cache.delete(key)
    cache.set(key, previous)
    return {
      value: previous.value as T,
      observation: { ...captureResourceObservation(resource), token: previous.token },
    }
  }
  let flight = flights.get(key)
  if (flight?.read.signal.aborted) {
    flights.delete(key)
    flight = undefined
  }
  if (flight) {
    if (force) flight.requestedGeneration = Math.max(flight.requestedGeneration, requiredGeneration!)
    const value = await flight.read.join(init?.signal)
    if (force && flight.generation < requiredGeneration!)
      return conditionalJsonObserved<T>(resource, url, init, true, requiredGeneration, forceFresh)
    return value as Observed<T>
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
      // Some callers have evidence that the retained representation is
      // incomplete and need a fresh body even if its validator is unchanged.
      if (cached && !forceFresh) headers.set('if-none-match', cached.etag)
      const response = await apiResponse(resolvedUrl, {
        ...init,
        headers,
        cache: 'no-store',
        signal: AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
      })
      signal.throwIfAborted()
      if (authVersion !== readServerAuthTokenVersion() || resolvedUrl !== apiUrl(url))
        throw new Error(i18n.global.t('errors.content.backendOrAuthChanged'))
      if (response.status === 304) {
        if (!cached) throw new Error(i18n.global.t('errors.content.notModifiedWithoutCache'))
        // A resource may advance while this particular representation stays
        // identical (another file changed). A 304 validates the retained body
        // at the response's resource revision, not just its old cache token.
        const token = response.headers.get('x-resource-revision') || cached.token
        const etag = response.headers.get('etag') || cached.etag
        if (noteResourceVersion(resource, token, observation))
          remember(key, { ...cached, token, etag, generation: current.generation })
        const currentToken = response.headers.get('x-resource-current-revision')
        if (currentToken && currentToken !== token) noteResourceVersion(resource, currentToken, observation)
        return { value: cached.value, observation: { ...observation, token } }
      }
      const text = await response.text()
      signal.throwIfAborted()
      if (authVersion !== readServerAuthTokenVersion() || resolvedUrl !== apiUrl(url))
        throw new Error(i18n.global.t('errors.content.backendOrAuthChanged'))
      const value = JSON.parse(text) as T
      const etag = response.headers.get('etag') ?? ''
      const token = response.headers.get('x-resource-revision') || /^(?:W\/)?"(.+)"$/.exec(etag)?.[1] || ''
      const accepted = token ? noteResourceVersion(resource, token, observation) : false
      const bytes = text.length * 2
      if (accepted && bytes <= 2 * 1024 * 1024)
        remember(key, { token, etag, value, bytes, generation: current.generation })
      else if (accepted || !token) forget(key)
      const currentToken = response.headers.get('x-resource-current-revision')
      if (currentToken && currentToken !== token) noteResourceVersion(resource, currentToken, observation)
      // A concurrent event leaves one throttled trailing read with the
      // subscriber. Keep this useful snapshot without caching an old token.
      return { value, observation: { ...observation, token: token || undefined } }
    }, signal),
  )
  flights.set(key, current)
  void current.read.promise
    .finally(() => {
      if (flights.get(key) === current) flights.delete(key)
    })
    .catch(() => {})
  return (await current.read.join(init?.signal)) as Observed<T>
}
