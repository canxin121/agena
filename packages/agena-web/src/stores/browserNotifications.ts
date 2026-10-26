import { computed, onScopeDispose, ref } from 'vue'
import { defineStore } from 'pinia'

import { i18n } from '@/i18n'
import {
  browserNotificationPermission,
  requestBrowserNotificationPermission,
  showBrowserNotification,
} from '@/lib/browserNotifications'
import { useSettingsStore } from './settings'
import { useToastsStore } from './toasts'

export const useBrowserNotificationsStore = defineStore('browserNotifications', () => {
  const settings = useSettingsStore()
  const toasts = useToastsStore()
  const permission = ref(browserNotificationPermission())
  const requesting = ref(false)
  const enabled = computed(() => settings.data?.browserNotificationsEnabled !== false && permission.value === 'granted')
  let permissionStatus: PermissionStatus | undefined
  let disposed = false

  function refreshPermission() {
    permission.value = browserNotificationPermission()
  }

  if (typeof window !== 'undefined') window.addEventListener('focus', refreshPermission)
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
      toasts.push(
        granted ? 'success' : 'info',
        String(
          i18n.global.t(granted ? 'browserNotifications.enabled' : `browserNotifications.status.${permission.value}`),
        ),
        5000,
      )
    } finally {
      requesting.value = false
    }
  }

  function setEnabled(value: boolean) {
    if (value) return enable()
    return settings.save({ browserNotificationsEnabled: false, browserNotificationsPrompted: true })
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

  function test() {
    refreshPermission()
    const notification = showBrowserNotification('Agena', {
      body: String(i18n.global.t('browserNotifications.testBody')),
      tag: 'agena:notification-test',
    })
    if (!notification) toasts.push('error', String(i18n.global.t('browserNotifications.deliveryFailed')), 5000)
    else window.setTimeout(() => notification.close(), 10_000)
  }

  onScopeDispose(() => {
    disposed = true
    permissionStatus?.removeEventListener('change', refreshPermission)
    if (typeof window !== 'undefined') window.removeEventListener('focus', refreshPermission)
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', refreshPermission)
  })

  return { permission, enabled, requesting, refreshPermission, enable, setEnabled, offerPermission, test }
})
