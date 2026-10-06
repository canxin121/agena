<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useActivityLogs } from '@/composables/useActivityLogs'
import { activityLogText, type SessionActivity } from '@/types/activity'
import Button from '@/components/ui/Button.vue'

const props = defineProps<{ sessionId: string; activity: SessionActivity }>()
const { t } = useI18n()
const key = computed(() => props.sessionId ? `${props.sessionId}/${props.activity.id}` : '')
const logs = useActivityLogs(key, () => props.activity)
</script>

<template>
  <div class="min-w-0 space-y-1" data-activity-output>
    <div class="flex items-center gap-2 text-xs text-muted-foreground">
      <span>{{ t('chat.sessionWork.logTail') }}</span>
      <span>{{ t(`chat.sessionWork.status.${logs.data.value?.status ?? activity.status}`) }}</span>
      <span v-if="logs.data.value?.exit_code != null">· exit {{ logs.data.value.exit_code }}</span>
      <Button size="sm" variant="ghost" :disabled="logs.loading.value" @click="logs.refresh()">
        {{ t('chat.sessionWork.refresh') }}
      </Button>
    </div>
    <p v-if="logs.error.value" role="alert" class="text-xs text-destructive">{{ logs.error.value }}</p>
    <pre
      class="max-h-60 overflow-auto whitespace-pre-wrap break-all rounded bg-muted/40 p-2 font-mono text-xs"
      :aria-busy="logs.loading.value"
    >{{ activityLogText(logs.data.value) || t(logs.loading.value ? 'chat.sessionWork.loading' : 'chat.sessionWork.noOutput') }}</pre>
    <p v-if="logs.data.value?.dropped_lines" class="text-xs text-muted-foreground">
      {{ t('chat.sessionWork.dropped', { count: logs.data.value.dropped_lines }) }}
    </p>
  </div>
</template>
