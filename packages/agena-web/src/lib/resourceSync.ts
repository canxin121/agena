import { apiJson } from './api'
import { createSharedRead, isDocumentVisible } from './backgroundReads'
import { readActiveBackendBaseUrl } from './backend'
import { readUiAuthTokenVersion } from './uiAuthToken'
import { createRevalidator } from './revalidation'
import type { SseEvent } from './sse'

// All visible consumers share one small revision heartbeat. Actual bodies are
// fetched only after a token changes; SSE wakes this check promptly.
const listeners = new Map<string, Set<(event?: SseEvent) => void>>()
const versions = new Map<string, string>()
const dirty = new Set<string>()
const checkedAt = new Map<string, number>()
const invalidation = new Map<string, number>()
const retiredEpochs = new Set<string>()
let serverEpoch: string | undefined
let runtimeUsers = 0
let scope = ''
let scopeGeneration = 0
let probeAllowedAt = 0
let probeFailures = 0
let probeError: unknown
let checkFlight: { keys: Set<string>; scope: number; read: ReturnType<typeof createSharedRead<void>> } | null = null

function ensureScope() {
  const next = `${readActiveBackendBaseUrl()}:${readUiAuthTokenVersion()}`
  if (scope === next) return scopeGeneration
  scope = next
  scopeGeneration++
  checkFlight?.read.abort()
  checkFlight = null
  versions.clear()
  dirty.clear()
  checkedAt.clear()
  invalidation.clear()
  retiredEpochs.clear()
  serverEpoch = undefined
  probeAllowedAt = 0
  probeFailures = 0
  probeError = undefined
  return scopeGeneration
}

function prune() {
  // Inactive resource metadata has the same finite lifetime as body caches.
  // Mounted subscriptions remain until their last consumer releases them.
  if (versions.size <= 512) return
  for (const key of versions.keys()) {
    if (listeners.has(key)) continue
    versions.delete(key)
    dirty.delete(key)
    checkedAt.delete(key)
    invalidation.delete(key)
    if (versions.size <= 512) break
  }
}

export function captureResourceObservation(key: string) {
  return { scope: ensureScope(), epoch: serverEpoch, token: versions.get(key), generation: invalidation.get(key) ?? 0 }
}

export function resourceUpdatedAtMs(key: string): number | null {
  ensureScope()
  const token = versions.get(key)
  return token ? Number(token.slice(token.lastIndexOf(':') + 1)) : null
}

function notify(key: string, event?: SseEvent) {
  for (const callback of listeners.get(key) ?? []) callback(event)
}

export function noteResourceVersion(
  key: string,
  token: string,
  observation?: ReturnType<typeof captureResourceObservation>,
  event?: SseEvent,
): boolean {
  if (observation && observation.scope !== ensureScope()) return false
  ensureScope()
  const separator = token.lastIndexOf(':')
  const epoch = token.slice(0, separator)
  const timestamp = Number(token.slice(separator + 1))
  if (separator < 1 || !Number.isSafeInteger(timestamp) || retiredEpochs.has(epoch)) return false
  const previous = versions.get(key)
  const accept = () => {
    checkedAt.set(key, Date.now())
    if (!observation || observation.generation === (invalidation.get(key) ?? 0)) dirty.delete(key)
  }
  if (previous === token) {
    // A repeated announcement is not evidence that a pending local mutation
    // was observed by the server. Only a read can clear that invalidation.
    checkedAt.set(key, Date.now())
    if (observation) accept()
    return true
  }
  // An older response cannot roll the revision index back while a read races
  // a stream event or a batch check. The subscriber retains a trailing read.
  if (
    previous &&
    previous.slice(0, previous.lastIndexOf(':')) === epoch &&
    Number(previous.slice(previous.lastIndexOf(':') + 1)) > timestamp
  ) {
    return false
  }
  if (serverEpoch && serverEpoch !== epoch) {
    // A restart applies to every resource, including keys not involved in
    // its first response. Retire the epoch globally before accepting bodies.
    if (observation && observation.epoch !== serverEpoch) return false
    retiredEpochs.add(serverEpoch)
    if (retiredEpochs.size > 16) retiredEpochs.delete(retiredEpochs.values().next().value!)
    const affected = new Set([...versions.keys(), ...listeners.keys()])
    versions.clear()
    checkedAt.clear()
    for (const affectedKey of affected) {
      dirty.add(affectedKey)
      invalidation.set(affectedKey, (invalidation.get(affectedKey) ?? 0) + 1)
      if (affectedKey !== key) notify(affectedKey)
    }
  }
  serverEpoch = epoch
  versions.set(key, token)
  if (event) invalidation.set(key, (invalidation.get(key) ?? 0) + 1)
  accept()
  prune()
  if (previous || !observation) notify(key, event)
  return true
}

export function canReuseResource(key: string, token: string) {
  ensureScope()
  return !dirty.has(key) && versions.get(key) === token
}

export async function checkResourceVersions(keys: Iterable<string>, signal?: AbortSignal): Promise<void> {
  signal?.throwIfAborted()
  const currentScope = ensureScope()
  const requested = new Set(
    [...keys].filter((key) => dirty.has(key) || !versions.has(key) || Date.now() - (checkedAt.get(key) ?? 0) >= 30_000),
  )
  if (!requested.size) return
  if (checkFlight?.read.signal.aborted) checkFlight = null
  if (checkFlight) {
    const flight = checkFlight
    if (flight.read.isAborted()) {
      checkFlight = null
      return checkResourceVersions(requested, signal)
    }
    await flight.read.join(signal)
    signal?.throwIfAborted()
    if (currentScope !== ensureScope()) throw new Error('Backend changed during revision check')
    const missing = [...requested].filter((key) => !flight.keys.has(key) || dirty.has(key))
    if (missing.length) await checkResourceVersions(missing, signal)
    return
  }
  if (probeFailures && Date.now() < probeAllowedAt) throw probeError
  const read = createSharedRead(async (sharedSignal) => {
    const all = [...requested]
    for (let offset = 0; offset < all.length; offset += 128) {
      const batch = all.slice(offset, offset + 128)
      const owners = new Map(batch.map((key) => [key, invalidation.get(key) ?? 0]))
      const observations = new Map(batch.map((key) => [key, captureResourceObservation(key)]))
      const lastChecked = Math.max(0, ...batch.map((key) => checkedAt.get(key) ?? 0))
      const delay = Math.max(500 - (Date.now() - lastChecked), probeAllowedAt - Date.now())
      sharedSignal.throwIfAborted()
      if (delay > 0)
        await new Promise<void>((resolve, reject) => {
          const abort = () => {
            clearTimeout(timer)
            reject(sharedSignal.reason)
          }
          const timer = setTimeout(() => {
            sharedSignal.removeEventListener('abort', abort)
            resolve()
          }, delay)
          sharedSignal.addEventListener('abort', abort, { once: true })
        })
      const query = new URLSearchParams({ resources: JSON.stringify(batch) })
      const result = await apiJson<Record<string, string>>(`/api/v1/changes/revisions?${query}`, {
        signal: AbortSignal.any([sharedSignal, AbortSignal.timeout(15_000)]),
        cache: 'no-store',
      })
      if (currentScope !== ensureScope()) throw new Error('Backend changed during revision check')
      sharedSignal.throwIfAborted()
      for (const key of batch) {
        const token = result[key]
        if (typeof token !== 'string') throw new Error('Missing resource revision')
        if (!noteResourceVersion(key, token, observations.get(key))) continue
        checkedAt.set(key, Date.now())
        if (owners.get(key) === (invalidation.get(key) ?? 0)) dirty.delete(key)
      }
      probeAllowedAt = Date.now() + 500
      probeFailures = 0
      probeError = undefined
    }
  })
  const current = { keys: requested, scope: currentScope, read }
  checkFlight = current
  // Clear on network completion, not on any one consumer's cancellation.
  void read.promise
    .catch((error) => {
      if (!read.signal.aborted && currentScope === ensureScope()) {
        probeError = error
        probeFailures = Math.min(6, probeFailures + 1)
        probeAllowedAt = Date.now() + Math.min(60_000, 5000 * 2 ** (probeFailures - 1))
      }
    })
    .finally(() => {
      if (checkFlight === current) checkFlight = null
    })
    .catch(() => {})
  await read.join(signal)
}

const queue = createRevalidator(
  async () => {
    await checkResourceVersions(listeners.keys())
  },
  {
    intervalMs: 500,
    retryMs: 5000,
    pollIntervalMs: 30_000,
    enabled: () => runtimeUsers > 0 && listeners.size > 0 && isDocumentVisible(),
  },
)

export function subscribeResource(key: string, callback: (event?: SseEvent) => void) {
  ensureScope()
  let callbacks = listeners.get(key)
  if (!callbacks) {
    callbacks = new Set()
    listeners.set(key, callbacks)
  }
  callbacks.add(callback)
  // Give the initial body read a chance to seed its token before the batch.
  queue.invalidate(500)
  return () => {
    callbacks.delete(callback)
    if (!callbacks.size) {
      listeners.delete(key)
      prune()
      if (!listeners.size) queue.pause()
    }
  }
}

export function invalidateResources(keys: Iterable<string> = listeners.keys()) {
  ensureScope()
  let changed = false
  for (const key of keys)
    if (listeners.has(key) || versions.has(key)) {
      dirty.add(key)
      invalidation.set(key, (invalidation.get(key) ?? 0) + 1)
      changed = true
    }
  if (changed) queue.invalidate(500)
}

export function applyResourceEvent(event: SseEvent) {
  const props = event.properties ?? {}
  const announced = props.resource_revisions
  if (announced && typeof announced === 'object' && !Array.isArray(announced)) {
    const part = props.part && typeof props.part === 'object' && !Array.isArray(props.part) ? props.part : null
    for (const [key, token] of Object.entries(announced)) {
      if (typeof token !== 'string' || (!listeners.has(key) && !versions.has(key))) continue
      // Text is rendered directly from the stream. Navigation ordering and
      // optimistic versions catch up at the shared 30s check; a token stream
      // must not repeatedly reload list/state representations.
      if ((part?.kind === 'text' || part?.kind === 'think') && !key.startsWith('part:')) continue
      if (noteResourceVersion(key, token, undefined, event)) {
        checkedAt.set(key, Date.now())
      }
    }
    return
  }
  const sid = String(props.session_id ?? '')
  const keys: string[] = []
  if (event.type === 'session_changed') {
    const part = props.part && typeof props.part === 'object' && !Array.isArray(props.part) ? props.part : null
    if (part) {
      keys.push(`part:${String(part.part_id)}`)
      if (part.kind !== 'text' && part.kind !== 'think') keys.push(`session:${sid}:state`)
      if (part.kind === 'tool_call') {
        for (const section of ['input', 'metadata', 'output']) keys.push(`part:${String(part.part_id)}:${section}`)
        const content =
          part.content && typeof part.content === 'object' && !Array.isArray(part.content) ? part.content : {}
        const name = String(content.name ?? '')
        if (
          ['completed', 'failed', 'cancelled'].includes(String(part.state)) &&
          (['fs.write', 'fs.replace', 'fs.apply_patch', 'code.rewrite_ast'].includes(name) || name.startsWith('shell.'))
        )
          keys.push(`session:${sid}:files`)
      }
    }
    if (!part || part.kind === 'run') {
      keys.push('sessions', 'workspaces', `session:${sid}:state`)
      keys.push(...[...listeners.keys()].filter((key) => key.startsWith('workspace:')))
    }
    if (props.kind === 'part_removed' || props.kind === 'session_deleted') keys.push(`session:${sid}:files`)
  } else if (event.type === 'runtime_signal') {
    const payload =
      props.payload && typeof props.payload === 'object' && !Array.isArray(props.payload) ? props.payload : {}
    if (props.kind === 'activity') {
      const activity =
        payload.activity && typeof payload.activity === 'object' && !Array.isArray(payload.activity)
          ? payload.activity
          : payload
      if (typeof activity.id === 'string') keys.push(`activity:${activity.id}`)
      if (payload.reason !== 'updated') {
        keys.push('activities')
        if (sid) keys.push(`session:${sid}:state`)
      }
    }
    if (props.kind === 'plugin' && payload.kind === 'plan.changed' && sid) keys.push(`session:${sid}:plan`)
  }
  invalidateResources(keys)
}

function visibility() {
  if (isDocumentVisible()) {
    invalidateResources()
    queue.resume()
  } else queue.pause()
}

export function startResourceSync() {
  runtimeUsers++
  if (runtimeUsers === 1) {
    document.addEventListener('visibilitychange', visibility)
    invalidateResources()
  }
  let released = false
  return () => {
    if (released) return
    released = true
    if (--runtimeUsers === 0) {
      queue.pause()
      document.removeEventListener('visibilitychange', visibility)
      checkFlight?.read.abort()
    }
  }
}
