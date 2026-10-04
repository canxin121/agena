<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useChatStore } from '@/stores/chat'
import { apiJson } from '@/lib/api'
import { sessionStateIsBusy, sessionStateNeedsAttention, sessionStateRequests } from '@/types/chat'
import { activityIsActive, mergeActivityLog, type ActivityLog, type SessionActivity } from '@/types/activity'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import { attentionRequestId } from '@/pages/chat/transcriptPartPresentation'
import SessionSection from './SessionSection.vue'
import Button from '@/components/ui/Button.vue'

const props = defineProps<{
  sessionId: string
  retry?: { attempt: number; message: string; next: number } | null
  countdown?: string
}>()
defineEmits<{ revealRequest: [id: string] }>()
const { t } = useI18n()
const chat = useChatStore()
const expanded = ref(false)
const selected = ref<SessionActivity | null>(null)
const mutation = ref('')
const mutationError = ref('')
const limit = ref(20)
const activities = computed(() => chat.sessionBackgroundActivities(props.sessionId))
// Reconcile ephemeral provider backoff as well as task controls. The store
// coalesces this with SSE-driven refreshes; hidden tabs suspend the heartbeat.
useVisibleResource<boolean>({
  key: computed(() =>
    sessionStateIsBusy(chat.selectedSessionState) ||
    sessionStateNeedsAttention(chat.selectedSessionState) ||
    activities.value.length
      ? props.sessionId
      : '',
  ),
  interval: () => 2500,
  load: async (sid) => {
    await chat.refreshExecutionStatus(sid)
    return true
  },
})
const requests = computed(() => sessionStateRequests(chat.selectedSessionState))
const attentionId = computed(() => attentionRequestId(chat.selectedAttention?.payload))
const pendingSummary = computed(() => {
  const counts = { permission: 0, review: 0, question: 0 }
  for (const request of requests.value) {
    if (!request || typeof request !== 'object' || Array.isArray(request)) continue
    if (request.kind === 'permission') counts.permission++
    else if (request.input_kind === 'review') counts.review++
    else counts.question++
  }
  return Object.entries(counts)
    .filter(([, count]) => count)
    .map(([kind, count]) => t(`chat.sessionWork.${kind}`, { count }))
    .join(' · ')
})
const logKey = computed(() => (expanded.value && selected.value ? `${props.sessionId}/${selected.value.id}` : ''))
const logs = useVisibleResource<ActivityLog>({
  key: logKey,
  interval: () =>
    logs.data.value?.has_more || activityIsActive(logs.data.value?.status || selected.value?.status || '')
      ? 2000
      : 30_000,
  async load(key, signal, previous) {
    const id = key.slice(key.indexOf('/') + 1)
    const next = await apiJson<ActivityLog>(
      `/api/v1/activities/${encodeURIComponent(id)}/logs?since_seq=${previous?.last_seq ?? Math.max(0, (selected.value?.last_seq || 0) - 200)}&limit=200&wait_ms=0`,
      { signal },
    )
    return mergeActivityLog(previous, next)
  },
})
watch(activities, (rows) => {
  const current = rows.find((row) => row.id === selected.value?.id)
  if (current) selected.value = current
})
async function control(activity: SessionActivity, action: string) {
  if (mutation.value || !activity.controls.includes(action)) return
  mutation.value = activity.id
  mutationError.value = ''
  const sid = props.sessionId
  try {
    const result = await apiJson<SessionActivity>(
      `/api/v1/activities/${encodeURIComponent(activity.id)}/${encodeURIComponent(action)}`,
      { method: 'POST', signal: AbortSignal.timeout(15_000) },
    )
    if (props.sessionId === sid && selected.value?.id === activity.id) selected.value = result
    await chat.refreshExecutionStatus(sid)
  } catch (cause) {
    if (props.sessionId === sid) mutationError.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    mutation.value = ''
  }
}
</script>

<template>
  <div
    v-if="requests.length || retry"
    class="my-3 rounded-lg border border-border/60 px-3 py-2 text-xs"
    data-transcript-chrome="true"
    aria-live="polite"
  >
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <Button v-if="attentionId" size="sm" variant="secondary" @click="$emit('revealRequest', attentionId)">{{
        t('chat.sessionWork.resolve')
      }}</Button>
      <span v-if="pendingSummary" class="font-medium">{{ pendingSummary }}</span>
      <span v-else-if="retry" class="font-medium"
        >{{ t('chat.sessionWork.retry', { attempt: retry.attempt }) }} · {{ countdown }}</span
      >
    </div>
    <p v-if="retry?.message" class="mt-1 line-clamp-3 break-words text-muted-foreground">{{ retry.message }}</p>
  </div>
  <SessionSection
    v-if="activities.length || selected"
    v-model:expanded="expanded"
    :title="t('chat.sessionWork.tasks')"
    :summary="t('chat.sessionWork.activeTasks', { count: activities.length })"
  >
    <div class="max-h-[min(45dvh,28rem)] space-y-2 overflow-auto overscroll-contain">
      <p v-if="mutationError" role="alert" class="text-xs text-destructive">{{ mutationError }}</p>
      <div
        v-for="activity in activities.slice(0, limit)"
        :key="activity.id"
        class="min-w-0 rounded-md border border-border/40 p-2"
      >
        <div class="flex flex-wrap items-center gap-2 text-xs">
          <button
            type="button"
            class="min-h-8 rounded px-2 text-left font-medium hover:bg-secondary/50 focus-visible:ring-2 focus-visible:ring-ring"
            :aria-expanded="selected?.id === activity.id"
            @click="selected = selected?.id === activity.id ? null : activity"
          >
            {{ t('chat.sessionWork.logs') }}
          </button>
          <Button
            v-for="action in activity.controls"
            :key="action"
            size="sm"
            variant="ghost"
            :disabled="Boolean(mutation)"
            @click="control(activity, action)"
            >{{ t(`chat.sessionWork.${action}`) }}</Button
          >
          <span class="text-muted-foreground">{{ t(`chat.sessionWork.status.${activity.status}`) }}</span>
          <span class="min-w-0 flex-1 truncate font-medium" :title="activity.title">{{ activity.title }}</span>
        </div>
        <p v-if="activity.failure?.user.fallback" class="mt-1 line-clamp-2 text-xs text-destructive">
          {{ activity.failure.user.fallback }}
        </p>
        <p v-if="activity.message" class="mt-1 line-clamp-2 text-xs text-muted-foreground">{{ activity.message }}</p>
        <p v-if="activity.next_event_at_ms && activity.status === 'waiting'" class="text-xs text-muted-foreground">
          {{ t('chat.sessionWork.nextWake') }} {{ new Date(activity.next_event_at_ms).toLocaleString() }}
        </p>
      </div>
      <Button v-if="activities.length > limit" size="sm" variant="ghost" @click="limit += 20">{{
        t('chat.sessionWork.more')
      }}</Button>
      <div v-if="selected" class="min-w-0 border-t border-border/50 pt-2">
        <div class="mb-2 flex flex-wrap items-center gap-2 text-xs">
          <Button size="sm" variant="ghost" @click="selected = null">{{ t('chat.sessionWork.close') }}</Button>
          <Button size="sm" variant="ghost" :disabled="logs.loading.value" @click="logs.refresh()">{{
            t('chat.sessionWork.refresh')
          }}</Button>
          <span class="min-w-0 flex-1 truncate">{{ selected.title }}</span>
          <span v-if="logs.data.value"
            >{{ t(`chat.sessionWork.status.${logs.data.value.status}`)
            }}<template v-if="logs.data.value.exit_code != null">
              · exit {{ logs.data.value.exit_code }}</template
            ></span
          >
        </div>
        <p
          v-if="selected.description && selected.description !== selected.title"
          class="mb-2 whitespace-pre-wrap break-words text-xs text-muted-foreground"
        >
          {{ selected.description }}
        </p>
        <p v-if="logs.error.value" role="alert" class="text-xs text-destructive">{{ logs.error.value }}</p>
        <p class="mb-1 text-xs text-muted-foreground">{{ t('chat.sessionWork.logTail') }}</p>
        <pre
          class="max-h-60 overflow-auto whitespace-pre-wrap break-all rounded bg-muted/40 p-2 font-mono text-xs"
          :aria-busy="logs.loading.value"
          >{{
            logs.data.value?.lines
              .map((line) => (line.stream === 'stderr' ? 'stderr › ' : '') + line.text)
              .join('\n') || t(logs.loading.value ? 'chat.sessionWork.loading' : 'chat.sessionWork.noOutput')
          }}</pre
        >
        <p v-if="logs.data.value?.dropped_lines" class="text-xs text-muted-foreground">
          {{ t('chat.sessionWork.dropped', { count: logs.data.value.dropped_lines }) }}
        </p>
      </div>
    </div>
  </SessionSection>
</template>
