import type { JsonValue as JsonLike } from '@/types/json'

import { emitAuthRequired, extractAuthRequiredMessageFromBodyText } from './authEvents.ts'
import { apiUrl } from './api'
import { buildActiveUiAuthHeaders } from './uiAuthToken'

export type SseEvent = {
  type: string
  // Some emitters use { type, properties } shaped events.
  properties?: Record<string, JsonLike>
  [k: string]: JsonLike
}

export type SseClientStats = {
  label: string
  url: string
  startedAt: number
  lastChunkAt: number
  lastEventAt: number
  lastCursor: string | null
  connectCount: number
  reconnectCount: number
  errorCount: number
  stallCount: number
  lastBackoffMs: number
  lastErrorAt: number
  lastErrorMessage: string
}

export type SseClientOptions = {
  directory?: string | null
  endpoint?: string
  initialLastEventId?: string | null
  debugLabel?: string
  debug?: boolean
  autoReconnect?: boolean
  stallTimeoutMsVisible?: number
  stallTimeoutMsHidden?: number
  onEvent: (evt: SseEvent) => void
  onCursor?: (lastEventId: string) => void
  onSequenceGap?: (gap: { previous: number; expected: number; current: number }) => void
  onError?: (err: Error | string) => void
}

export type SseClient = {
  close: () => void
  getStats: () => SseClientStats
}

type UnknownRecord = Record<string, JsonLike>

function linkAbortSignals(signals: AbortSignal[]): { signal: AbortSignal; cleanup: () => void } {
  const controller = new AbortController()
  const listeners: Array<() => void> = []

  const abort = () => {
    try {
      controller.abort()
    } catch {
      // ignore
    }
  }

  for (const sig of signals) {
    if (!sig) continue
    if (sig.aborted) {
      abort()
      continue
    }
    const handler = () => abort()
    try {
      sig.addEventListener('abort', handler, { once: true })
      listeners.push(() => {
        try {
          sig.removeEventListener('abort', handler)
        } catch {
          // ignore
        }
      })
    } catch {
      // ignore
    }
  }

  return {
    signal: controller.signal,
    cleanup: () => {
      for (const off of listeners) off()
    },
  }
}

function isRecord(value: JsonLike): value is UnknownRecord {
  return typeof value === 'object' && value !== null
}

function getRecord(value: JsonLike, key: string): UnknownRecord | null {
  if (!isRecord(value)) return null
  const nested = value[key]
  return isRecord(nested) ? nested : null
}

function getString(value: JsonLike, key: string): string {
  if (!isRecord(value)) return ''
  const candidate = value[key]
  return typeof candidate === 'string' ? candidate : ''
}

function parsePositiveSeq(raw: string | null | undefined): number | null {
  const id = typeof raw === 'string' ? raw.trim() : ''
  if (!id) return null
  const parsed = Number.parseInt(id, 10)
  if (!Number.isFinite(parsed) || parsed <= 0) return null
  return parsed
}

export function detectSseSequenceGap(
  previousLastEventId: string | null | undefined,
  currentLastEventId: string | null | undefined,
): { previous: number; expected: number; current: number } | null {
  const previous = parsePositiveSeq(previousLastEventId)
  const current = parsePositiveSeq(currentLastEventId)
  if (previous === null || current === null) return null
  if (current <= previous + 1) return null
  return {
    previous,
    expected: previous + 1,
    current,
  }
}

export function advanceSseCursor(
  previousLastEventId: string | null | undefined,
  incomingLastEventId: string | null | undefined,
): {
  accepted: boolean
  nextCursor: string | null
  gap: { previous: number; expected: number; current: number } | null
} {
  const previous = typeof previousLastEventId === 'string' ? previousLastEventId.trim() : ''
  const incoming = typeof incomingLastEventId === 'string' ? incomingLastEventId.trim() : ''
  if (!incoming) {
    return { accepted: false, nextCursor: previous || null, gap: null }
  }

  const previousSeq = parsePositiveSeq(previous)
  const incomingSeq = parsePositiveSeq(incoming)
  if (previousSeq !== null && incomingSeq !== null && incomingSeq <= previousSeq) {
    return { accepted: false, nextCursor: previous || null, gap: null }
  }

  return {
    accepted: true,
    nextCursor: incoming,
    gap: detectSseSequenceGap(previous || null, incoming),
  }
}

export function connectSse(opts: SseClientOptions): SseClient {
  const defaultEndpoint = '/api/v1/changes/stream?scope_kind=global'
  const urlBase = apiUrl((opts.endpoint || defaultEndpoint).trim() || defaultEndpoint)
  const dir = typeof opts.directory === 'string' ? opts.directory.trim() : ''
  const url = dir ? `${urlBase}${urlBase.includes('?') ? '&' : '?'}directory=${encodeURIComponent(dir)}` : urlBase

  const label = (opts.debugLabel || '').trim() || 'sse'
  const dev = Boolean(import.meta.env && import.meta.env.DEV)
  const debugEnabled = dev && (opts.debug === true || Boolean(opts.debugLabel))
  let lastDebugAt = 0
  function debugLog(message: string, data?: unknown, optsLog?: { force?: boolean }) {
    if (!debugEnabled) return
    const now = Date.now()
    if (!optsLog?.force && now - lastDebugAt < 15000) return
    lastDebugAt = now
    try {
      if (data !== undefined) console.debug(`[${label}] ${message}`, data)
      else console.debug(`[${label}] ${message}`)
    } catch {
      // ignore
    }
  }

  const controller = new AbortController()
  let closed = false
  let reconnectTimer: number | null = null
  let lastEventId: string | undefined =
    typeof opts.initialLastEventId === 'string' && opts.initialLastEventId.trim()
      ? opts.initialLastEventId.trim()
      : undefined
  let retryDelay = 3000
  let attempt = 0

  const stats: SseClientStats = {
    label,
    url,
    startedAt: Date.now(),
    lastChunkAt: 0,
    lastEventAt: 0,
    lastCursor: lastEventId || null,
    connectCount: 0,
    reconnectCount: 0,
    errorCount: 0,
    stallCount: 0,
    lastBackoffMs: 0,
    lastErrorAt: 0,
    lastErrorMessage: '',
  }

  // Coalesce high-frequency events (message.part.updated) so UI work stays near 60fps.
  // Inspired by OpenCode's own web/app event coalescing.
  const queue: Array<SseEvent | undefined> = []
  const coalesced = new Map<string, number>()
  let timer: number | null = null
  let lastFlushAt = 0

  function getNumeric(value: JsonLike, key: string): number | null {
    if (!isRecord(value)) return null
    const candidate = value[key]
    return typeof candidate === 'number' && Number.isFinite(candidate) ? candidate : null
  }

  function eventKey(evt: SseEvent): string | null {
    const props = isRecord(evt.properties) ? evt.properties : {}
    if (evt.type === 'session_changed') {
      const changeKind = getString(props, 'kind')
      if (changeKind === 'part_added' || changeKind === 'part_updated') {
        const sid = getNumeric(props, 'session_id') ?? -1
        const part = getRecord(props, 'part')
        const pid = getNumeric(props, 'part_id') ?? (part ? getNumeric(part, 'part_id') : null)
        if (pid === null) return null
        return `session_changed:part:${String(sid)}:${String(pid)}`
      }
      if (changeKind === 'session_meta_updated') {
        const sid = getNumeric(props, 'session_id') ?? -1
        return `session_changed:meta:${String(sid)}`
      }
      return null
    }
    if (evt.type === 'runtime_signal') {
      const sid = getNumeric(props, 'session_id') ?? -1
      return `runtime_signal:${String(sid)}`
    }
    return null
  }

  function mergeEvent(prev: SseEvent, next: SseEvent): SseEvent {
    if (prev.type !== next.type) return next

    if (next.type === 'session_changed') {
      // Latest part payload wins (each PartResource is complete).
      const prevProps = isRecord(prev.properties) ? prev.properties : {}
      const nextProps = isRecord(next.properties) ? next.properties : {}
      const mergedPart = {
        ...(getRecord(prevProps, 'part') || {}),
        ...(getRecord(nextProps, 'part') || {}),
      }
      const merged: SseEvent = {
        ...prev,
        ...next,
        properties: {
          ...prevProps,
          ...nextProps,
          ...(Object.keys(mergedPart).length ? { part: mergedPart } : {}),
        },
      }
      return merged
    }

    // Runtime signals / meta updates: latest wins.
    return next
  }

  function flush() {
    if (timer !== null) {
      window.clearTimeout(timer)
      timer = null
    }
    if (queue.length === 0) return

    const events = queue.splice(0, queue.length)
    coalesced.clear()

    lastFlushAt = Date.now()
    for (const evt of events) {
      if (!evt) continue
      stats.lastEventAt = Date.now()
      opts.onEvent(evt)
    }
  }

  function scheduleFlush() {
    if (timer !== null) return
    const elapsed = Date.now() - lastFlushAt
    timer = window.setTimeout(flush, Math.max(0, 16 - elapsed))
  }

  function pushEvent(evt: SseEvent) {
    const k = eventKey(evt)
    if (k) {
      const idx = coalesced.get(k)
      if (idx !== undefined) {
        const prev = queue[idx]
        queue[idx] = prev ? mergeEvent(prev, evt) : evt
      } else {
        coalesced.set(k, queue.length)
        queue.push(evt)
      }
    } else {
      queue.push(evt)
    }
    scheduleFlush()
  }

  /**
   * Normalize an agena Notification envelope into the SseEvent shape
   * (type + properties) that the chat store and activity store consume.
   *
   * Wire: `event: notification`, body = Notification (serde tag = "kind"):
   *   { kind: "session_changed",     data: { subscription, change: SessionChangeResource } }
   *   { kind: "runtime_signal",      data: { subscription, signal: RuntimeSignalResource } }
   *   { kind: "lagged",              data: { subscription, skipped } }
   *   { kind: "subscription_closed", data: { subscription, reason } }
   *
   * Mapping (opencode SSE → agena):
   *   message.part.created / .updated  → session_changed (part_added / part_updated)
   *   message.part.removed             → session_changed (part_removed)
   *   session.updated                  → session_changed (session_meta_updated)
   *   session.status / idle            → runtime_signal
   *   permission.asked / question.asked → pending_interactive_requests (state-driven)
   */
  function normalizeAndQueue(raw: JsonLike, meta?: { directory?: string; lastEventId?: string }) {
    if (!isRecord(raw)) return

    let evt: UnknownRecord | null = null

    const kind = getString(raw, 'kind')
    const data = getRecord(raw, 'data')
    if (kind && data) {
      if (kind === 'session_changed') {
        const change = getRecord(data, 'change')
        if (change) {
          const changeKind = getString(change, 'kind')
          const sid = getNumeric(change, 'session_id')
          const props: UnknownRecord = { kind: changeKind }
          if (sid !== null) props.session_id = sid
          if (changeKind === 'part_added' || changeKind === 'part_updated') {
            const part = getRecord(change, 'part')
            if (part) props.part = part
          } else if (changeKind === 'part_removed') {
            const pid = getNumeric(change, 'part_id')
            if (pid !== null) props.part_id = pid
          } else if (changeKind === 'session_meta_updated') {
            const version = getNumeric(change, 'version')
            const updatedMs = getNumeric(change, 'updated_at_ms')
            if (typeof change.title === 'string') props.title = change.title
            if (version !== null) props.version = version
            if (updatedMs !== null) props.updated_at_ms = updatedMs
          }
          evt = { type: 'session_changed', properties: props }
        }
      } else if (kind === 'runtime_signal') {
        const signal = getRecord(data, 'signal')
        if (signal) {
          const signalKind = getString(signal, 'kind')
          const sid = getNumeric(signal, 'session_id')
          const payload = signal.payload
          const props: UnknownRecord = { kind: signalKind }
          if (sid !== null) props.session_id = sid
          if (isRecord(payload)) props.payload = payload
          evt = { type: 'runtime_signal', properties: props }
        }
      } else if (kind === 'lagged') {
        const skipped = getNumeric(data, 'skipped')
        const props: UnknownRecord = {}
        if (skipped !== null) props.skipped = skipped
        evt = { type: 'lagged', properties: props }
      } else if (kind === 'subscription_closed') {
        evt = { type: 'subscription_closed', properties: {} }
      }
    }

    // Legacy passthrough for any remaining {type, properties} emitters.
    if (!evt) {
      if (typeof raw.type === 'string') {
        evt = raw
      } else {
        const payload = raw.payload
        if (isRecord(payload) && typeof payload.type === 'string') {
          evt = payload
        }
      }
    }

    if (!evt || typeof evt.type !== 'string') return
    if (typeof meta?.lastEventId === 'string' && meta.lastEventId) {
      evt.lastEventId = meta.lastEventId
    }
    pushEvent(evt as SseEvent)
  }

  async function sleep(ms: number) {
    await new Promise<void>((resolve) => {
      reconnectTimer = window.setTimeout(() => resolve(), ms)
    })
  }

  async function runFetchStream() {
    while (!closed && !controller.signal.aborted) {
      attempt += 1
      let linkedCleanup: (() => void) | null = null
      try {
        debugLog('connect attempt', { attempt, lastEventId: lastEventId || '' })
        const headers = new Headers()
        headers.set('accept', 'text/event-stream')
        if (lastEventId) headers.set('last-event-id', lastEventId)

        const auth = buildActiveUiAuthHeaders()
        if (auth.authorization) {
          try {
            headers.set('authorization', auth.authorization)
          } catch {
            // ignore
          }
        }

        const attemptAbort = new AbortController()
        const linked = linkAbortSignals([controller.signal, attemptAbort.signal])
        linkedCleanup = linked.cleanup

        const resp = await fetch(url, {
          method: 'GET',
          headers,
          cache: 'no-store',
          credentials: auth.authorization ? 'omit' : 'include',
          signal: linked.signal,
        })
        if (!resp.ok) {
          if (resp.status === 401) {
            let msg = ''
            try {
              const txt = await resp.text().catch(() => '')
              msg = extractAuthRequiredMessageFromBodyText(txt)
            } catch {
              // ignore
            }
            emitAuthRequired({
              message: msg || 'UI authentication required',
              status: resp.status,
              code: 'auth_required',
              url,
            })
            // Stop reconnect loop until the app is re-authenticated.
            closed = true
          }
          throw new Error(`SSE failed: ${resp.status} ${resp.statusText}`)
        }
        if (!resp.body || typeof resp.body.getReader !== 'function') {
          throw new Error('SSE streaming required')
        }

        stats.connectCount += 1
        debugLog('connected', { attempt, lastEventId: lastEventId || '' }, { force: true })

        // Successful (re)connect resets exponential backoff attempts.
        attempt = 0

        const reader = resp.body.getReader()
        const decoder = new TextDecoder()
        let buf = ''
        let allowCursorResetOnFirstId = true
        let lastByteAt = Date.now()

        try {
          while (!closed && !controller.signal.aborted) {
            const visibility =
              typeof document !== 'undefined' && typeof document.visibilityState === 'string'
                ? document.visibilityState
                : 'visible'
            const stallTimeoutMs =
              visibility === 'visible'
                ? Math.max(1, Math.floor(opts.stallTimeoutMsVisible ?? 45_000))
                : Math.max(1, Math.floor(opts.stallTimeoutMsHidden ?? 120_000))
            let stallTimer: number | null = null
            stallTimer = window.setTimeout(() => {
              // No bytes received for too long: abort this attempt so we can reconnect.
              // This handles fetch-stream stalls that never resolve or throw.
              if (closed || controller.signal.aborted) return
              const now = Date.now()
              const age = now - lastByteAt
              stats.stallCount += 1
              debugLog('stall timeout abort', { stallTimeoutMs, age }, { force: true })
              try {
                attemptAbort.abort()
              } catch {
                // ignore
              }
            }, stallTimeoutMs)

            const { done, value } = await reader.read().finally(() => {
              if (stallTimer !== null) {
                window.clearTimeout(stallTimer)
                stallTimer = null
              }
            })
            if (done) break
            if (!value) continue

            lastByteAt = Date.now()
            stats.lastChunkAt = lastByteAt

            buf += decoder.decode(value, { stream: true })
            // Normalize line endings: CRLF -> LF, then CR -> LF
            buf = buf.replace(/\r\n/g, '\n').replace(/\r/g, '\n')

            const chunks = buf.split('\n\n')
            buf = chunks.pop() ?? ''

            for (const chunk of chunks) {
              const lines = chunk.split('\n')
              const dataLines: string[] = []
              let seenId: string | undefined
              let skipChunk = false

              for (const line of lines) {
                if (!line) continue
                if (line.startsWith('data:')) {
                  dataLines.push(line.replace(/^data:\s*/, ''))
                  continue
                }
                if (line.startsWith('id:')) {
                  const v = line.replace(/^id:\s*/, '')
                  const previousSeq = parsePositiveSeq(lastEventId)
                  const incomingSeq = parsePositiveSeq(v)
                  const allowReset =
                    allowCursorResetOnFirstId &&
                    previousSeq !== null &&
                    incomingSeq !== null &&
                    incomingSeq < previousSeq

                  const transition = advanceSseCursor(lastEventId, v)
                  if (!transition.accepted) {
                    if (allowReset && v) {
                      if (previousSeq !== null && incomingSeq !== null && incomingSeq !== previousSeq + 1) {
                        opts.onSequenceGap?.({
                          previous: previousSeq,
                          expected: previousSeq + 1,
                          current: incomingSeq,
                        })
                      }
                      // On a fresh reconnect, allow a one-time cursor reset when the
                      // server stream restarts from a lower numeric sequence baseline.
                      lastEventId = v
                      opts.onCursor?.(v)
                      seenId = v
                      allowCursorResetOnFirstId = false
                      continue
                    }
                    skipChunk = true
                    continue
                  }

                  if (transition.gap) {
                    opts.onSequenceGap?.(transition.gap)
                  }

                  if (transition.nextCursor) {
                    lastEventId = transition.nextCursor
                    opts.onCursor?.(transition.nextCursor)
                    seenId = transition.nextCursor
                    allowCursorResetOnFirstId = false
                  }
                  continue
                }
                if (line.startsWith('retry:')) {
                  const parsed = Number.parseInt(line.replace(/^retry:\s*/, ''), 10)
                  if (!Number.isNaN(parsed) && parsed > 0) {
                    retryDelay = parsed
                  }
                  continue
                }
                // Ignore: event:, comments, etc.
              }

              if (skipChunk) continue

              if (!dataLines.length) continue
              const rawData = dataLines.join('\n')
              let raw: JsonLike
              try {
                raw = JSON.parse(rawData)
              } catch {
                continue
              }
              normalizeAndQueue(raw, { directory: opts.directory ?? undefined, lastEventId: seenId })
            }
          }
        } finally {
          try {
            reader.releaseLock()
          } catch {
            // ignore
          }
        }

        // If the server closed the stream (but the client didn't), mimic EventSource
        // by reconnecting with backoff instead of spinning.
        if (!closed && !controller.signal.aborted) {
          throw new Error('SSE connection closed')
        }
      } catch (err) {
        flush()
        const nextError: Error | string = err instanceof Error ? err : String(err)
        stats.errorCount += 1
        stats.lastErrorAt = Date.now()
        stats.lastErrorMessage = nextError instanceof Error ? nextError.message : String(nextError)
        opts.onError?.(nextError)

        debugLog('error', { message: stats.lastErrorMessage, attempt }, { force: true })

        // No fallback: modern browsers only.
        if (nextError instanceof Error && nextError.message.includes('SSE streaming required')) {
          closed = true
          break
        }

        if (opts.autoReconnect === false) {
          closed = true
          break
        }

        if (closed || controller.signal.aborted) break

        const backoff = Math.min(retryDelay * 2 ** Math.max(0, attempt - 1), 30000)
        stats.lastBackoffMs = backoff
        stats.reconnectCount += 1
        await sleep(backoff)
      } finally {
        if (linkedCleanup) {
          linkedCleanup()
          linkedCleanup = null
        }
      }
    }
  }

  if (typeof fetch !== 'function' || typeof TextDecoder === 'undefined') {
    throw new Error('SSE requires fetch + TextDecoder')
  }
  void runFetchStream()

  return {
    close: () => {
      closed = true
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer)
        reconnectTimer = null
      }
      try {
        controller.abort()
      } catch {
        // ignore
      }
      flush()
    },
    getStats: () => {
      // Return a shallow copy to avoid accidental external mutation.
      return {
        ...stats,
        lastCursor: typeof lastEventId === 'string' && lastEventId.trim() ? lastEventId.trim() : stats.lastCursor,
      }
    },
  }
}
