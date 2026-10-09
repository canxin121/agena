import { i18n } from '@/i18n'
import { apiJson, apiUrl } from './api'
import { createSharedRead, limitBackgroundReads } from './backgroundReads'
import { captureResourceObservation } from './resourceSync'
import { readUiAuthTokenVersion } from './uiAuthToken'
import type { RuntimeSettingReadResponse, RuntimeSettingsReadBundle } from './runtimeSettings'

type Pending = {
  path: string
  epoch: string
  signal: AbortSignal
  resolve: (value: RuntimeSettingsReadBundle) => void
  reject: (reason: unknown) => void
}
let generation = 0
let scheduled = false
const queued: Pending[] = []
const flights = new Map<string, ReturnType<typeof createSharedRead<RuntimeSettingsReadBundle>>>()
// Leave room for the endpoint and scope parameters under common 8 KB URL limits.
const MAX_ENCODED_QUERY_LENGTH = 6_000

/** Writes retire reads started before the mutation, including a still-running batch. */
export function retireRuntimeSettingReads() {
  generation++
}

async function fetchBatch(entries: Pending[]) {
  const controller = new AbortController()
  const check = () => {
    if (entries.every((entry) => entry.signal.aborted)) controller.abort()
  }
  for (const entry of entries) entry.signal.addEventListener('abort', check)
  check()
  try {
    const query = new URLSearchParams({ paths: JSON.stringify(entries.map((entry) => entry.path)) })
    const response = await limitBackgroundReads(
      () =>
        apiJson<Record<string, RuntimeSettingsReadBundle>>(`/api/v1/settings/sources?${query}`, {
          signal: AbortSignal.any([controller.signal, AbortSignal.timeout(15_000)]),
        }),
      controller.signal,
    )
    for (const entry of entries) {
      const value = response[entry.path]
      if (value) entry.resolve(value)
      else entry.reject(new Error(i18n.global.t('errors.settings.batchOmittedPath')))
    }
  } catch (reason) {
    for (const entry of entries) entry.reject(reason)
  } finally {
    for (const entry of entries) entry.signal.removeEventListener('abort', check)
  }
}

function flush() {
  scheduled = false
  const groups = new Map<string, Pending[]>()
  for (const entry of queued.splice(0)) {
    if (entry.signal.aborted) {
      entry.reject(entry.signal.reason)
      continue
    }
    const group = groups.get(entry.epoch) ?? []
    if (!groups.has(entry.epoch)) groups.set(entry.epoch, group)
    group.push(entry)
  }
  for (const group of groups.values()) {
    let batch: Pending[] = []
    let encodedLength = 'paths=%5B%5D'.length
    for (const entry of group) {
      const pathLength = new URLSearchParams({ paths: JSON.stringify(entry.path) }).toString().length - 'paths='.length
      const addition = pathLength + (batch.length ? '%2C'.length : 0)
      if (batch.length && (batch.length === 64 || encodedLength + addition > MAX_ENCODED_QUERY_LENGTH)) {
        void fetchBatch(batch)
        batch = []
        encodedLength = 'paths=%5B%5D'.length
      }
      encodedLength += pathLength + (batch.length ? '%2C'.length : 0)
      batch.push(entry)
    }
    if (batch.length) void fetchBatch(batch)
  }
}

export function readBatchedRuntimeSettingSources(
  path: string,
  signal?: AbortSignal,
): Promise<RuntimeSettingsReadBundle> {
  signal?.throwIfAborted()
  const epoch = JSON.stringify([
    captureResourceObservation('sessions').scope,
    readUiAuthTokenVersion(),
    apiUrl('/'),
    generation,
  ])
  const key = `${epoch}:${path}`
  let flight = flights.get(key)
  if (flight?.isAborted()) {
    flights.delete(key)
    flight = undefined
  }
  if (!flight) {
    flight = createSharedRead(async (sharedSignal) => {
      // The advanced JSON editor also supports the root document. Leaf fields
      // use the compact batch endpoint; preserve the existing root read API.
      if (!path.trim()) {
        const read = (url: string) =>
          limitBackgroundReads(
            () =>
              apiJson<RuntimeSettingReadResponse>(url, {
                signal: AbortSignal.any([sharedSignal, AbortSignal.timeout(15_000)]),
              }),
            sharedSignal,
          )
        const [effective, file, global, workspace] = await Promise.all([
          read('/api/v1/settings?source=effective'),
          read('/api/v1/settings?source=file'),
          read('/api/v1/settings/layers/global'),
          read('/api/v1/settings/layers/workspace'),
        ])
        return { effective, file, global, workspace }
      }
      return new Promise<RuntimeSettingsReadBundle>((resolve, reject) => {
        queued.push({ path, epoch, signal: sharedSignal, resolve, reject })
        if (!scheduled) {
          scheduled = true
          queueMicrotask(flush)
        }
      })
    })
    const shared = flight
    flights.set(key, shared)
    const release = () => {
      if (flights.get(key) === shared) flights.delete(key)
    }
    shared.promise.then(release, release)
  }
  return flight.join(signal)
}
