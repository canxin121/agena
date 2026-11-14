import { withInterfaceLocale } from '@/i18n/interfaceLocale'
import { isDocumentVisible } from './backgroundReads'
import { connectSse, type SseClient, type SseEvent } from './sse'

/** A mounted conversation owns its live tail. The project-list stream excludes
 * workspace-free Bot conversations and cannot stand in for this subscription. */
export function connectSessionLive(
  sessionId: string,
  onEvent: (event: SseEvent) => void,
  reconcile: () => void,
): () => void {
  if (typeof document === 'undefined' || typeof window === 'undefined') return () => {}
  let stream: SseClient | undefined
  let generation = 0
  let disposed = false

  function close() {
    generation++
    stream?.close()
    stream = undefined
  }

  function open() {
    close()
    if (disposed || !isDocumentVisible()) return
    const owner = generation
    const current = () => !disposed && owner === generation && isDocumentVisible()
    const recover = () => {
      if (current()) reconcile()
    }
    const query = new URLSearchParams({ scope_kind: 'session', session_id: sessionId })
    stream = connectSse({
      endpoint: withInterfaceLocale(`/api/v1/changes/stream?${query}`),
      debugLabel: `sse:session:${sessionId}`,
      // Install the tail before checking snapshots, on the first connection as
      // well as reconnection. This closes the subscribe/read race.
      onOpen: recover,
      onSequenceGap: recover,
      onError: recover,
      onEvent(event) {
        if (!current()) return
        if (event.type === 'lagged' || event.type === 'subscription_closed') recover()
        else onEvent(event)
      },
    })
  }

  function visibility() {
    if (isDocumentVisible()) open()
    else close()
  }

  document.addEventListener('visibilitychange', visibility)
  window.addEventListener('online', open)
  window.addEventListener('pageshow', open)
  window.addEventListener('pagehide', close)
  open()
  return () => {
    if (disposed) return
    disposed = true
    close()
    document.removeEventListener('visibilitychange', visibility)
    window.removeEventListener('online', open)
    window.removeEventListener('pageshow', open)
    window.removeEventListener('pagehide', close)
  }
}
