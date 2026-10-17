<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine } from '@remixicon/vue'
import { useActivityLogs } from '@/composables/useActivityLogs'
import { activityLogText, type SessionActivity } from '@/types/activity'
import Button from '@/components/ui/Button.vue'

const props = defineProps<{ sessionId: string; activity: SessionActivity }>()
const { t } = useI18n()
const key = computed(() => (props.sessionId ? `${props.sessionId}/${props.activity.id}` : ''))
const logs = useActivityLogs(key, () => props.activity)
</script>

<template>
  <div class="my-1 min-w-0" data-activity-output>
    <div class="part-toolbar text-xs text-muted-foreground">
      <div class="flex min-w-0 flex-wrap items-center gap-2">
        <span>{{ t('chat.sessionWork.logTail') }}</span>
        <span>{{ t(`chat.sessionWork.status.${logs.data.value?.status ?? activity.status}`) }}</span>
        <span v-if="logs.data.value?.exit_code != null"
          >· {{ t('chat.activityLog.exitCode', { code: logs.data.value.exit_code }) }}</span
        >
      </div>
      <Button size="sm" variant="outline" class="part-action" :disabled="logs.loading.value" @click="logs.refresh()">
        <RiRefreshLine :class="logs.loading.value ? 'animate-spin' : ''" aria-hidden="true" />
        {{ t('chat.sessionWork.refresh') }}
      </Button>
    </div>
    <p v-if="logs.error.value" role="alert" class="px-2 py-1 text-xs text-destructive">{{ logs.error.value }}</p>
    <pre
      tabindex="0"
      data-transcript-keyboard-owner="true"
      :aria-label="t('chat.sessionWork.logTail')"
      class="max-h-60 overflow-auto whitespace-pre-wrap break-all bg-background p-2 font-mono text-xs"
      :aria-busy="logs.loading.value && !logs.data.value"
      >{{
        activityLogText(logs.data.value) ||
        t(logs.loading.value && !logs.data.value ? 'chat.sessionWork.loading' : 'chat.sessionWork.noOutput')
      }}</pre
    >
    <p v-if="logs.data.value?.dropped_lines" class="px-2 py-1 text-xs text-muted-foreground">
      {{ t('chat.sessionWork.dropped', { count: logs.data.value.dropped_lines }) }}
    </p>
  </div>
</template>
