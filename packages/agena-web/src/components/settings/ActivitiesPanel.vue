<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '../../lib/api'
import { useToastsStore } from '../../stores/toasts'

type ActivityControl = 'stop' | 'pause' | 'resume' | 'delete' | 'dismiss'

type Activity = {
  id: string
  kind: string
  status: string
  title: string
  description: string
  session_id?: number | null
  created_at_ms: number
  message?: string | null
  controls?: ActivityControl[]
}

const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const activities = ref<Activity[]>([])
const busyId = ref<string | null>(null)

const sortedActivities = computed(() =>
  [...activities.value].sort((a, b) => Number(b.created_at_ms || 0) - Number(a.created_at_ms || 0)),
)

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

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const data = await apiJson<Activity[]>('/api/v1/activities')
    activities.value = Array.isArray(data) ? data : []
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    activities.value = []
  } finally {
    loading.value = false
  }
}

async function runAction(id: string, action: ActivityControl) {
  if (!id || busyId.value) return
  busyId.value = id
  try {
    await apiJson(`/api/v1/activities/${encodeURIComponent(id)}/${action}`, { method: 'POST' })
    toasts.push('success', `${controlLabel(action)} requested`)
    await refresh()
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
    await apiJson('/api/v1/activities/clear-finished', { method: 'POST' })
    toasts.push('success', 'Finished activities cleared')
    await refresh()
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    toasts.push('error', msg)
  } finally {
    busyId.value = null
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="space-y-6">
    <div>
      <div class="text-lg font-medium">Activities</div>
      <div class="mt-1 text-sm text-muted-foreground">Background activities running on the Agena server.</div>
    </div>

    <div class="grid gap-3">
      <div v-if="loading" class="text-sm text-muted-foreground">Loading activities...</div>
      <div
        v-else-if="error"
        class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {{ error }}
      </div>
      <div v-else-if="sortedActivities.length === 0" class="text-sm text-muted-foreground">
        No background activities.
      </div>

      <div v-else class="space-y-2">
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
                :title="'Delete activity?'"
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
                  Delete
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
        :tooltip="loading ? 'Refreshing...' : 'Refresh'"
        :aria-label="loading ? 'Refreshing...' : 'Refresh'"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
      <Button variant="outline" size="sm" :disabled="loading" @click="refresh">
        {{ loading ? 'Refreshing...' : 'Refresh' }}
      </Button>
      <Button
        variant="outline"
        size="sm"
        :disabled="busyId !== null || sortedActivities.length === 0"
        @click="clearFinished"
      >
        {{ busyId === '__clear_finished__' ? 'Clearing...' : 'Clear finished' }}
      </Button>
    </div>
  </div>
</template>
