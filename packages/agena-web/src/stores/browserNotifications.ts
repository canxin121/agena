import { computed, onScopeDispose, ref, watch } from 'vue'
import { defineStore } from 'pinia'

import { i18n } from '@/i18n'
import {
  browserNotificationPermission,
  createBrowserNotifier,
  requestBrowserNotificationPermission,
  showBrowserNotification,
  type PluginAttentionNotification,
} from '@/lib/browserNotifications'
import { useSettingsStore } from './settings'
import { useToastsStore } from './toasts'
import { ApiError, apiJson, apiUrl } from '@/lib/api'
import { browserNotificationPlatformHint, subscribeBrowserPush, supportsBrowserPush } from '@/lib/browserPush'

export const useBrowserNotificationsStore = defineStore('browserNotifications', () => {
  const settings = useSettingsStore()
  const toasts = useToastsStore()
  const permission = ref(browserNotificationPermission())
  const requesting = ref(false)
  const pushReady = ref(false)
  const pushError = ref('')
  const testing = ref(false)
  const platformHint = computed(() => browserNotificationPlatformHint())
  const testState = ref<'sending' | 'delivered' | 'timeout' | 'failed' | ''>('')
  const enabled = computed(() => settings.data?.browserNotificationsEnabled !== false && permission.value === 'granted')
  let permissionStatus: PermissionStatus | undefined
  let disposed = false
  let subscription: PushSubscription | null = null
  let syncing: Promise<void> | undefined
  let retryTimer: ReturnType<typeof setTimeout> | undefined
  let failures = 0
  let resync = false
  const receipts = new Map<string, { shown: boolean; error: string }>()
  const receiptListeners = new Set<() => void>()
  let pageTimer: ReturnType<typeof setTimeout> | undefined
  let pollingPage = false
  const pageNotifier = createBrowserNotifier({
    enabled: () => enabled.value && !pushReady.value,
    scope: () => apiUrl('/'),
    message: (kind) => String(i18n.global.t(`browserNotifications.events.${kind}`)),
    openSession: (sid) => window.location.assign(`/chat?sessionId=${encodeURIComponent(sid)}`),
  })

  async function pollPageNotifications() {
    if (pollingPage || disposed || !enabled.value || pushReady.value) return
    pollingPage = true
    try {
      const snapshot = await apiJson<{ notifications: PluginAttentionNotification[]; snapshot_at_ms: number }>(
        '/api/v1/browser-notifications/events',
        {
          signal: AbortSignal.timeout(15_000),
        },
      )
      await pageNotifier.applyNotifications(snapshot.notifications, snapshot.snapshot_at_ms)
    } catch {
      // Failed local delivery remains eligible on the next plugin snapshot.
    } finally {
      pollingPage = false
      if (!disposed && enabled.value && !pushReady.value)
        pageTimer = setTimeout(() => void pollPageNotifications(), 2000)
    }
  }

  function onWorkerMessage(event: MessageEvent) {
    if (!['agena:notification-shown', 'agena:notification-error'].includes(event.data?.type)) return
    if (typeof event.data.id !== 'string') return
    receipts.set(event.data.id, {
      shown: event.data.type === 'agena:notification-shown',
      error: typeof event.data.error === 'string' ? event.data.error.slice(0, 500) : '',
    })
    if (receipts.size > 64) receipts.delete(receipts.keys().next().value!)
    for (const listener of receiptListeners) listener()
  }
  if (supportsBrowserPush()) navigator.serviceWorker.addEventListener('message', onWorkerMessage)

  async function syncOnce() {
    if (!supportsBrowserPush()) {
      if (enabled.value) throw new Error(String(i18n.global.t('browserNotifications.pushUnavailable')))
      return
    }
    if (!enabled.value) {
      pushReady.value = false
      pushError.value = ''
      testState.value = ''
      const registration = await navigator.serviceWorker.getRegistration('/')
      const existing = subscription || (await registration?.pushManager.getSubscription())
      if (existing) {
        try {
          await apiJson('/api/v1/browser-notifications/subscriptions', {
            method: 'DELETE',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({ endpoint: existing.endpoint }),
            signal: AbortSignal.timeout(15_000),
          })
        } finally {
          // Revoke at the browser even when the server is unreachable.
          await existing.unsubscribe()
          subscription = null
        }
      }
      return
    }
    const config = await apiJson<{ public_key: string }>('/api/v1/browser-notifications/config', {
      signal: AbortSignal.timeout(15_000),
    })
    subscription = await subscribeBrowserPush(config.public_key)
    if (!enabled.value || disposed) {
      resync = true
      return
    }
    const messages = Object.fromEntries(
      ['completed', 'failed', 'cancelled', 'permission', 'question'].map((kind) => [
        kind,
        String(i18n.global.t(`browserNotifications.events.${kind}`)),
      ]),
    )
    await apiJson('/api/v1/browser-notifications/subscriptions', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        subscription: subscription.toJSON(),
        messages: { ...messages, test: String(i18n.global.t('browserNotifications.testBody')) },
      }),
      signal: AbortSignal.timeout(15_000),
    })
    if (!enabled.value || disposed) {
      resync = true
      return
    }
    pushReady.value = true
    pushError.value = ''
    failures = 0
  }

  function syncPush(): Promise<void> {
    if (disposed) return Promise.resolve()
    if (syncing) {
      resync = true
      return syncing
    }
    clearTimeout(retryTimer)
    syncing = (async () => {
      do {
        resync = false
        clearTimeout(retryTimer)
        try {
          await syncOnce()
        } catch (error) {
          pushReady.value = false
          pushError.value = error instanceof Error ? error.message : String(error)
          const needsUserAction =
            (error instanceof ApiError && [401, 403, 404].includes(error.status)) ||
            (error instanceof DOMException && ['NotAllowedError', 'SecurityError'].includes(error.name))
          if (!disposed && enabled.value && supportsBrowserPush() && !needsUserAction) {
            // Push brokers can take longer to initialize than the page. A
            // transient Firefox lookup/Chrome registration failure must not
            // leave background notifications disconnected until another visit.
            retryTimer = setTimeout(
              () => {
                if (!disposed && enabled.value) void syncPush()
              },
              Math.min(60_000, 5000 * 2 ** Math.min(failures++, 4)),
            )
          }
        }
      } while (resync && !disposed)
    })().finally(() => {
      syncing = undefined
    })
    return syncing
  }

  function refreshPermission() {
    permission.value = browserNotificationPermission()
  }

  function resume() {
    refreshPermission()
    if (enabled.value) void syncPush()
  }

  if (typeof window !== 'undefined') window.addEventListener('focus', resume)
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', refreshPermission)
  if (typeof navigator !== 'undefined' && navigator.permissions?.query) {
    void navigator.permissions
      .query({ name: 'notifications' })
      .then((status) => {
        if (disposed) return
        permissionStatus = status
        status.addEventListener('change', refreshPermission)
        refreshPermission()
      })
      .catch(() => {})
  }

  async function enable() {
    if (requesting.value) return
    requesting.value = true
    try {
      // Invoke before awaiting persistence so the browser retains user activation.
      permission.value = await requestBrowserNotificationPermission()
      await settings.save({
        browserNotificationsEnabled: permission.value === 'granted',
        browserNotificationsPrompted: true,
      })
      const granted = permission.value === 'granted'
      if (granted) await syncPush()
      toasts.push(
        granted && pushReady.value ? 'success' : 'info',
        String(
          i18n.global.t(
            granted
              ? pushReady.value
                ? 'browserNotifications.enabled'
                : 'browserNotifications.pushUnavailable'
              : `browserNotifications.status.${permission.value}`,
          ),
        ),
        5000,
      )
    } finally {
      requesting.value = false
    }
  }

  async function setEnabled(value: boolean) {
    if (value) return enable()
    await settings.save({ browserNotificationsEnabled: false, browserNotificationsPrompted: true })
    await syncPush()
  }

  function offerPermission() {
    refreshPermission()
    if (
      permission.value !== 'default' ||
      settings.data?.browserNotificationsPrompted ||
      settings.data?.browserNotificationsEnabled === false
    )
      return
    void settings.save({ browserNotificationsPrompted: true })
    toasts.push('info', String(i18n.global.t('browserNotifications.offer')), 12_000, {
      label: String(i18n.global.t('browserNotifications.enable')),
      onClick: () => void enable(),
    })
  }

  async function test() {
    if (testing.value) return
    testing.value = true
    testState.value = 'sending'
    refreshPermission()
    try {
      await syncPush()
      if (supportsBrowserPush() && !pushReady.value)
        throw new Error(pushError.value || String(i18n.global.t('browserNotifications.pushUnavailable')))
      if (pushReady.value && subscription) {
        const { id } = await apiJson<{ id: string }>('/api/v1/browser-notifications/test', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ endpoint: subscription.endpoint }),
          signal: AbortSignal.timeout(25_000),
        })
        testState.value = await new Promise<'delivered' | 'failed' | 'timeout'>((resolve) => {
          const timer = setTimeout(() => finish('timeout'), 20_000)
          function finish(result: 'delivered' | 'failed' | 'timeout') {
            clearTimeout(timer)
            receiptListeners.delete(check)
            resolve(result)
          }
          function check() {
            const receipt = receipts.get(id)
            if (!receipt) return
            if (!receipt.shown)
              pushError.value = receipt.error || String(i18n.global.t('browserNotifications.deliveryFailed'))
            finish(receipt.shown ? 'delivered' : 'failed')
          }
          receiptListeners.add(check)
          check()
        })
      } else {
        const notification = await showBrowserNotification('Agena', {
          body: String(i18n.global.t('browserNotifications.testBody')),
          tag: 'agena:notification-test',
          data: { url: '/settings/interface?view=web-appearance' },
        })
        testState.value = notification ? 'delivered' : 'failed'
      }
    } catch (error) {
      testState.value = 'failed'
      pushError.value = error instanceof Error ? error.message : String(error)
    } finally {
      testing.value = false
      if (testState.value === 'failed' || testState.value === 'timeout')
        toasts.push('error', String(i18n.global.t('browserNotifications.deliveryFailed')), 8000)
    }
  }

  watch([enabled, () => i18n.global.locale.value], () => {
    void syncPush()
  })

  watch(enabled, () => pageNotifier.reset())

  watch(
    [enabled, pushReady],
    () => {
      clearTimeout(pageTimer)
      if (enabled.value && !pushReady.value) void pollPageNotifications()
    },
    { immediate: true },
  )

  onScopeDispose(() => {
    disposed = true
    clearTimeout(retryTimer)
    clearTimeout(pageTimer)
    pageNotifier.dispose()
    permissionStatus?.removeEventListener('change', refreshPermission)
    if (typeof window !== 'undefined') window.removeEventListener('focus', resume)
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', refreshPermission)
    if (supportsBrowserPush()) navigator.serviceWorker.removeEventListener('message', onWorkerMessage)
  })

  return {
    permission,
    enabled,
    requesting,
    pushReady,
    pushError,
    testing,
    platformHint,
    testState,
    syncPush,
    refreshPermission,
    enable,
    setEnabled,
    offerPermission,
    test,
  }
})
