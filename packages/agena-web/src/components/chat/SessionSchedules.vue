<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useDocumentVisibility } from '@vueuse/core'
import { useI18n } from 'vue-i18n'
import { RiCalendarScheduleLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import SessionChip from './SessionChip.vue'
import { apiJson, apiResponse } from '@/lib/api'
import { conditionalJson } from '@/lib/conditionalJson'
import {
  captureResourceObservation,
  checkResourceVersions,
  invalidateResources,
  subscribeResource,
} from '@/lib/resourceSync'
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
const visibility = useDocumentVisibility()
const enabled = computed(() => visibility.value !== 'hidden' && (pane?.isVisible.value ?? true))
const endpoint = `/api/v1/sessions/${encodeURIComponent(props.sessionId)}/schedules`
const resource = `session:${props.sessionId}:schedules`
const lifecycle = new AbortController()
let mutation = 0

function date(value: string) {
  return new Date(value).toLocaleString(locale.value)
}

const title = computed(() => t('chat.operations.schedules'))
const label = computed(() => (schedules.value?.length ? `${title.value} · ${schedules.value.length}` : title.value))
const caption = computed(() => {
  if (error.value) return t('common.retry')
  const jobs = schedules.value ?? []
  const next = jobs
    .flatMap((job) => (!job.paused && !job.completed && job.next_fire_at ? [job.next_fire_at] : []))
    .sort()[0]
  if (next) return t('chat.operations.nextRun', { time: date(next) })
  if (jobs.some((job) => job.paused && !job.completed)) return t('chat.operations.paused')
  if (jobs.length && jobs.every((job) => job.completed)) return t('chat.partStatus.completed')
  return ''
})

const reader = createRevalidator(
  async () => {
    const scope = captureResourceObservation(resource).scope
    const revision = mutation
    try {
      const signal = AbortSignal.any([lifecycle.signal, AbortSignal.timeout(8000)])
      if (schedules.value) await checkResourceVersions([resource], signal)
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
watch(open, (value) => {
  if (value) reader.invalidate(0)
})

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
        if (!schedules.value?.length) open.value = false
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
    if (scope === captureResourceObservation(resource).scope) invalidateResources([resource], 'mutation')
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
    v-if="open || schedules?.length || error"
    :title="title"
    :label="label"
    :hint="[label, caption].filter(Boolean).join(' · ')"
    :open="open"
    data-schedules-chip
    @update:open="open = $event"
  >
    <template #icon>
      <RiCalendarScheduleLine class="h-3 w-3 shrink-0" aria-hidden="true" />
    </template>
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
