<script setup lang="ts">
import { onMounted, onScopeDispose, ref, watch } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'

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

const sortedActivities = activities

function fieldNames(activity: Activity): Array<'kind' | 'session_id' | 'status'> {
  const keys: Array<'kind' | 'session_id' | 'status'> = ['kind', 'session_id', 'status']
  return keys.filter((key) => {
    const value = activity[key]
    return value !== undefined && value !== null && String(value).trim().length > 0
  })
}

function displayValue(value: unknown): string {
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') return String(value)
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

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
  return control.charAt(0).toUpperCase() + control.slice(1)
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
</script>

<template>
  <div class="space-y-6">
    <div class="text-sm text-muted-foreground">
      {{ $st('Background activities running on the Agena server.') }}
    </div>

    <div class="grid gap-3">
      <div v-if="loading && activities.length === 0" class="text-sm text-muted-foreground">
        {{ $st('Loading activities...') }}
      </div>
      <div
        v-if="error"
        class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {{ error }}
      </div>
      <div v-if="!loading && sortedActivities.length === 0" class="text-sm text-muted-foreground">
        {{ $st('No background activities.') }}
      </div>

      <div v-if="sortedActivities.length > 0" class="space-y-2">
        <div
          v-for="activity in sortedActivities"
          :key="activityId(activity)"
          class="rounded-md border border-border/60 bg-background/50 px-3 py-2.5"
        >
          <div class="flex items-center justify-between gap-3">
            <div class="min-w-0">
              <div class="text-sm font-semibold break-words">{{ activity.title || activityId(activity) }}</div>
              <div class="mt-0.5 font-mono text-[11px] text-muted-foreground break-all">{{ activityId(activity) }}</div>
              <div v-if="activity.description" class="mt-1 text-xs text-muted-foreground break-words">
                {{ activity.description }}
              </div>
              <div
                v-if="fieldNames(activity).length"
                class="mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground"
              >
                <span v-for="key in fieldNames(activity)" :key="key" class="break-all">
                  {{ key }}: {{ displayValue(activity[key]) }}
                </span>
                <span v-if="formatCreatedAt(activity.created_at_ms)">{{
                  formatCreatedAt(activity.created_at_ms)
                }}</span>
              </div>
              <div v-if="activity.message" class="mt-1 text-xs text-muted-foreground break-words">
                {{ activity.message }}
              </div>
            </div>

            <div class="flex shrink-0 items-center gap-1.5">
              <Button
                v-for="control in activityControls(activity).filter((item) => item !== 'delete')"
                :key="control"
                variant="outline"
                size="sm"
                :disabled="busyId === activityId(activity)"
                @click="runAction(activityId(activity), control)"
              >
                {{ busyId === activityId(activity) ? 'Working...' : controlLabel(control) }}
              </Button>
              <ConfirmPopover
                v-if="activityControls(activity).includes('delete')"
                :title="$st('Delete activity?')"
                :description="activityId(activity)"
                :confirm-text="'Delete'"
                :cancel-text="'Cancel'"
                variant="destructive"
                @confirm="runAction(activityId(activity), 'delete')"
              >
                <Button
                  variant="outline"
                  size="sm"
                  class="shrink-0 text-destructive border-destructive/30 hover:bg-destructive/10"
                >
                  {{ $st('Delete') }}
                </Button>
              </ConfirmPopover>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="flex items-center gap-2 flex-wrap">
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? 'Refreshing...' : $st('Refresh')"
        :aria-label="loading ? 'Refreshing...' : $st('Refresh')"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
      <Button variant="outline" size="sm" :disabled="loading" @click="refresh">
        {{ loading ? 'Refreshing...' : $st('Refresh') }}
      </Button>
      <Button
        variant="outline"
        size="sm"
        :disabled="busyId !== null || sortedActivities.length === 0"
        @click="clearFinished"
      >
        {{ busyId === '__clear_finished__' ? $st('Clearing...') : $st('Clear finished') }}
      </Button>
    </div>
  </div>
</template>
