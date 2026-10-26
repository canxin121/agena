<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiNotification3Line } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsToggle from './SettingsToggle.vue'
import Button from '@/components/ui/Button.vue'
import { useBrowserNotificationsStore } from '@/stores/browserNotifications'

const { t } = useI18n()
const notifications = useBrowserNotificationsStore()
const unavailable = computed(() => ['unsupported', 'insecure', 'denied'].includes(notifications.permission))
const status = computed(() =>
  notifications.enabled
    ? notifications.pushReady
      ? 'granted'
      : 'localOnly'
    : notifications.permission === 'granted'
      ? 'disabled'
      : notifications.permission,
)
</script>

<template>
  <SettingsCard
    :title="t('browserNotifications.title')"
    :description="t('browserNotifications.description')"
    :icon="RiNotification3Line"
  >
    <SettingsRow
      :title="t('browserNotifications.title')"
      :description="t(`browserNotifications.status.${status}`)"
      toggle
    >
      <SettingsToggle
        :model-value="notifications.enabled"
        :label="t('browserNotifications.title')"
        :disabled="unavailable || notifications.requesting"
        @update:model-value="notifications.setEnabled"
      />
    </SettingsRow>
    <div class="mt-3 flex flex-wrap gap-2">
      <Button
        v-if="notifications.permission === 'default'"
        variant="outline"
        size="sm"
        :disabled="notifications.requesting"
        @click="notifications.enable"
      >
        {{ t('browserNotifications.enable') }}
      </Button>
      <Button
        v-if="notifications.enabled && !notifications.pushReady"
        variant="outline"
        size="sm"
        @click="notifications.syncPush"
      >
        {{ t('browserNotifications.reconnect') }}
      </Button>
      <Button
        variant="outline"
        size="sm"
        :disabled="!notifications.enabled || notifications.testing"
        @click="notifications.test"
      >
        {{ t('browserNotifications.test') }}
      </Button>
    </div>
    <p v-if="notifications.pushError" role="alert" class="mt-3 text-xs leading-relaxed text-destructive">
      {{ t(notifications.pushReady ? 'browserNotifications.deliveryFailed' : 'browserNotifications.pushUnavailable') }}
      {{ notifications.pushError }}
    </p>
    <p v-if="notifications.testState" role="status" class="mt-3 text-xs leading-relaxed text-muted-foreground">
      {{ t(`browserNotifications.testStatus.${notifications.testState}`) }}
    </p>
    <p class="mt-3 text-xs leading-relaxed text-muted-foreground">{{ t('browserNotifications.keepOpen') }}</p>
    <p v-if="notifications.platformHint" class="mt-3 text-xs leading-relaxed text-muted-foreground">
      {{ t(`browserNotifications.${notifications.platformHint}`) }}
    </p>
  </SettingsCard>
</template>
