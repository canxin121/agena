<script setup lang="ts">
import { computed, onMounted, onScopeDispose, ref, watch } from 'vue'
import { RiRefreshLine, RiPulseLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { activityIsActive } from '@/types/activity'
import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { conditionalJsonObserved } from '../../lib/conditionalJson'
import { clearFinishedActivities, controlActivity, subscribeActivityMutations } from '../../lib/activityApi'
import { createLatestRequestGuard } from '../../lib/latestRequest'
import { createRevalidator } from '../../lib/revalidation'
import { captureResourceObservation, checkResourceVersions, subscribeResource } from '../../lib/resourceSync'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { useToastsStore } from '../../stores/toasts'
import { settingsText as st } from '@/i18n/settingsText'
import type { SseEvent } from '@/lib/sse'

type ActivityControl = 'stop' | 'pause' | 'resume' | 'delete' | 'dismiss'

type Activity = {
  id: string
  kind: string
  status: string
  title: string
  description: string
  session_id?: number | null
  parent_session_id?: number | null
  created_at_ms: number
  message?: string | null
  controls?: ActivityControl[]
}

const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const activities = ref<Activity[]>([])
const busyId = ref<string | null>(null)
const visible = usePaneVisibility()
let readController: AbortController | undefined
let needsBaseline = true
let activityScope = captureResourceObservation('activities').scope
let activityEpoch = captureResourceObservation('activities').epoch
const eventTimes = new Map<string, number>()
const eventTokens = new Map<string, string>()
type RowPatch = { activity: Activity; dismissed: boolean; token?: string }
// Only retain overlays while a full read is in flight. Normal stream updates
// replace one row immediately and never schedule a list request.
const readPatches = new Map<string, RowPatch>()

function ensureActivityScope() {
  const observation = captureResourceObservation('activities')
  if (activityScope === observation.scope && activityEpoch === observation.epoch) return
  const changedBackend =
    activityScope !== observation.scope || Boolean(activityEpoch && activityEpoch !== observation.epoch)
  activityScope = observation.scope
  activityEpoch = observation.epoch
  eventTimes.clear()
  eventTokens.clear()
  readPatches.clear()
  if (changedBackend) {
    activities.value = []
    needsBaseline = true
  }
}

function compareActivities(left: Activity, right: Activity): number {
  return Number(right.created_at_ms || 0) - Number(left.created_at_ms || 0) || left.id.localeCompare(right.id)
}

function patchRows(rows: Activity[], patch: RowPatch): Activity[] {
  const index = rows.findIndex((row) => row.id === patch.activity.id)
  if (patch.dismissed) return index < 0 ? rows : rows.filter((row) => row.id !== patch.activity.id)
  if (index >= 0 && rows[index].created_at_ms === patch.activity.created_at_ms) {
    const next = [...rows]
    next[index] = patch.activity
    return next
  }
  const next = rows.filter((row) => row.id !== patch.activity.id)
  // Creation order is stable through state/log updates. Only insertions or
  // corrected creation timestamps need to find a new position.
  let low = 0
  let high = next.length
  while (low < high) {
    const middle = (low + high) >>> 1
    if (compareActivities(patch.activity, next[middle]) < 0) high = middle
    else low = middle + 1
  }
  next.splice(low, 0, patch.activity)
  return next
}

function applyRow(patch: RowPatch) {
  activities.value = patchRows(activities.value, patch)
  if (readController) readPatches.set(patch.activity.id, patch)
}

function tokenCovers(snapshot: string | undefined, patch: string | undefined): boolean {
  if (!snapshot || !patch) return false
  const separator = snapshot.lastIndexOf(':')
  const patchSeparator = patch.lastIndexOf(':')
  return (
    snapshot.slice(0, separator) === patch.slice(0, patchSeparator) &&
    Number(snapshot.slice(separator + 1)) >= Number(patch.slice(patchSeparator + 1))
  )
}

function applyActivityEvent(event?: SseEvent): boolean {
  if (event?.type !== 'runtime_signal' || event.properties?.kind !== 'activity') return false
  const payload = event.properties.payload
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return false
  const activity = payload.activity
  if (
    !activity ||
    typeof activity !== 'object' ||
    Array.isArray(activity) ||
    typeof activity.id !== 'string' ||
    typeof activity.kind !== 'string' ||
    typeof activity.status !== 'string' ||
    typeof activity.title !== 'string' ||
    typeof activity.description !== 'string' ||
    typeof activity.created_at_ms !== 'number' ||
    !Array.isArray(activity.controls) ||
    typeof payload.ts_ms !== 'number'
  )
    return false
  ensureActivityScope()
  const time = payload.ts_ms
  if (time <= (eventTimes.get(activity.id) ?? -Infinity)) return true
  eventTimes.delete(activity.id)
  eventTimes.set(activity.id, time)
  if (eventTimes.size > 1024) {
    const oldest = eventTimes.keys().next().value!
    eventTimes.delete(oldest)
    eventTokens.delete(oldest)
  }
  const revisions = event.properties.resource_revisions
  const token =
    revisions && typeof revisions === 'object' && !Array.isArray(revisions) ? revisions.activities : undefined
  if (typeof token === 'string') eventTokens.set(activity.id, token)
  applyRow({
    activity: activity as Activity,
    dismissed: payload.reason === 'dismissed',
    token: typeof token === 'string' ? token : undefined,
  })
  if (needsBaseline && !readController) queue.invalidate(0)
  return true
}

const searchQuery = ref('')
const statusFilter = ref('all')
const activeCount = computed(() => activities.value.filter((activity) => activityIsActive(activity.status)).length)
const finishedCount = computed(() => activities.value.length - activeCount.value)
const filters = computed(() => [
  { id: 'all', label: st('All'), count: activities.value.length },
  { id: 'active', label: st('Active'), count: activeCount.value },
  { id: 'finished', label: st('Finished'), count: finishedCount.value },
])
const sortedActivities = computed(() => {
  const query = searchQuery.value.trim().toLowerCase()
  return activities.value.filter(
    (activity) =>
      (statusFilter.value === 'all' || activityIsActive(activity.status) === (statusFilter.value === 'active')) &&
      (!query ||
        [activity.title, activity.description, activity.id, activity.kind, activity.session_id]
          .join(' ')
          .toLowerCase()
          .includes(query)),
  )
})
const statusLabels = computed<Record<string, string>>(() => ({
  pending: st('Pending'),
  running: st('Running'),
  waiting: st('Waiting'),
  paused: st('Paused'),
  succeeded: st('Succeeded'),
  failed: st('Failed'),
  cancelled: st('Cancelled'),
  stopped: st('Stopped'),
}))

function activityId(activity: Activity): string {
  return String(activity.id || '')
}

function activityControls(activity: Activity): ActivityControl[] {
  return Array.isArray(activity.controls) ? activity.controls : []
}

function formatCreatedAt(value: number): string {
  return Number.isFinite(value) && value > 0 ? new Date(value).toLocaleString() : ''
}

function controlLabel(control: ActivityControl): string {
  return { stop: st('Stop'), pause: st('Pause'), resume: st('Resume'), delete: st('Delete'), dismiss: st('Dismiss') }[
    control
  ]
}

const beginRead = createLatestRequestGuard(() => captureResourceObservation('activities').scope, activities)
async function readActivities() {
  if (!visible.value) return
  ensureActivityScope()
  const controller = new AbortController()
  readPatches.clear()
  readController = controller
  const isCurrent = beginRead()
  loading.value = true
  error.value = ''
  try {
    const { value: data, observation } = await conditionalJsonObserved<Activity[]>('activities', '/api/v1/activities', {
      signal: controller.signal,
    })
    if (controller.signal.aborted || !isCurrent()) return
    const current = captureResourceObservation('activities')
    const responseEpoch = observation.token?.slice(0, observation.token.lastIndexOf(':'))
    if (responseEpoch && current.epoch && responseEpoch !== current.epoch) {
      queue.invalidate(0)
      return
    }
    let rows = [...(Array.isArray(data) ? data : [])].sort(compareActivities)
    for (const patch of readPatches.values()) {
      if (!tokenCovers(observation.token, patch.token)) rows = patchRows(rows, patch)
    }
    activities.value = rows
    needsBaseline = false
  } catch (err) {
    if (controller.signal.aborted || !isCurrent()) return
    error.value = err instanceof Error ? err.message : String(err)
    throw err
  } finally {
    if (readController === controller) {
      readController = undefined
      readPatches.clear()
    }
    if (isCurrent()) loading.value = false
  }
}

async function runAction(id: string, action: ActivityControl) {
  if (!id || busyId.value) return
  ensureActivityScope()
  busyId.value = id
  const scope = captureResourceObservation('activities').scope
  try {
    await controlActivity(id, action)
    if (scope !== captureResourceObservation('activities').scope) return
    toasts.push('success', st('{action} requested', { action: controlLabel(action) }))
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    toasts.push('error', msg)
  } finally {
    busyId.value = null
  }
}

async function clearFinished() {
  if (busyId.value) return
  busyId.value = '__clear_finished__'
  try {
    await clearFinishedActivities(activities.value)
    toasts.push('success', st('Finished activities cleared'))
    await refresh()
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    toasts.push('error', msg)
  } finally {
    busyId.value = null
  }
}

onMounted(() => {
  void queue.refresh().catch(() => {})
})
const queue = createRevalidator(readActivities, { intervalMs: 250, retryMs: 5000, enabled: () => visible.value })
const refresh = () => queue.refresh().catch(() => {})
const releaseMutations = subscribeActivityMutations(({ activity, dismissed, observation }) => {
  ensureActivityScope()
  if (observation.scope !== activityScope || (observation.epoch && observation.epoch !== activityEpoch)) return
  const streamed = eventTokens.get(activity.id)
  // A descriptor streamed after this command began is at least as fresh as
  // its response, and may already include a subsequent delivery or edit.
  if (streamed && !tokenCovers(observation.token, streamed)) return
  applyRow({ activity: activity as Activity, dismissed })
})
const release = subscribeResource(
  'activities',
  (event) => {
    if (!applyActivityEvent(event)) {
      needsBaseline = true
      queue.invalidate(150)
    }
  },
  {
    onInvalidate: (reason) => {
      // All panels apply the shared mutation response. Reconnects, missed events,
      // visibility and backend changes still require an authoritative baseline.
      if (reason === 'mutation') return
      needsBaseline = true
      queue.invalidate(0)
    },
  },
)
watch(
  visible,
  (shown) => {
    if (shown) {
      ensureActivityScope()
      // Hidden panes still apply the shared stream. Validate its revision;
      // only a known gap, changed version or failed read needs a full body.
      if (needsBaseline || error.value) queue.invalidate(0)
      void checkResourceVersions(['activities']).catch(() => {})
      queue.resume()
    } else {
      queue.pause()
      if (readController) {
        readController.abort()
        beginRead()
        loading.value = false
        queue.invalidate(0)
      }
    }
  },
  { flush: 'sync' },
)
onScopeDispose(() => {
  release()
  releaseMutations()
  queue.dispose()
  readController?.abort()
  beginRead()
})
function clearFilters() {
  searchQuery.value = ''
  statusFilter.value = 'all'
}
defineExpose({ refresh })
</script>

<template>
  <SettingsCard :title="$st('Background activities')" :icon="RiPulseLine" :count="activities.length">
    <template #actions>
      <div class="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" :disabled="busyId !== null || finishedCount === 0" @click="clearFinished">{{
          busyId === '__clear_finished__' ? $st('Clearing...') : $st('Clear finished')
        }}</Button>
        <IconButton
          variant="outline"
          size="sm"
          :tooltip="$st('Refresh')"
          :aria-label="$st('Refresh')"
          :disabled="loading"
          @click="refresh"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''"
        /></IconButton>
      </div>
    </template>
    <div class="mb-4 flex min-w-0 flex-col gap-3 xl:flex-row xl:items-center">
      <SearchInput
        v-model="searchQuery"
        class="min-w-0 flex-1"
        :placeholder="$st('Search activities')"
        :input-aria-label="$st('Search activities')"
        :show-search-button="false"
      />
      <div
        class="grid grid-cols-3 gap-1 rounded-lg border border-border/50 bg-muted/20 p-1"
        :aria-label="$st('Status')"
      >
        <button
          v-for="filter in filters"
          :key="filter.id"
          type="button"
          class="flex min-w-0 flex-wrap items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :class="
            statusFilter === filter.id
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground'
          "
          :aria-pressed="statusFilter === filter.id"
          @click="statusFilter = filter.id"
        >
          {{ filter.label }}<span class="text-[11px] tabular-nums text-muted-foreground">{{ filter.count }}</span>
        </button>
      </div>
    </div>
    <p
      v-if="error"
      role="alert"
      class="mb-3 break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
    >
      {{ error }}
    </p>
    <p v-if="loading && !activities.length" class="py-6 text-center text-sm text-muted-foreground">
      {{ $st('Loading activities...') }}
    </p>
    <div
      v-else-if="!sortedActivities.length"
      class="rounded-lg border border-dashed border-border/60 bg-muted/10 px-4 py-8 text-center"
    >
      <RiPulseLine class="mx-auto mb-2 h-6 w-6 text-muted-foreground/60" />
      <p class="text-sm text-muted-foreground">
        {{ activities.length ? $st('No matching activities.') : $st('No background activities.') }}
      </p>
      <Button v-if="activities.length" variant="ghost" size="sm" class="mt-2" @click="clearFilters">{{
        $st('Clear filters')
      }}</Button>
    </div>
    <div v-else class="space-y-3">
      <article
        v-for="activity in sortedActivities"
        :key="activityId(activity)"
        class="min-w-0 rounded-xl border border-border/60 bg-background/30 p-3 sm:p-4"
      >
        <div class="flex min-w-0 flex-wrap items-start gap-3">
          <div class="min-w-0 flex-1">
            <h3 class="break-words text-sm font-semibold">{{ activity.title || activityId(activity) }}</h3>
            <p v-if="activity.description" class="mt-1 break-words text-xs leading-relaxed text-muted-foreground">
              {{ activity.description }}
            </p>
          </div>
          <span
            class="inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px]"
            :class="
              activity.status === 'failed'
                ? 'border-destructive/20 bg-destructive/10 text-destructive'
                : activityIsActive(activity.status)
                  ? 'border-primary/20 bg-primary/10 text-foreground'
                  : 'border-border/50 bg-muted/40 text-muted-foreground'
            "
          >
            <span class="h-1.5 w-1.5 rounded-full bg-current" />{{ statusLabels[activity.status] || activity.status }}
          </span>
        </div>
        <div class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
          <span class="break-all">{{ activity.kind }}</span
          ><span v-if="activity.session_id">{{ $st('Session') }} #{{ activity.session_id }}</span
          ><span>{{ formatCreatedAt(activity.created_at_ms) }}</span>
        </div>
        <p
          v-if="activity.message"
          class="mt-3 break-words rounded-lg bg-muted/30 p-3 text-xs leading-relaxed text-muted-foreground"
        >
          {{ activity.message }}
        </p>
        <div class="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-border/50 pt-3">
          <details class="min-w-0 text-[11px] text-muted-foreground">
            <summary class="cursor-pointer">{{ $st('Details') }}</summary>
            <code class="mt-2 block break-all">{{ activityId(activity) }}</code>
          </details>
          <div class="flex flex-wrap items-center gap-2">
            <Button
              v-for="control in activityControls(activity).filter((item) => item !== 'delete')"
              :key="control"
              variant="outline"
              size="sm"
              :disabled="busyId !== null"
              @click="runAction(activityId(activity), control)"
              >{{ busyId === activityId(activity) ? $st('Working...') : controlLabel(control) }}</Button
            >
            <ConfirmPopover
              v-if="activityControls(activity).includes('delete')"
              :title="$st('Delete activity?')"
              :description="activityId(activity)"
              :confirm-text="$st('Delete')"
              :cancel-text="$st('Cancel')"
              variant="destructive"
              @confirm="runAction(activityId(activity), 'delete')"
            >
              <Button
                variant="ghost"
                size="sm"
                class="text-destructive hover:bg-destructive/10"
                :disabled="busyId !== null"
                >{{ $st('Delete') }}</Button
              >
            </ConfirmPopover>
          </div>
        </div>
      </article>
    </div>
  </SettingsCard>
</template>
