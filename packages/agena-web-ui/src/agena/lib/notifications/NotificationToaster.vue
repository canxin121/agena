<script setup lang="ts">
import { onBeforeUnmount, watch } from 'vue'
import { useRouter } from 'vue-router'

import { dispatchNotificationAction } from './notificationActions'
import type { NotificationAction } from './types'
import { useNotifications } from './useNotifications'

const router = useRouter()
const { toasts, dismiss, resolveAction, notify } = useNotifications()

const timers = new Map<string, ReturnType<typeof setTimeout>>()

function scheduleAutoDismiss(id: string, expiresAtMs: number | null | undefined) {
  if (timers.has(id)) return
  const ttl = expiresAtMs != null ? Math.max(0, expiresAtMs - Date.now()) : 5_000
  timers.set(
    id,
    setTimeout(() => {
      timers.delete(id)
      void dismiss(id)
    }, ttl),
  )
}

watch(
  toasts,
  (list) => {
    const ids = new Set(list.map((toast) => toast.id))
    for (const [id, timer] of timers) {
      if (!ids.has(id)) {
        clearTimeout(timer)
        timers.delete(id)
      }
    }
    for (const toast of list) scheduleAutoDismiss(toast.id, toast.expires_at_ms)
  },
  { immediate: true, deep: true },
)

onBeforeUnmount(() => {
  for (const timer of timers.values()) clearTimeout(timer)
  timers.clear()
})

async function handleAction(toast: { id: string }, action: NotificationAction) {
  const target = await resolveAction(toast.id, action.id)
  if (!target) return
  await dispatchNotificationAction(target, router, (message) => notify.notice(message))
}
</script>

<template>
  <div v-if="toasts.length" class="notification-toaster" aria-live="polite">
    <div
      v-for="toast in toasts"
      :key="toast.id"
      class="notification-toast"
      :class="`notification-${toast.severity}`"
      role="status"
    >
      <div class="notification-toast-copy">
        <div class="notification-toast-summary">{{ toast.summary }}</div>
        <div v-if="toast.detail" class="notification-toast-detail">{{ toast.detail }}</div>
      </div>
      <div class="notification-toast-actions">
        <button
          v-for="action in toast.actions"
          :key="action.id"
          type="button"
          class="button small"
          @click="handleAction(toast, action)"
        >
          {{ action.label }}
        </button>
        <button type="button" class="notification-toast-close" @click="dismiss(toast.id)">Close</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.notification-toaster {
  position: fixed;
  top: 16px;
  right: 16px;
  z-index: 1000;
  display: grid;
  gap: 10px;
  width: min(360px, calc(100vw - 32px));
}

.notification-toast {
  display: grid;
  gap: 8px;
  padding: 12px 14px;
  border-radius: 12px;
  border: 1px solid rgba(15, 23, 42, 0.12);
  background: rgba(255, 255, 255, 0.96);
  box-shadow: 0 12px 32px rgba(15, 23, 42, 0.16);
  backdrop-filter: blur(12px);
}

.notification-toast.notification-info {
  border-color: rgba(14, 116, 144, 0.24);
}

.notification-toast.notification-success {
  border-color: rgba(22, 163, 74, 0.28);
}

.notification-toast.notification-warning {
  border-color: rgba(245, 158, 11, 0.32);
}

.notification-toast.notification-error {
  border-color: rgba(220, 38, 38, 0.28);
}

.notification-toast-summary {
  line-height: 1.4;
  color: #0f172a;
}

.notification-toast-detail {
  margin-top: 4px;
  font-size: 13px;
  color: #475569;
}

.notification-toast-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}

.notification-toast-close {
  margin-left: auto;
  border: 0;
  background: transparent;
  color: #475569;
  cursor: pointer;
  font: inherit;
  padding: 0;
}
</style>
