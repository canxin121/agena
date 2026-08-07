// Unified notification store (Phase 4 web migration).
//
// Single module-level store: REST history + SSE live subscription on
// `/api/v1/notifications*`, plus local (frontend-source) notifications emitted
// by page composables. Every caller of `useNotifications()` shares one store.

import { computed, readonly, ref, type ComputedRef } from 'vue'

import { apiJson, apiResponseError, apiText, apiUrl, userErrorMessage } from '@/lib/api'
import { buildActiveUiAuthHeaders } from '@/lib/uiAuthToken'

import { normalizeSseBuffer, parseSseEventBlock } from '../sse'
import {
  mergeServerList,
  pushLocalNotification,
  selectBanners,
  selectToasts,
  sortNotifications,
  upsertNotification,
} from './notificationsModel'
import type {
  LocalNotificationOptions,
  NotificationActionTarget,
  NotificationResource,
  NotificationSeverity,
  NotificationsHandle,
  NotificationStreamClosedPayload,
  NotificationStreamLaggedPayload,
  NotificationStreamResumedPayload,
  PaginatedNotificationsResponse,
} from './types'

const NOTIFICATIONS_PATH = '/api/v1/notifications'
const NOTIFICATIONS_STREAM_PATH = '/api/v1/notifications/stream'
const DEFAULT_LIST_LIMIT = 100

const serverNotifications = ref<NotificationResource[]>([])
const localNotifications = ref<NotificationResource[]>([])
const connected = ref(false)
const loading = ref(false)
const streamError = ref('')

let controller: AbortController | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let stopped = true
let localSequence = 0

function localId(): string {
  localSequence += 1
  return `web-local-${Date.now()}-${localSequence}`
}

function allNotifications(): NotificationResource[] {
  return sortNotifications([...localNotifications.value, ...serverNotifications.value])
}

function latestTimestamp(): number {
  let latest = 0
  for (const notification of allNotifications()) {
    latest = Math.max(latest, notification.created_at_ms)
  }
  return latest
}

async function load(): Promise<void> {
  loading.value = true
  try {
    const response = await apiJson<PaginatedNotificationsResponse>(
      `${NOTIFICATIONS_PATH}?active_only=true&limit=${DEFAULT_LIST_LIMIT}`,
    )
    serverNotifications.value = mergeServerList(serverNotifications.value, response.items ?? [])
    streamError.value = ''
  } catch (error) {
    streamError.value = userErrorMessage(error, 'Notifications could not be loaded.')
  } finally {
    loading.value = false
  }
}

function handleStreamBlock(block: string): void {
  const parsed = parseSseEventBlock(block)
  if (!parsed.data) return

  switch (parsed.event) {
    case 'notification': {
      let next: NotificationResource
      try {
        next = JSON.parse(parsed.data) as NotificationResource
      } catch {
        return
      }
      if (next && typeof next.id === 'string') {
        serverNotifications.value = upsertNotification(serverNotifications.value, next)
      }
      return
    }
    case 'lagged': {
      // The broadcast dropped messages; backfill with a fresh list request.
      let skipped = 0
      try {
        const payload = JSON.parse(parsed.data) as NotificationStreamLaggedPayload
        skipped = Number.isFinite(payload.skipped) ? payload.skipped : 0
      } catch {
        // ignore malformed lagged payload
      }
      if (skipped > 0) streamError.value = `${skipped} notifications were skipped; reloading.`
      void load()
      return
    }
    case 'resumed': {
      // History replay finished; live events continue. The resume watermark is
      // derived from `latestTimestamp()` on every reconnect, so nothing to store.
      try {
        const payload = JSON.parse(parsed.data) as NotificationStreamResumedPayload
        if (Number.isFinite(payload.up_to_ms)) {
          // informational only
        }
      } catch {
        // ignore malformed resumed payload
      }
      return
    }
    case 'subscription_closed': {
      let reason = 'notification store closed'
      try {
        const payload = JSON.parse(parsed.data) as NotificationStreamClosedPayload
        if (typeof payload.reason === 'string' && payload.reason) reason = payload.reason
      } catch {
        // ignore malformed close payload
      }
      streamError.value = reason
      scheduleReconnect(1_000)
      return
    }
    default:
      return
  }
}

async function readResponseStream(response: Response): Promise<void> {
  const reader = response.body?.getReader()
  if (!reader) throw new Error('Notification stream response body is unavailable')

  const decoder = new TextDecoder()
  let buffer = ''
  while (!stopped) {
    const { done, value } = await reader.read()
    buffer = normalizeSseBuffer(buffer + decoder.decode(value ?? new Uint8Array(), { stream: !done }))

    let boundary = buffer.indexOf('\n\n')
    while (boundary >= 0) {
      const block = buffer.slice(0, boundary).trim()
      buffer = buffer.slice(boundary + 2)
      if (block) handleStreamBlock(block)
      boundary = buffer.indexOf('\n\n')
    }

    if (done) {
      const trailing = buffer.trim()
      if (trailing) handleStreamBlock(trailing)
      return
    }
  }
}

function scheduleReconnect(delayMs: number): void {
  if (stopped || reconnectTimer) return
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null
    void openStream()
  }, delayMs)
}

async function openStream(): Promise<void> {
  if (stopped) return

  await load()
  if (stopped) return

  const authHeaders = buildActiveUiAuthHeaders()
  const url = new URL(apiUrl(NOTIFICATIONS_STREAM_PATH))
  url.searchParams.set('active_only', 'true')
  const since = latestTimestamp()
  if (since > 0) url.searchParams.set('since_ms', String(since))

  controller = new AbortController()
  try {
    const response = await fetch(url.toString(), {
      method: 'GET',
      signal: controller.signal,
      credentials: authHeaders.authorization ? 'omit' : 'include',
      headers: {
        accept: 'text/event-stream',
        ...(authHeaders.authorization ? authHeaders : {}),
      },
    })
    if (!response.ok) throw await apiResponseError(response, url.toString())

    connected.value = true
    streamError.value = ''
    await readResponseStream(response)
    connected.value = false
    if (!stopped) scheduleReconnect(1_000)
  } catch (error) {
    if (stopped || controller?.signal.aborted) return
    connected.value = false
    streamError.value = userErrorMessage(error, 'Live notifications were interrupted. Reconnecting…')
    scheduleReconnect(2_000)
  }
}

function notifyLocal(options: LocalNotificationOptions & { severity: NotificationSeverity; summary: string }): void {
  localNotifications.value = pushLocalNotification(
    localNotifications.value,
    {
      severity: options.severity,
      surface: options.surface ?? 'banner',
      summary: options.summary,
      detail: options.detail,
      dedupKey: options.dedupKey,
      priority: options.priority,
      ttlMs: options.ttlMs,
    },
    localId(),
  )
}

function error(message: string, options?: LocalNotificationOptions): void {
  notifyLocal({ severity: 'error', summary: message, ...options })
}

function notice(message: string, options?: LocalNotificationOptions): void {
  notifyLocal({ severity: 'info', summary: message, ...options })
}

function success(message: string, options?: LocalNotificationOptions): void {
  notifyLocal({ severity: 'success', summary: message, ...options })
}

function toast(severity: NotificationSeverity, message: string, ttlMs?: number): void {
  notifyLocal({ severity, summary: message, surface: 'toast', ttlMs })
}

function clearBanner(): void {
  localNotifications.value = localNotifications.value.filter((entry) => entry.surface !== 'banner')
}

async function dismiss(id: string): Promise<void> {
  const local = localNotifications.value.find((entry) => entry.id === id)
  if (local) {
    localNotifications.value = localNotifications.value.filter((entry) => entry.id !== id)
    return
  }

  const target = serverNotifications.value.find((entry) => entry.id === id)
  if (!target) return

  serverNotifications.value = serverNotifications.value.map((entry) =>
    entry.id === id ? { ...entry, dismissed: true } : entry,
  )
  try {
    await apiText(`${NOTIFICATIONS_PATH}/${encodeURIComponent(id)}/dismiss`, { method: 'POST' })
  } catch (dismissError) {
    serverNotifications.value = serverNotifications.value.map((entry) => (entry.id === id ? target : entry))
    streamError.value = userErrorMessage(dismissError, 'The notification could not be dismissed.')
  }
}

async function resolveAction(notificationId: string, actionId: string): Promise<NotificationActionTarget | null> {
  try {
    return await apiJson<NotificationActionTarget>(
      `${NOTIFICATIONS_PATH}/${encodeURIComponent(notificationId)}/actions/${encodeURIComponent(actionId)}`,
      { method: 'POST' },
    )
  } catch (actionError) {
    streamError.value = userErrorMessage(actionError, 'The notification action could not be resolved.')
    return null
  }
}

export interface NotificationsStore {
  notifications: ComputedRef<NotificationResource[]>
  banner: ComputedRef<NotificationResource[]>
  toasts: ComputedRef<NotificationResource[]>
  connected: Readonly<typeof connected>
  loading: Readonly<typeof loading>
  streamError: Readonly<typeof streamError>
  start: () => void
  stop: () => void
  load: () => Promise<void>
  dismiss: (id: string) => Promise<void>
  resolveAction: (notificationId: string, actionId: string) => Promise<NotificationActionTarget | null>
  notify: NotificationsHandle
}

export function useNotifications(): NotificationsStore {
  function start(): void {
    if (!stopped) return
    stopped = false
    void openStream()
  }

  function stop(): void {
    stopped = true
    controller?.abort()
    controller = null
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
    connected.value = false
  }

  return {
    notifications: computed(() => allNotifications()),
    banner: computed(() => selectBanners(allNotifications())),
    toasts: computed(() => selectToasts(allNotifications())),
    connected: readonly(connected),
    loading: readonly(loading),
    streamError: readonly(streamError),
    start,
    stop,
    load,
    dismiss,
    resolveAction,
    notify: { error, notice, success, toast, clearBanner },
  }
}
