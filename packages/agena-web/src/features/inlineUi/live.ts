import { apiFetch, apiUrl } from '@/lib/api'
import { SseFrames } from '@/lib/sseFrames'
import { parseInlineDocument, type InlineDocument } from './model'
import { validInlineReference } from './references'

export type InlineSnapshot = {
  reference: string
  revision: number
  phase: 'working' | 'complete' | 'error'
  updated_at_ms: number
  last_call_id: number
  document: InlineDocument
}
const snapshots = new Map<string, InlineSnapshot>()
export const inlineScope = (session: string, reference: string) =>
  apiUrl(`/api/v1/sessions/${encodeURIComponent(session)}/inline-ui/${encodeURIComponent(reference)}`)
export function currentInlineSnapshot(session: string, reference: string) {
  return snapshots.get(inlineScope(session, reference))
}
export function parseInlineSnapshot(raw: unknown, reference: string): InlineSnapshot {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) throw new Error('Invalid UI snapshot')
  const v = raw as Record<string, unknown>
  if (
    v.reference !== reference ||
    !validInlineReference(reference) ||
    !Number.isSafeInteger(v.revision) ||
    Number(v.revision) < 1 ||
    !['working', 'complete', 'error'].includes(String(v.phase))
  )
    throw new Error('Invalid UI snapshot identity/revision')
  return {
    reference,
    revision: Number(v.revision),
    phase: v.phase as InlineSnapshot['phase'],
    updated_at_ms: Number(v.updated_at_ms || 0),
    last_call_id: Number(v.last_call_id || 0),
    document: parseInlineDocument(v.document),
  }
}

/** A complete snapshot on every reconnect is intentional. Sequence numbers are
 * document revisions, not incremental patches that can become unreplayable. */
export function watchInlineReference(
  session: string,
  reference: string,
  callbacks: {
    snapshot: (value: InlineSnapshot) => void
    status: (state: 'connected' | 'reconnecting' | 'unavailable') => void
  },
): () => void {
  const controller = new AbortController(),
    scope = inlineScope(session, reference)
  // Remounting/visibility reconnects must not roll a known reference back to
  // an older head. Do not expose cached content before authorization succeeds.
  let highest = snapshots.get(scope)?.revision || 0,
    delivered = false,
    closed = false,
    delay = 250
  const pause = (ms: number) =>
    new Promise<void>((resolve) => {
      const timer = setTimeout(done, ms)
      function done() {
        clearTimeout(timer)
        controller.signal.removeEventListener('abort', done)
        resolve()
      }
      controller.signal.addEventListener('abort', done, { once: true })
    })
  void (async () => {
    while (!closed) {
      let reader: ReadableStreamDefaultReader<Uint8Array> | undefined
      try {
        const response = await apiFetch(`${scope}/stream`, {
          signal: controller.signal,
          headers: { Accept: 'text/event-stream' },
          cache: 'no-store',
        })
        if ([401, 403, 404, 410].includes(response.status)) {
          snapshots.delete(scope)
          callbacks.status('unavailable')
          break
        }
        if (!response.ok || !response.body) throw new Error('Live UI stream unavailable')
        reader = response.body.getReader()
        const decoder = new TextDecoder(),
          frames = new SseFrames()
        let pendingBytes = 0
        while (!closed) {
          const result = await reader.read()
          if (result.done) break
          pendingBytes += result.value.byteLength
          if (pendingBytes > 512 * 1024) throw new Error('Live UI frame exceeds budget')
          for (const frame of frames.push(decoder.decode(result.value, { stream: true }))) {
            pendingBytes = 0
            const lines = frame.split('\n'),
              event = lines
                .find((line) => line.startsWith('event:'))
                ?.slice(6)
                .trim()
            if (event === 'inline_ui_error') {
              snapshots.delete(scope)
              callbacks.status('unavailable')
              closed = true
              break
            }
            if (event !== 'inline_ui') continue
            const payload = lines
              .filter((line) => line.startsWith('data:'))
              .map((line) => line.slice(5).trimStart())
              .join('\n')
            const snapshot = parseInlineSnapshot(JSON.parse(payload), reference)
            if (snapshot.revision >= highest) {
              callbacks.status('connected')
              delay = 250
              if (snapshot.revision > highest || !delivered) {
                highest = snapshot.revision
                delivered = true
                snapshots.set(scope, snapshot)
                while (snapshots.size > 64) snapshots.delete(snapshots.keys().next().value!)
                callbacks.snapshot(snapshot)
              }
            }
          }
        }
      } catch {
        if (closed || controller.signal.aborted) break
      } finally {
        if (reader) {
          try {
            await reader.cancel()
          } catch {
            /* request may already be aborted */
          }
          reader.releaseLock()
        }
      }
      if (!closed) {
        callbacks.status('reconnecting')
        await pause(delay)
        delay = Math.min(delay * 2, 8000)
      }
    }
  })()
  return () => {
    closed = true
    controller.abort()
  }
}
