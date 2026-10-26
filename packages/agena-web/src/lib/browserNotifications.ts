import { isRunCancelled, isRunFailureState, isRunInFlight, normalizeRunState } from './chatRunState'
import { interactiveRequests, pendingPartInteractions, type InteractiveRequest } from './interactiveRequests'
import { localStorageKeys } from './persistence/storageKeys'
import type { SseEvent } from './sse'

export type BrowserNotificationPermission = NotificationPermission | 'unsupported' | 'insecure'
export type BrowserNotificationKind = 'completed' | 'failed' | 'cancelled' | 'permission' | 'question'
export type BrowserNotificationSnapshot = { parts: unknown[]; requests?: unknown[] }

export function browserNotificationPermission(): BrowserNotificationPermission {
  if (typeof window === 'undefined') return 'unsupported'
  if (!window.isSecureContext) return 'insecure'
  if (typeof window.Notification !== 'function') return 'unsupported'
  return window.Notification.permission
}

/** Call directly from a user gesture; browsers reject unsolicited permission prompts. */
export async function requestBrowserNotificationPermission(): Promise<BrowserNotificationPermission> {
  const permission = browserNotificationPermission()
  if (permission !== 'default') return permission
  try {
    return await window.Notification.requestPermission()
  } catch {
    return browserNotificationPermission()
  }
}

export function showBrowserNotification(title: string, options: NotificationOptions, onClick?: () => unknown) {
  if (browserNotificationPermission() !== 'granted') return null
  try {
    const notification = new window.Notification(title, {
      icon: '/android-chrome-192x192.png?v=shift',
      ...options,
    })
    notification.onclick = () => {
      notification.close()
      window.focus()
      try {
        void Promise.resolve(onClick?.()).catch(() => {})
      } catch {
        // Navigation may be unavailable after the app has been disposed.
      }
    }
    return notification
  } catch {
    // Permissions can change at any time; some browsers expose the API but
    // do not support the constructor. Notification delivery must not break SSE.
    return null
  }
}

type NotificationIntent = { key: string; sessionId: string; kind: BrowserNotificationKind }

function requestIntent(request: InteractiveRequest): NotificationIntent {
  return {
    key: `request:${request.sessionId}:${request.requestId}`,
    sessionId: request.sessionId,
    kind: request.kind === 'permission' ? 'permission' : 'question',
  }
}

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function notificationIntents(evt: SseEvent): NotificationIntent[] {
  const props = evt.properties ?? {}
  if (evt.type !== 'session_changed' || (props.kind !== 'part_added' && props.kind !== 'part_updated')) return []
  const sid = typeof props.session_id === 'number' ? String(props.session_id) : ''
  const part = record(props.part)
  if (!sid || part.role !== 'assistant' || typeof part.part_id !== 'number') return []
  // Forked/shared history belongs to its original conversation.
  if (typeof part.origin_session_id === 'number' && String(part.origin_session_id) !== sid) return []
  const intents: NotificationIntent[] = []
  if (part.message_boundary === true) {
    // A finished content stream or tool does not imply a finished AI reply.
    const state = normalizeRunState(part.run_state || part.state)
    const kind =
      state === 'completed'
        ? 'completed'
        : isRunFailureState(state)
          ? 'failed'
          : isRunCancelled(state)
            ? 'cancelled'
            : null
    if (kind) {
      intents.push({
        key: `reply:${sid}:${part.part_id}:${part.created_at_ms ?? ''}`,
        sessionId: sid,
        kind,
      })
    }
  }
  for (const request of pendingPartInteractions({
    agenaKind: part.kind,
    agenaContent: part.content,
    partState: part.state,
    originSessionId: part.origin_session_id,
    sessionID: sid,
  })) {
    intents.push(requestIntent(request))
  }
  return intents
}

const DELIVERY_KEY = localStorageKeys.notifications.delivered
const DELIVERY_LIMIT = 256
const DELIVERY_TTL_MS = 24 * 60 * 60 * 1000

function deliveredNotifications(): Array<[string, number]> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(DELIVERY_KEY) || '[]')
    if (!Array.isArray(parsed)) return []
    const cutoff = Date.now() - DELIVERY_TTL_MS
    return parsed
      .filter(
        (entry): entry is [string, number] =>
          Array.isArray(entry) && typeof entry[0] === 'string' && typeof entry[1] === 'number' && entry[1] > cutoff,
      )
      .slice(-DELIVERY_LIMIT)
  } catch {
    return []
  }
}

export function createBrowserNotifier(options: {
  enabled: () => boolean
  scope: () => string
  sessionTitle: (sessionId: string) => string
  message: (kind: BrowserNotificationKind) => string
  openSession: (sessionId: string) => unknown
  /** Undefined: pending interactions only; []: discover an existing active reply. */
  readSession?: (sessionId: string, replyIds: number[] | undefined) => Promise<BrowserNotificationSnapshot | null>
}) {
  const seen = new Set<string>()
  const revisions = new Map<string, number>()
  const active = new Map<string, Notification>()
  const pendingReplies = new Map<string, Set<number>>()
  const changedSessions = new Set<string>()
  const generations = new Map<string, number>()
  let currentScope = ''
  let disposed = false

  function close() {
    for (const notification of active.values()) notification.close()
    active.clear()
  }

  async function deliver(intent: NotificationIntent, scope: string) {
    const key = JSON.stringify([scope, intent.key])
    const send = () => {
      if (disposed || !options.enabled() || options.scope() !== scope) return
      const delivered = deliveredNotifications()
      if (delivered.some(([existing]) => existing === key)) return
      const notification = showBrowserNotification(
        options.sessionTitle(intent.sessionId) || 'Agena',
        {
          body: options.message(intent.kind),
          tag: `agena:${scope}:session:${intent.sessionId}`,
        },
        () => !disposed && options.openSession(intent.sessionId),
      )
      if (!notification) return
      active.get(intent.sessionId)?.close()
      active.delete(intent.sessionId)
      active.set(intent.sessionId, notification)
      notification.onclose = () => {
        if (active.get(intent.sessionId) === notification) active.delete(intent.sessionId)
      }
      if (active.size > 32) {
        const oldest = active.keys().next().value!
        active.get(oldest)?.close()
        active.delete(oldest)
      }
      try {
        localStorage.setItem(DELIVERY_KEY, JSON.stringify([...delivered, [key, Date.now()]].slice(-DELIVERY_LIMIT)))
      } catch {
        // In-memory deduplication and the stable tag still work without storage.
      }
    }
    // Share delivery claims across tabs. The stable notification tag also
    // replaces duplicates on browsers without Web Locks or localStorage.
    if (typeof navigator !== 'undefined' && navigator.locks) {
      try {
        await navigator.locks.request(DELIVERY_KEY, send)
        return
      } catch {
        // Private browsing and embedded contexts may disallow Web Locks.
      }
    }
    send()
  }

  async function notify(intents: NotificationIntent[], scope: string) {
    for (const intent of intents) {
      if (disposed || options.scope() !== scope) return
      if (seen.has(intent.key)) continue
      seen.add(intent.key)
      if (seen.size > 512) seen.delete(seen.values().next().value!)
      if (options.enabled()) await deliver(intent, scope)
    }
  }

  async function applyEvent(evt: SseEvent) {
    if (disposed) return
    const scope = options.scope()
    if (scope !== currentScope) {
      close()
      seen.clear()
      revisions.clear()
      pendingReplies.clear()
      changedSessions.clear()
      generations.clear()
      currentScope = scope
    }
    const props = evt.properties ?? {}
    const sid = typeof props.session_id === 'number' ? String(props.session_id) : ''
    if (evt.type === 'session_changed' && sid) {
      generations.set(sid, (generations.get(sid) ?? 0) + 1)
      if (props.kind === 'session_meta_updated') changedSessions.add(sid)
      if (changedSessions.size > 256) changedSessions.delete(changedSessions.values().next().value!)
      if (generations.size > 512) generations.delete(generations.keys().next().value!)
    }
    if (evt.type === 'session_changed' && props.kind === 'session_deleted') {
      active.get(sid)?.close()
      active.delete(sid)
      pendingReplies.delete(sid)
      changedSessions.delete(sid)
      return
    }
    const part = record(props.part)
    if (typeof part.part_id === 'number' && typeof part.revision === 'number') {
      const key = `${props.session_id}:${part.part_id}`
      if (part.revision < (revisions.get(key) ?? -Infinity)) return
      revisions.delete(key)
      revisions.set(key, part.revision)
      if (revisions.size > 512) revisions.delete(revisions.keys().next().value!)
    }
    if (
      sid &&
      part.role === 'assistant' &&
      part.message_boundary === true &&
      typeof part.part_id === 'number' &&
      (part.origin_session_id == null || String(part.origin_session_id) === sid)
    ) {
      if (isRunInFlight(part.run_state || part.state)) {
        let ids = pendingReplies.get(sid)
        if (!ids) pendingReplies.set(sid, (ids = new Set()))
        ids.add(part.part_id)
      } else if (pendingReplies.has(sid)) {
        const ids = pendingReplies.get(sid)!
        ids.delete(part.part_id)
        if (!ids.size) pendingReplies.delete(sid)
      }
      if (pendingReplies.size > 256) pendingReplies.delete(pendingReplies.keys().next().value!)
    }
    await notify(notificationIntents(evt), scope)
  }

  return {
    applyEvent,
    /** Seed only sessions known to be active; opening the app must not notify old replies. */
    trackSessions(sessionIds: string[]) {
      currentScope = options.scope()
      for (const sid of sessionIds.slice(-256)) if (!pendingReplies.has(sid)) pendingReplies.set(sid, new Set())
    },
    async reconcile() {
      if (disposed || !options.enabled() || !options.readSession) return
      const scope = options.scope()
      const sessions = new Set([...changedSessions, ...pendingReplies.keys()])
      changedSessions.clear()
      await Promise.all(
        [...sessions].map(async (sid) => {
          const generation = generations.get(sid) ?? 0
          const ids = pendingReplies.get(sid)
          try {
            const snapshot = await options.readSession!(sid, ids ? [...ids] : undefined)
            if (disposed || options.scope() !== scope) return
            // A live change during the read takes precedence over that snapshot.
            if ((generations.get(sid) ?? 0) !== generation) {
              changedSessions.add(sid)
              return
            }
            if (!snapshot) {
              pendingReplies.delete(sid)
              return
            }
            await notify(interactiveRequests(snapshot.requests || [], sid).map(requestIntent), scope)
            for (const part of snapshot.parts)
              await applyEvent({
                type: 'session_changed',
                properties: { kind: 'part_updated', session_id: Number(sid), part },
              } as SseEvent)
          } catch (error) {
            if (!disposed && options.scope() === scope) changedSessions.add(sid)
            throw error
          }
        }),
      )
    },
    dispose() {
      disposed = true
      close()
      seen.clear()
      revisions.clear()
      pendingReplies.clear()
      changedSessions.clear()
      generations.clear()
    },
  }
}
