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
  notifications.enabled ? 'granted' : notifications.permission === 'granted' ? 'disabled' : notifications.permission,
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
      <Button variant="outline" size="sm" :disabled="!notifications.enabled" @click="notifications.test">
        {{ t('browserNotifications.test') }}
      </Button>
    </div>
    <p class="mt-3 text-xs leading-relaxed text-muted-foreground">{{ t('browserNotifications.keepOpen') }}</p>
  </SettingsCard>
</template>
