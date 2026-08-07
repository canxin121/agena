// Pure helpers for the unified notification store. Kept framework-free so the
// selection/expiry/dedup logic is unit-testable (bun test).

import type { LocalNotificationOptions, NotificationResource, NotificationSeverity, NotificationSurface } from './types'

/** Stable sort: newest first; ties broken by priority (higher first). */
export function sortNotifications(list: NotificationResource[]): NotificationResource[] {
  return [...list].sort((a, b) => {
    if (a.created_at_ms !== b.created_at_ms) return b.created_at_ms - a.created_at_ms
    return b.priority - a.priority
  })
}

export function isNotificationActive(notification: NotificationResource, nowMs = Date.now()): boolean {
  if (notification.dismissed) return false
  if (notification.expires_at_ms != null && nowMs >= notification.expires_at_ms) return false
  return true
}

export function activeNotifications(list: NotificationResource[], nowMs = Date.now()): NotificationResource[] {
  return list.filter((notification) => isNotificationActive(notification, nowMs))
}

/** Insert or replace by id, preserving list order. */
export function upsertNotification(list: NotificationResource[], next: NotificationResource): NotificationResource[] {
  const existingIndex = list.findIndex((notification) => notification.id === next.id)
  if (existingIndex >= 0) {
    const updated = [...list]
    updated[existingIndex] = { ...updated[existingIndex], ...next }
    return updated
  }
  return [...list, next]
}

/**
 * Merge a freshly fetched server page into the current list without dropping
 * entries that arrived over SSE while the fetch was in flight. Fetched items
 * win; unknown current items are kept (they are still filtered by active state).
 */
export function mergeServerList(
  current: NotificationResource[],
  fetched: NotificationResource[],
): NotificationResource[] {
  let merged = [...fetched]
  for (const item of current) {
    if (!merged.some((notification) => notification.id === item.id)) {
      merged = [...merged, item]
    }
  }
  return merged
}

/** Active banner notifications, most important first (priority, then recency). */
export function selectBanners(list: NotificationResource[], nowMs = Date.now()): NotificationResource[] {
  return activeNotifications(list, nowMs)
    .filter((notification) => notification.surface === 'banner')
    .sort((a, b) => {
      if (a.priority !== b.priority) return b.priority - a.priority
      return b.created_at_ms - a.created_at_ms
    })
}

/** Active toast notifications, newest first. */
export function selectToasts(list: NotificationResource[], nowMs = Date.now()): NotificationResource[] {
  return sortNotifications(activeNotifications(list, nowMs).filter((notification) => notification.surface === 'toast'))
}

export interface CreateLocalNotificationInput {
  severity: NotificationSeverity
  surface: NotificationSurface
  summary: string
  detail?: string | null
  dedupKey?: string | null
  priority?: number
  ttlMs?: number | null
  code?: string
}

/**
 * Frontend-originated notification. `source = frontend`; it lives only in the
 * client store and is never persisted server-side.
 */
export function createLocalNotification(
  input: CreateLocalNotificationInput,
  id: string,
  nowMs = Date.now(),
): NotificationResource {
  return {
    id,
    kind: { kind: 'notice', code: input.code ?? 'web.local' },
    severity: input.severity,
    scope: 'global',
    surface: input.surface,
    source: 'frontend',
    summary: input.summary,
    detail: input.detail ?? null,
    control: 'dismiss',
    actions: [],
    priority: input.priority ?? 0,
    dedup_key: input.dedupKey ?? null,
    created_at_ms: nowMs,
    expires_at_ms: input.ttlMs != null ? nowMs + input.ttlMs : null,
    dismissed: false,
  }
}

export interface EmitLocalInput extends LocalNotificationOptions {
  severity: NotificationSeverity
  surface: NotificationSurface
  summary: string
}

/**
 * Push one local notification into the local list. Banner is a single slot
 * (the legacy chat banner was `errorMessage` XOR `localCommandNotice`), so a
 * new local banner replaces previous local banners. Expired entries are pruned.
 */
export function pushLocalNotification(
  current: NotificationResource[],
  input: EmitLocalInput,
  id: string,
  nowMs = Date.now(),
): NotificationResource[] {
  const notification = createLocalNotification(
    {
      severity: input.severity,
      surface: input.surface,
      summary: input.summary,
      detail: input.detail,
      dedupKey: input.dedupKey ?? null,
      priority: input.priority,
      ttlMs: input.ttlMs ?? null,
    },
    id,
    nowMs,
  )

  let next: NotificationResource[]
  if (notification.surface === 'banner') {
    next = [notification]
  } else if (notification.dedup_key) {
    next = upsertNotification(
      current.filter((entry) => entry.dedup_key !== notification.dedup_key),
      notification,
    )
  } else {
    next = [...current, notification]
  }
  return activeNotifications(next, nowMs)
}
