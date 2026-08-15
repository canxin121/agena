<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { userErrorMessage } from '@/lib/api'

import {
  clearFinishedActivities,
  deleteActivity,
  dismissActivity,
  fetchActivities,
  fetchActivityLogs,
  pauseActivity,
  resumeActivity,
  stopActivity,
  type BackgroundActivityLogResource,
  type BackgroundActivityResource,
} from '@/agena/lib/agenaApi'
import { useNotifications } from '@/agena/lib/notifications/useNotifications'

const { notify } = useNotifications()
const activities = ref<BackgroundActivityResource[]>([])
const logs = ref<BackgroundActivityLogResource | null>(null)
const selectedId = ref<string | null>(null)
const loading = ref(false)
const busyId = ref<string | null>(null)
const kindFilter = ref('')
const statusFilter = ref('')
const activeOnly = ref(false)

let refreshTimer: ReturnType<typeof setInterval> | undefined

const kindOptions = [
  { value: '', label: 'All kinds' },
  { value: 'shell', label: 'Shell' },
  { value: 'monitor', label: 'Monitor' },
  { value: 'task', label: 'Task' },
  { value: 'cron', label: 'Cron' },
  { value: 'runtime', label: 'Runtime' },
  { value: 'browser', label: 'Browser' },
]

const statusOptions = [
  { value: '', label: 'All statuses' },
  { value: 'running', label: 'Running' },
  { value: 'pending', label: 'Pending' },
  { value: 'paused', label: 'Paused' },
  { value: 'succeeded', label: 'Succeeded' },
  { value: 'failed', label: 'Failed' },
  { value: 'cancelled', label: 'Cancelled' },
  { value: 'stopped', label: 'Stopped' },
]

const filtered = computed(() =>
  activities.value
    .filter((activity) => {
      if (kindFilter.value && activity.kind !== kindFilter.value) return false
      if (statusFilter.value && activity.status !== statusFilter.value) return false
      if (activeOnly.value && !isActive(activity.status)) return false
      return true
    })
    .sort((left, right) => {
      const activeDiff = Number(isActive(right.status)) - Number(isActive(left.status))
      if (activeDiff !== 0) return activeDiff
      return right.started_at_ms - left.started_at_ms
    }),
)

const activeCount = computed(() => activities.value.filter((activity) => isActive(activity.status)).length)
const finishedCount = computed(() => activities.value.length - activeCount.value)

function isActive(status: string): boolean {
  return status === 'running' || status === 'pending' || status === 'paused'
}

function kindIcon(kind: string): string {
  switch (kind) {
    case 'shell':
      return '⚙'
    case 'task':
      return '◈'
    case 'monitor':
      return '≋'
    case 'cron':
      return '◷'
    case 'runtime':
      return '↻'
    case 'browser':
      return '◉'
    default:
      return '•'
  }
}

function statusClass(status: string): string {
  return `status-${status}`
}

function durationLabel(activity: BackgroundActivityResource): string {
  const start = activity.started_at_ms
  const end = activity.finished_at_ms ?? Date.now()
  const seconds = Math.max(0, Math.floor((end - start) / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  return `${minutes}m ${seconds % 60}s`
}

async function loadActivities() {
  loading.value = true
  notify.clearBanner()
  try {
    activities.value = await fetchActivities({
      kinds: kindFilter.value ? [kindFilter.value] : [],
      statuses: statusFilter.value ? [statusFilter.value] : [],
      activeOnly: activeOnly.value,
    })
    if (selectedId.value && !activities.value.some((activity) => activity.id === selectedId.value)) {
      selectedId.value = null
      logs.value = null
    }
  } catch (err) {
    notify.error(userErrorMessage(err))
  } finally {
    loading.value = false
  }
}

async function loadLogs() {
  if (!selectedId.value) return
  try {
    logs.value = await fetchActivityLogs(selectedId.value, { limit: 300 })
  } catch (err) {
    notify.error(userErrorMessage(err))
  }
}

function toggleDetail(activity: BackgroundActivityResource) {
  selectedId.value = selectedId.value === activity.id ? null : activity.id
  logs.value = null
  if (selectedId.value) void loadLogs()
}

async function handleControl(activity: BackgroundActivityResource, control: string) {
  busyId.value = activity.id
  notify.clearBanner()
  try {
    if (control === 'stop') await stopActivity(activity.id)
    else if (control === 'pause') await pauseActivity(activity.id)
    else if (control === 'resume') await resumeActivity(activity.id)
    else if (control === 'delete') await deleteActivity(activity.id)
    else if (control === 'dismiss') await dismissActivity(activity.id)
    await loadActivities()
  } catch (err) {
    notify.error(userErrorMessage(err))
  } finally {
    busyId.value = null
  }
}

async function handleClearFinished() {
  busyId.value = 'clear'
  notify.clearBanner()
  try {
    await clearFinishedActivities()
    selectedId.value = null
    logs.value = null
    await loadActivities()
  } catch (err) {
    notify.error(userErrorMessage(err))
  } finally {
    busyId.value = null
  }
}

onMounted(() => {
  void loadActivities()
  refreshTimer = setInterval(() => {
    void loadActivities()
    const selected = activities.value.find((activity) => activity.id === selectedId.value)
    if (selected && selected.kind !== 'cron' && ['pending', 'running'].includes(selected.status)) void loadLogs()
  }, 4000)
})

onBeforeUnmount(() => {
  if (refreshTimer) clearInterval(refreshTimer)
})
</script>

<template>
  <div class="page">
    <section class="page-header">
      <div>
        <h1>Background Activities</h1>
        <p class="muted">
          {{ activeCount }} managed · {{ finishedCount }} finished — durable shell, monitor, task, and cron state, plus
          runtime maintenance and browser sessions
        </p>
      </div>
      <div class="button-row">
        <button class="button" :disabled="loading || busyId === 'clear'" @click="loadActivities">Refresh</button>
        <button
          class="button danger"
          :disabled="finishedCount === 0 || busyId === 'clear'"
          @click="handleClearFinished"
        >
          Clear Finished
        </button>
      </div>
    </section>

    <section class="card">
      <div class="form-grid" style="align-items: end">
        <div class="field">
          <label class="label" for="activity-kind-filter">Kind</label>
          <select id="activity-kind-filter" v-model="kindFilter" class="select" @change="loadActivities">
            <option v-for="option in kindOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="activity-status-filter">Status</label>
          <select id="activity-status-filter" v-model="statusFilter" class="select" @change="loadActivities">
            <option v-for="option in statusOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>
        <label class="field checkbox-field">
          <input v-model="activeOnly" type="checkbox" @change="loadActivities" />
          <span>Active only</span>
        </label>
      </div>
    </section>

    <section class="card">
      <div v-if="filtered.length" class="list">
        <div
          v-for="activity in filtered"
          :key="activity.id"
          class="activity-row"
          :class="{ expanded: selectedId === activity.id }"
        >
          <button class="activity-summary" @click="toggleDetail(activity)">
            <span class="activity-kind">{{ kindIcon(activity.kind) }}</span>
            <span class="activity-main">
              <strong>{{ activity.title }}</strong>
              <span v-if="activity.description && activity.description !== activity.title" class="muted">
                — {{ activity.description }}
              </span>
              <span v-if="activity.command" class="muted mono"> · {{ activity.command }}</span>
            </span>
            <span class="badge" :class="statusClass(activity.status)">{{ activity.status }}</span>
            <span class="muted mono duration">{{ durationLabel(activity) }}</span>
            <span class="activity-actions">
              <button
                v-for="control in activity.controls"
                :key="control"
                class="button small"
                :class="{ danger: control === 'delete' || control === 'dismiss' }"
                :disabled="busyId === activity.id"
                @click.stop="handleControl(activity, control)"
              >
                {{ control }}
              </button>
            </span>
          </button>
          <div v-if="selectedId === activity.id" class="activity-detail">
            <div v-if="activity.message" class="muted" style="margin-bottom: 8px">{{ activity.message }}</div>
            <div v-if="activity.exit_code != null" class="muted mono" style="margin-bottom: 8px">
              exit code {{ activity.exit_code }}
            </div>
            <div v-if="activity.source_part_id != null" class="muted mono" style="margin-bottom: 8px">
              source part #{{ activity.source_part_id }}
            </div>
            <div v-if="activity.next_event_at_ms != null" class="muted mono" style="margin-bottom: 8px">
              next {{ new Date(activity.next_event_at_ms).toLocaleString() }}
            </div>
            <pre v-if="logs?.lines.length" class="log-tail mono">{{
              logs.lines.map((line) => line.text).join('\n')
            }}</pre>
            <p v-else class="muted">No output available for this activity.</p>
          </div>
        </div>
      </div>
      <p v-else class="muted">No background activities match the current filters.</p>
    </section>
  </div>
</template>

<style scoped>
.activity-row {
  border-bottom: 1px solid var(--border-color, rgba(128, 128, 128, 0.15));
}

.activity-row:last-child {
  border-bottom: none;
}

.activity-summary {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  background: transparent;
  border: none;
  cursor: pointer;
  text-align: left;
  color: inherit;
}

.activity-summary:hover {
  background: var(--hover-bg, rgba(128, 128, 128, 0.06));
}

.activity-kind {
  width: 20px;
  text-align: center;
  font-size: 14px;
}

.activity-main {
  flex: 1;
  min-width: 0;
}

.duration {
  min-width: 72px;
  text-align: right;
}

.activity-actions {
  display: flex;
  gap: 6px;
}

.activity-detail {
  padding: 10px 12px 12px 42px;
  border-top: 1px solid var(--border-color, rgba(128, 128, 128, 0.15));
}

.log-tail {
  max-height: 360px;
  overflow: auto;
  padding: 10px;
  border-radius: 6px;
  background: rgba(0, 0, 0, 0.25);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.badge.status-running {
  background: rgba(0, 122, 255, 0.15);
  color: #5aa8ff;
}

.badge.status-pending {
  background: rgba(255, 170, 0, 0.15);
  color: #d9a13b;
}

.badge.status-paused {
  background: rgba(160, 120, 255, 0.15);
  color: #b59bff;
}

.badge.status-succeeded {
  background: rgba(0, 180, 90, 0.15);
  color: #43c17a;
}

.badge.status-failed {
  background: rgba(220, 50, 50, 0.15);
  color: #ff6b6b;
}

.badge.status-cancelled,
.badge.status-stopped {
  background: rgba(160, 160, 160, 0.15);
  color: #a5a5a5;
}
</style>
