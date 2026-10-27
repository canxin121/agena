import { localStorageKeys } from './persistence/storageKeys'
import { browserNotificationPlatformHint, notificationWorker } from './browserPush'

export type BrowserNotificationPermission = NotificationPermission | 'unsupported' | 'insecure'
export type BrowserNotificationKind = 'completed' | 'failed' | 'cancelled' | 'permission' | 'question'

export function browserNotificationPermission(): BrowserNotificationPermission {
  if (typeof window === 'undefined') return 'unsupported'
  if (!window.isSecureContext) return 'insecure'
  if (browserNotificationPlatformHint() === 'iosInstall') return 'unsupported'
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

export type BrowserNotificationHandle = { close: () => void }

export async function showBrowserNotification(
  title: string,
  options: NotificationOptions,
  onClick?: () => unknown,
): Promise<BrowserNotificationHandle | null> {
  if (browserNotificationPermission() !== 'granted') return null
  const notificationOptions = { icon: '/android-chrome-192x192.png?v=shift', requireInteraction: true, ...options }
  try {
    if (typeof navigator !== 'undefined' && navigator.serviceWorker) {
      const worker = await notificationWorker()
      await worker.showNotification(title, notificationOptions)
      const shown = await worker.getNotifications({ tag: options.tag })
      return {
        close: () => shown.forEach((item) => item.close()),
      }
    }
  } catch {
    // The worker can be unavailable on older servers or blocked by browser
    // settings. Desktop browsers can still use the constructor below.
  }
  try {
    const notification = new window.Notification(title, notificationOptions)
    notification.onclick = () => {
      notification.close()
      window.focus()
      try {
        void Promise.resolve(onClick?.()).catch(() => {})
      } catch {
        // Navigation may be unavailable after the app has been disposed.
      }
    }
    return await new Promise<BrowserNotificationHandle | null>((resolve) => {
      const timeout = setTimeout(() => finish(false), 8000)
      function finish(shown: boolean) {
        clearTimeout(timeout)
        notification.onshow = null
        notification.onerror = null
        if (!shown) notification.close()
        resolve(shown ? notification : null)
      }
      notification.onshow = () => finish(true)
      notification.onerror = () => finish(false)
    })
  } catch {
    // Permissions can change at any time; some browsers expose the API but
    // do not support the constructor. Notification delivery must not break SSE.
    return null
  }
}

export type PluginAttentionNotification = {
  plugin_id: string
  title: string
  session_id?: number | null
  attention?: {
    id: string
    kind: BrowserNotificationKind
    created_at_ms: number
    resolved: boolean
  }
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

/** Render plugin-owned intents; the browser never infers AI lifecycle events. */
export function createBrowserNotifier(options: {
  enabled: () => boolean
  scope: () => string
  message: (kind: BrowserNotificationKind) => string
  openSession: (sessionId: string) => unknown
}) {
  let since: number | undefined
  let generation = 0
  const seen = new Set<string>()
  const active = new Map<string, BrowserNotificationHandle>()
  let disposed = false

  async function applyNotifications(notifications: PluginAttentionNotification[], snapshotAtMs = Date.now()) {
    // Compare with the same server clock as the event. A remote device's
    // clock can differ enough to otherwise suppress every completed reply.
    since ??= snapshotAtMs
    const applyingGeneration = generation
    const current = new Set(notifications.filter((item) => !item.attention?.resolved).map((item) => item.attention?.id))
    for (const [id, notification] of active) {
      if (!current.has(id)) {
        notification.close()
        active.delete(id)
      }
    }
    for (const notification of notifications) {
      const event = notification.attention
      if (!event || event.resolved || seen.has(event.id)) continue
      if (!['completed', 'failed', 'cancelled', 'permission', 'question'].includes(event.kind)) continue
      if (event.created_at_ms < since && !['permission', 'question'].includes(event.kind)) continue
      if (disposed || applyingGeneration !== generation || !options.enabled()) return
      const scope = options.scope()
      const key = JSON.stringify([scope, event.id])
      const deliver = async () => {
        if (disposed || applyingGeneration !== generation || !options.enabled() || options.scope() !== scope)
          return false
        const delivered = deliveredNotifications()
        if (delivered.some(([existing]) => existing === key)) return true
        const sid = notification.session_id ? String(notification.session_id) : ''
        const handle = await showBrowserNotification(
          notification.title || 'Agena',
          {
            body: options.message(event.kind),
            tag: `agena:push:${event.id}`,
            data: {
              url: sid ? `/chat?sessionId=${encodeURIComponent(sid)}` : '/settings/interface?view=web-appearance',
            },
          },
          () => !disposed && options.openSession(sid),
        )
        if (!handle) return false
        if (disposed || applyingGeneration !== generation || !options.enabled() || options.scope() !== scope) {
          handle.close()
          return false
        }
        active.set(event.id, handle)
        try {
          localStorage.setItem(DELIVERY_KEY, JSON.stringify([...delivered, [key, Date.now()]].slice(-DELIVERY_LIMIT)))
        } catch {
          /* Browser storage can be blocked. Stable tags still dedupe. */
        }
        return true
      }
      let delivered: boolean
      if (typeof navigator !== 'undefined' && navigator.locks) {
        try {
          delivered = await navigator.locks.request(DELIVERY_KEY, deliver)
        } catch {
          delivered = await deliver()
        }
      } else {
        delivered = await deliver()
      }
      if (!delivered) throw new Error('Browser notification delivery failed')
      seen.add(event.id)
      while (seen.size > 512) seen.delete(seen.values().next().value!)
    }
  }

  return {
    applyNotifications,
    reset() {
      // Enabling later must recover pending questions, not all replies that
      // arrived while the user had disabled page notifications.
      since = undefined
      generation++
      for (const notification of active.values()) notification.close()
      active.clear()
    },
    dispose() {
      disposed = true
      for (const notification of active.values()) notification.close()
      active.clear()
    },
  }
}
