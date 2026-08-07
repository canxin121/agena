<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'

import { dispatchNotificationAction } from './notificationActions'
import type { NotificationAction } from './types'
import { useNotifications } from './useNotifications'

const router = useRouter()
const { banner, dismiss, resolveAction, notify } = useNotifications()

const top = computed(() => banner.value[0] ?? null)

async function handleAction(action: NotificationAction) {
  if (!top.value) return
  const target = await resolveAction(top.value.id, action.id)
  if (!target) return
  await dispatchNotificationAction(target, router, (message) => notify.notice(message))
}
</script>

<template>
  <div v-if="top" class="notification-banner" :class="`notification-${top.severity}`" role="status">
    <div class="notification-banner-copy">
      <div class="notification-banner-summary">{{ top.summary }}</div>
      <div v-if="top.detail" class="notification-banner-detail">{{ top.detail }}</div>
    </div>
    <div class="notification-banner-actions">
      <button
        v-for="action in top.actions"
        :key="action.id"
        type="button"
        class="button small"
        @click="handleAction(action)"
      >
        {{ action.label }}
      </button>
      <button v-if="top.control === 'dismiss'" type="button" class="button small ghost" @click="dismiss(top.id)">
        Dismiss
      </button>
    </div>
  </div>
</template>

<style scoped>
.notification-banner {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 12px;
  align-items: center;
  padding: 12px 14px;
  border-radius: var(--radius, 10px);
  border: 1px solid rgba(245, 213, 155, 0.9);
  background: var(--warning-soft, #fdf6e3);
  color: #5d4300;
}

.notification-banner-error {
  background: rgba(220, 38, 38, 0.08);
  border-color: rgba(220, 38, 38, 0.28);
  color: #7f1d1d;
}

.notification-banner-success {
  background: rgba(22, 163, 74, 0.08);
  border-color: rgba(22, 163, 74, 0.28);
  color: #14532d;
}

.notification-banner-info {
  background: rgba(14, 116, 144, 0.08);
  border-color: rgba(14, 116, 144, 0.28);
  color: #164e63;
}

.notification-banner-copy {
  min-width: 0;
}

.notification-banner-summary {
  line-height: 1.4;
}

.notification-banner-detail {
  margin-top: 4px;
  font-size: 13px;
  opacity: 0.85;
}

.notification-banner-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}
</style>
