<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCalendarScheduleLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import SessionChip from './SessionChip.vue'
import { apiJson, apiResponse } from '@/lib/api'
import { conditionalJson } from '@/lib/conditionalJson'
import { captureResourceObservation, checkResourceVersions, subscribeResource } from '@/lib/resourceSync'
import { createRevalidator } from '@/lib/revalidation'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'

type Schedule = {
  id: string
  kind: 'cron' | 'once'
  prompt: string
  expression?: string
  timezone?: string
  at?: string
  next_fire_at?: string
  paused: boolean
  completed: boolean
  last_run?: { status: string; failure?: { user?: { fallback?: string } } }
}

const props = defineProps<{ sessionId: string }>()
const { t, locale } = useI18n()
const pane = useWorkspacePaneContext()
const open = defineModel<boolean>('open', { default: false })
const schedules = ref<Schedule[]>()
const error = ref('')
const pending = ref('')
const limit = ref(20)
const enabled = computed(() => open.value && (pane?.isVisible.value ?? true))
const endpoint = `/api/v1/sessions/${encodeURIComponent(props.sessionId)}/schedules`
const resource = `session:${props.sessionId}:schedules`
const lifecycle = new AbortController()
let mutation = 0

function date(value: string) {
  return new Date(value).toLocaleString(locale.value)
}

const reader = createRevalidator(
  async () => {
    const scope = captureResourceObservation(resource).scope
    const revision = mutation
    try {
      const signal = AbortSignal.any([lifecycle.signal, AbortSignal.timeout(8000)])
      await checkResourceVersions([resource], signal)
      const rows = await conditionalJson<Schedule[]>(resource, endpoint, { signal })
      if (!signal.aborted && scope === captureResourceObservation(resource).scope && revision === mutation) {
        schedules.value = rows
        error.value = ''
      }
    } catch (cause) {
      if (!lifecycle.signal.aborted) error.value = cause instanceof Error ? cause.message : String(cause)
      throw cause
    }
  },
  { intervalMs: 500, retryMs: 5000, enabled: () => enabled.value && !pending.value },
)
watch(
  enabled,
  (value, _, cleanup) => {
    if (!value) return
    cleanup(subscribeResource(resource, () => reader.invalidate(100)))
    reader.invalidate(0)
  },
  { immediate: true },
)

async function change(schedule: Schedule, action: 'pause' | 'resume' | 'delete') {
  if (pending.value) return
  pending.value = schedule.id
  error.value = ''
  ++mutation
  const scope = captureResourceObservation(resource).scope
  try {
    const url = `${endpoint}/${encodeURIComponent(schedule.id)}`
    const signal = AbortSignal.any([lifecycle.signal, AbortSignal.timeout(8000)])
    if (action === 'delete') {
      await apiResponse(url, { method: 'DELETE', signal })
      if (!signal.aborted && scope === captureResourceObservation(resource).scope) {
        schedules.value = schedules.value?.filter((item) => item.id !== schedule.id)
      }
    } else {
      const updated = await apiJson<Schedule>(url, {
        method: 'PATCH',
        body: JSON.stringify({ paused: action === 'pause' }),
        headers: { 'Content-Type': 'application/json' },
        signal,
      })
      if (!signal.aborted && scope === captureResourceObservation(resource).scope) {
        schedules.value = schedules.value?.map((item) => (item.id === updated.id ? updated : item))
      }
    }
  } catch (cause) {
    if (!lifecycle.signal.aborted) error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    pending.value = ''
    reader.invalidate(250)
  }
}

onBeforeUnmount(() => {
  lifecycle.abort()
  reader.dispose()
})
</script>

<template>
  <SessionChip
    :title="t('chat.operations.schedules')"
    :label="t('chat.operations.schedules')"
    :open="open"
    @update:open="open = $event"
  >
    <template #icon><RiCalendarScheduleLine class="h-3 w-3 shrink-0" /></template>
    <div class="space-y-2 text-xs">
      <p v-if="error" class="text-destructive" role="alert">{{ error }}</p>
      <p v-else-if="!schedules" class="text-muted-foreground">{{ t('common.loading') }}</p>
      <p v-else-if="!schedules.length" class="text-muted-foreground">{{ t('chat.operations.noSchedules') }}</p>
      <article v-for="schedule in schedules?.slice(0, limit)" :key="schedule.id" class="space-y-2 border-b pb-2">
        <p class="break-words font-medium">{{ schedule.prompt }}</p>
        <p class="flex flex-wrap items-center gap-x-2 text-muted-foreground">
          <span v-if="schedule.kind === 'cron'" class="font-mono"
            >{{ schedule.expression }} {{ schedule.timezone }}</span
          >
          <span v-else-if="schedule.at && !schedule.next_fire_at">{{ date(schedule.at) }}</span>
          <span v-if="schedule.completed">{{
            t(schedule.last_run?.status === 'failed' ? 'chat.partStatus.failed' : 'chat.partStatus.completed')
          }}</span>
          <span v-else-if="schedule.paused">{{ t('chat.operations.paused') }}</span>
          <span v-else-if="schedule.next_fire_at">{{
            t('chat.operations.nextRun', { time: date(schedule.next_fire_at) })
          }}</span>
        </p>
        <p v-if="schedule.last_run?.failure?.user?.fallback" class="text-destructive">
          {{ schedule.last_run.failure.user.fallback }}
        </p>
        <div class="flex items-center gap-2">
          <Button
            v-if="!schedule.completed"
            size="sm"
            variant="outline"
            :disabled="Boolean(pending)"
            @click="change(schedule, schedule.paused ? 'resume' : 'pause')"
            >{{ t(schedule.paused ? 'chat.operations.resume' : 'chat.operations.pause') }}</Button
          >
          <Button size="sm" variant="ghost" :disabled="Boolean(pending)" @click="change(schedule, 'delete')">
            {{ t('common.delete') }}
          </Button>
        </div>
      </article>
      <Button v-if="schedules && schedules.length > limit" size="sm" variant="ghost" @click="limit += 20">
        {{ t('chat.operations.showMoreSchedules') }}
      </Button>
    </div>
  </SessionChip>
</template>
