import { userErrorMessage } from '@/lib/api'
<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'

import type {
  ModelCatalogEntry,
  ModelCatalogSummary,
  ProviderModel,
  ProviderModelThinkingMode,
  ProviderModelPricing,
  ProviderModelSpeedMode,
  ProviderSummary,
  RuntimeBackgroundTask,
  RuntimeStatus,
} from '@/agena/lib/agenaApi'
import { cancelRuntimeBackgroundTask, listModelCatalogEntries } from '@/agena/lib/agenaApi'

import { useRuntimeModelCatalogActions } from './useRuntimeModelCatalogActions'
import { useNotifications } from '@/agena/lib/notifications/useNotifications'

const props = defineProps<{
  catalogEntries: ModelCatalogEntry[]
  operatorCards: Array<{ label: string; value: string | number }>
  runtimeSnapshotFacts: Array<{ label: string; value: string; mono?: boolean }>
  runtime: RuntimeStatus | null
  providers: ProviderSummary[]
  providerModels: Record<string, ProviderModel[]>
  sessionCacheFacts: Array<{ label: string; value: string; mono?: boolean }>
  formatProviderModel: (model: ProviderModel) => string
  load: () => Promise<void>
}>()

const { notify } = useNotifications()

function summarizeCatalogEntries(entries: ModelCatalogEntry[]): ModelCatalogSummary {
  return {
    refreshing: false,
    entry_count: entries.length,
  }
}

const actionError = ref('')
const actionMessage = ref('')
const catalogEntriesState = ref<ModelCatalogEntry[]>(props.catalogEntries.map((entry) => ({ ...entry })))
const catalogSummaryState = ref<ModelCatalogSummary | null>(
  props.catalogEntries.length ? summarizeCatalogEntries(props.catalogEntries) : null,
)
const catalogOriginFilter = ref('all')
const catalogQuery = ref('')
const catalogOriginOptions = ref<string[]>([])
const catalogTotal = ref(props.catalogEntries.length)
const catalogOffset = ref(0)
const catalogLimit = ref(50)
const submitting = ref(false)
const cancellingTaskIds = reactive<Record<string, boolean>>({})
const taskStatuses = new Map<string, RuntimeBackgroundTask['status']>()

const { refreshCatalogAction } = useRuntimeModelCatalogActions({
  actionError,
  actionMessage,
  load: props.load,
})

const sortedCatalogEntries = computed(() => catalogEntriesState.value)

const hasCatalogFilters = computed(() => Boolean(catalogQuery.value.trim()) || catalogOriginFilter.value !== 'all')

const filteredCatalogEntries = computed(() => sortedCatalogEntries.value)
const backgroundTasks = computed(() => props.runtime?.background_tasks ?? [])
const catalogRefreshing = computed(() =>
  backgroundTasks.value.some((task) => task.kind === 'model_catalog_refresh' && task.status === 'running'),
)
const catalogRefreshButtonLabel = computed(() => (catalogRefreshing.value ? 'Refreshing…' : 'Refresh Catalog'))

function clearCatalogFilters() {
  catalogQuery.value = ''
  catalogOriginFilter.value = 'all'
  void loadCatalogPage(0)
}

type ProviderModelThinkingModeWithDisabled = ProviderModelThinkingMode & {
  disabled?: boolean
}

type ProviderModelSpeedModeWithDisabled = ProviderModelSpeedMode & {
  disabled?: boolean
}

function entryThinkingModeItems(entry: ModelCatalogEntry): Array<[string, ProviderModelThinkingModeWithDisabled]> {
  const defaultName = typeof entry.thinking_modes?.default === 'string' ? entry.thinking_modes.default : ''
  return Object.entries(entry.thinking_modes || {}).flatMap(([name, mode]) =>
    name !== 'default' && mode && typeof mode === 'object'
      ? [[name, { ...mode, default: name === defaultName } as ProviderModelThinkingModeWithDisabled]]
      : [],
  )
}

function entrySpeedModeItems(entry: ModelCatalogEntry): Array<[string, ProviderModelSpeedModeWithDisabled]> {
  const defaultName = typeof entry.speed_modes?.default === 'string' ? entry.speed_modes.default : ''
  return Object.entries(entry.speed_modes || {})
    .filter(([name, mode]) => name !== 'default' && mode && typeof mode === 'object')
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, mode]) => [name, { ...mode, default: name === defaultName } as ProviderModelSpeedModeWithDisabled])
}

function formatModeJson(value: Record<string, unknown> | null | undefined) {
  if (!value) return ''
  const text = JSON.stringify(value)
  return text.length > 96 ? `${text.slice(0, 93)}...` : text
}

function formatOutputModalities(value: string[] | null | undefined) {
  return Array.isArray(value) && value.length ? value.join(', ') : ''
}

function formatPricingSummary(pricing: ProviderModelPricing | null | undefined) {
  if (!pricing) return ''
  const parts: string[] = []
  if (pricing.input_usd_per_million_tokens) parts.push(`in ${pricing.input_usd_per_million_tokens}`)
  if (pricing.output_usd_per_million_tokens) parts.push(`out ${pricing.output_usd_per_million_tokens}`)
  if (pricing.cache_read_usd_per_million_tokens) parts.push(`cache read ${pricing.cache_read_usd_per_million_tokens}`)
  if (pricing.cache_write_usd_per_million_tokens)
    parts.push(`cache write ${pricing.cache_write_usd_per_million_tokens}`)
  if (Array.isArray(pricing.tiers) && pricing.tiers.length) parts.push(`${pricing.tiers.length} tier`)
  return parts.join(' · ')
}

function formatSamplingSummary(
  temperature: string | number | null | undefined,
  topP: string | number | null | undefined,
  topK: string | number | null | undefined,
) {
  const parts: string[] = []
  if (temperature != null && String(temperature).trim()) parts.push(`temp ${String(temperature).trim()}`)
  if (topP != null && String(topP).trim()) parts.push(`top_p ${String(topP).trim()}`)
  if (topK != null && String(topK).trim()) parts.push(`top_k ${String(topK).trim()}`)
  return parts.join(' · ')
}

function taskStatusLabel(status: RuntimeBackgroundTask['status']) {
  return status.charAt(0).toUpperCase() + status.slice(1)
}

function taskStatusClass(status: RuntimeBackgroundTask['status']) {
  return `task-status-${status}`
}

function taskOriginLabel(origin: RuntimeBackgroundTask['origin']) {
  return origin === 'user' ? 'User' : 'System'
}

function taskFailureMessage(task: RuntimeBackgroundTask) {
  const failure = task.failure
  if (!failure) return ''
  const summary = failure.user?.fallback?.trim() || `${task.title} failed.`
  return failure.category === 'internal' || failure.category === 'data_corruption'
    ? `${summary} Reference: ${failure.id}`
    : summary
}

function taskMessage(task: RuntimeBackgroundTask) {
  return taskFailureMessage(task) || task.message || ''
}

function taskCanCancel(task: RuntimeBackgroundTask) {
  return task.cancellable && task.status === 'running' && !cancellingTaskIds[task.id]
}

async function loadCatalogPage(offset = 0) {
  const response = await listModelCatalogEntries({
    q: catalogQuery.value,
    origin: catalogOriginFilter.value,
    offset,
    limit: catalogLimit.value,
  })
  catalogEntriesState.value = response.items ?? []
  catalogSummaryState.value = response.summary
  catalogOriginOptions.value = response.available_origins ?? []
  catalogTotal.value = response.total ?? 0
  catalogOffset.value = response.offset ?? offset
  catalogLimit.value = response.limit ?? catalogLimit.value
}

function previousCatalogPage() {
  if (catalogOffset.value <= 0) return
  void loadCatalogPage(Math.max(0, catalogOffset.value - catalogLimit.value))
}

function nextCatalogPage() {
  if (catalogOffset.value + catalogEntriesState.value.length >= catalogTotal.value) return
  void loadCatalogPage(catalogOffset.value + catalogLimit.value)
}

async function runCatalogSearch() {
  await loadCatalogPage(0)
}

async function refreshCatalog() {
  submitting.value = true
  try {
    const result = await refreshCatalogAction()
    const message = result.started
      ? `Started ${result.task.title.toLowerCase()}.`
      : `${result.task.title} is already running.`
    actionError.value = ''
    actionMessage.value = message
    notify.toast('info', message)
  } catch (err) {
    const message = userErrorMessage(err)
    notify.toast('error', message, 7000)
  } finally {
    submitting.value = false
  }
}

async function cancelTask(task: RuntimeBackgroundTask) {
  if (!taskCanCancel(task)) return
  cancellingTaskIds[task.id] = true
  try {
    const updated = await cancelRuntimeBackgroundTask(task.id)
    const message = updated.message || `Cancellation requested for ${updated.title.toLowerCase()}.`
    actionError.value = ''
    actionMessage.value = message
    notify.toast('info', message)
    await props.load()
  } catch (err) {
    const message = userErrorMessage(err)
    actionMessage.value = ''
    actionError.value = message
    notify.toast('error', message, 7000)
  } finally {
    delete cancellingTaskIds[task.id]
  }
}

function handleTaskCompletion(task: RuntimeBackgroundTask) {
  if (task.kind === 'model_catalog_refresh' && task.status === 'succeeded') {
    void loadCatalogPage(catalogOffset.value)
  }

  if (task.origin !== 'user') {
    return
  }

  if (task.status === 'succeeded') {
    const message = task.message || `${task.title} completed.`
    actionError.value = ''
    actionMessage.value = message
    notify.toast('success', message)
    return
  }

  if (task.status === 'failed') {
    const message = taskFailureMessage(task) || `${task.title} failed.`
    actionMessage.value = ''
    actionError.value = message
    notify.toast('error', message, 7000)
    return
  }

  if (task.status === 'cancelled') {
    const message = task.message || `${task.title} was cancelled.`
    actionError.value = ''
    actionMessage.value = message
    notify.toast('info', message)
  }
}

function syncTaskStatuses(tasks: RuntimeBackgroundTask[]) {
  const currentIds = new Set(tasks.map((task) => task.id))

  for (const task of tasks) {
    const previousStatus = taskStatuses.get(task.id)
    if (!previousStatus) {
      taskStatuses.set(task.id, task.status)
      continue
    }
    if (previousStatus !== task.status) {
      taskStatuses.set(task.id, task.status)
      if (previousStatus === 'running') {
        handleTaskCompletion(task)
      }
    }
  }

  for (const taskId of Array.from(taskStatuses.keys())) {
    if (!currentIds.has(taskId)) {
      taskStatuses.delete(taskId)
    }
  }
}

watch(
  () => props.runtime?.background_tasks ?? [],
  (tasks) => {
    syncTaskStatuses(tasks)
  },
  { immediate: true },
)

onMounted(() => {
  void loadCatalogPage(0)
})
</script>

<template>
  <div>
    <div class="grid three">
      <section v-for="card in props.operatorCards" :key="card.label" class="card">
        <div class="muted">{{ card.label }}</div>
        <div style="font-size: 1.5rem; font-weight: 600">{{ card.value }}</div>
      </section>
    </div>

    <div class="grid two" style="margin-top: 16px">
      <section class="card">
        <h3>Runtime Snapshot</h3>
        <div v-if="props.runtimeSnapshotFacts.length" class="stack">
          <div v-for="fact in props.runtimeSnapshotFacts" :key="fact.label">
            <strong>{{ fact.label }}:</strong>
            <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
          </div>
        </div>
        <p v-else class="muted">Loading runtime snapshot…</p>
      </section>

      <section class="card">
        <h3>Runtime Tasks</h3>
        <div v-if="props.runtime" class="stack">
          <div>
            <strong>Reload:</strong> {{ props.runtime.reload.enabled ? 'enabled' : 'disabled' }} ({{
              props.runtime.reload.interval_secs
            }}s)
          </div>
          <div>
            <strong>Session GC:</strong> {{ props.runtime.session_gc.enabled ? 'enabled' : 'disabled' }} ({{
              props.runtime.session_gc.interval_secs
            }}s)
          </div>
          <div><strong>Watch Paths:</strong></div>
          <div v-if="props.runtime.watch_paths.length" class="list">
            <div v-for="path in props.runtime.watch_paths" :key="path" class="list-item mono">{{ path }}</div>
          </div>
          <div v-else class="muted">No watch paths configured.</div>
        </div>
        <p v-else class="muted">Loading runtime tasks…</p>
      </section>
    </div>

    <div class="grid two" style="margin-top: 16px">
      <section class="card">
        <h3>Recent Automation</h3>
        <div v-if="props.runtime?.automation.recent_jobs.length" class="list">
          <div v-for="job in props.runtime.automation.recent_jobs" :key="job.id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div>
                  <strong>{{ job.kind }}</strong> <span class="muted mono">{{ job.id }}</span>
                </div>
                <div class="muted">session {{ job.owner_session_id ?? 'n/a' }}</div>
                <div v-if="job.last_run" class="muted">
                  {{ job.last_run.status }} · triggered {{ job.last_run.triggered_at }}
                </div>
                <div v-else-if="job.next_fire_at" class="muted">next {{ job.next_fire_at }}</div>
                <div v-if="job.last_run?.failure" class="muted">{{ job.last_run.failure.user.fallback }}</div>
              </div>
              <span class="badge">{{ job.expression || job.at || 'scheduled' }}</span>
            </div>
          </div>
        </div>
        <p v-else class="muted">No scheduled jobs visible yet.</p>
      </section>

      <section class="card">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Background Tasks</h3>
            <p class="muted">In-process runtime tasks are tracked here and can be cancelled while active.</p>
          </div>
          <span class="badge">{{ backgroundTasks.length }}</span>
        </div>
        <div v-if="backgroundTasks.length" class="list" style="margin-top: 12px">
          <div v-for="task in backgroundTasks" :key="task.id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div>
                  <strong>{{ task.title }}</strong> <span class="muted mono">{{ task.id }}</span>
                </div>
                <div class="muted">{{ taskOriginLabel(task.origin) }} · started {{ task.started_at }}</div>
                <div v-if="task.finished_at" class="muted">finished {{ task.finished_at }}</div>
                <div v-if="taskMessage(task)" class="muted">{{ taskMessage(task) }}</div>
              </div>
              <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
                <span class="badge" :class="taskStatusClass(task.status)">{{ taskStatusLabel(task.status) }}</span>
                <button
                  v-if="task.cancellable && task.status === 'running'"
                  class="button"
                  :disabled="!taskCanCancel(task)"
                  @click="cancelTask(task)"
                >
                  {{ cancellingTaskIds[task.id] ? 'Cancelling…' : 'Cancel' }}
                </button>
              </div>
            </div>
          </div>
        </div>
        <p v-else class="muted" style="margin-top: 12px">No runtime background tasks yet.</p>
      </section>

      <section class="card">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Provider Defaults</h3>
            <p class="muted">Provider defaults keep adapter and model as separate runtime fields.</p>
          </div>
        </div>
        <div v-if="props.providers.length" class="list">
          <div v-for="provider in props.providers" :key="provider.provider_id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div>
                  <strong>{{ provider.provider_id }}</strong>
                </div>
                <div class="muted">Default adapter: {{ provider.defaults.adapter || 'auto' }}</div>
                <div class="muted">Default model: {{ provider.defaults.model || 'unset' }}</div>
              </div>
            </div>
          </div>
        </div>
        <p v-else class="muted">No providers loaded.</p>
      </section>

      <section class="card">
        <h3>Session Cache</h3>
        <div v-if="props.sessionCacheFacts.length" class="stack">
          <div v-for="fact in props.sessionCacheFacts" :key="fact.label">
            <strong>{{ fact.label }}:</strong> {{ fact.value }}
          </div>
        </div>
        <p v-else class="muted">Session cache is not available.</p>
      </section>
    </div>

    <div class="grid two" style="margin-top: 16px">
      <section class="card">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Model Catalog</h3>
            <p class="muted">The catalog is read-only and exposes official entries only.</p>
          </div>
          <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
            <span v-if="catalogRefreshing" class="badge">Refreshing</span>
            <button class="button primary" :disabled="submitting || catalogRefreshing" @click="refreshCatalog">
              {{ catalogRefreshButtonLabel }}
            </button>
          </div>
        </div>

        <div v-if="props.runtime?.model_catalog" class="stack" style="margin-top: 12px">
          <div><strong>Last Source:</strong> {{ props.runtime.model_catalog.last_successful_source || 'none' }}</div>
          <div><strong>Last Refresh:</strong> {{ props.runtime.model_catalog.last_refresh_at || 'never' }}</div>
          <div v-if="props.runtime.model_catalog.last_failure" class="muted">
            {{ props.runtime.model_catalog.last_failure.user.fallback }}
          </div>
        </div>
        <p v-else class="muted" style="margin-top: 12px">Model catalog is not available in the runtime snapshot yet.</p>

        <p v-if="actionMessage" class="muted" style="margin-top: 12px">{{ actionMessage }}</p>
        <p v-if="actionError" class="muted" style="margin-top: 8px">{{ actionError }}</p>
        <p class="muted" style="margin-top: 12px">
          Refresh rebuilds the official catalog from the current provider registry and replaces the cached snapshot.
        </p>
      </section>

      <section class="card">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Catalog Entries</h3>
            <p class="muted">Browse the official catalog snapshot currently loaded into the runtime.</p>
          </div>
          <span class="badge">{{ catalogEntriesState.length }}/{{ catalogTotal }}</span>
        </div>

        <div class="settings-summary" style="margin-top: 12px">
          <div class="summary-item">
            <div class="summary-label">Models</div>
            <div class="summary-value">{{ catalogTotal }}</div>
          </div>
          <div class="summary-item">
            <div class="summary-label">Showing</div>
            <div class="summary-value">{{ catalogEntriesState.length }}</div>
          </div>
        </div>

        <div class="grid two" style="margin-top: 12px">
          <div class="field">
            <label class="label" for="catalog-entry-search">Find Entries</label>
            <input
              id="catalog-entry-search"
              v-model="catalogQuery"
              class="input mono"
              placeholder="model, origin, mode, description"
            />
          </div>
          <div class="field">
            <label class="label" for="catalog-entry-origin-filter">Origin</label>
            <select id="catalog-entry-origin-filter" v-model="catalogOriginFilter" class="select">
              <option value="all">All origins</option>
              <option v-for="origin in catalogOriginOptions" :key="origin" :value="origin">
                {{ origin }}
              </option>
            </select>
          </div>
        </div>

        <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
          <button class="button primary" :disabled="submitting" @click="runCatalogSearch">Search</button>
          <button class="button" :disabled="!hasCatalogFilters" @click="clearCatalogFilters">Clear Filters</button>
          <button class="button" :disabled="submitting || catalogOffset <= 0" @click="previousCatalogPage">
            Previous
          </button>
          <button
            class="button"
            :disabled="submitting || catalogOffset + catalogEntriesState.length >= catalogTotal"
            @click="nextCatalogPage"
          >
            Next
          </button>
        </div>

        <div v-if="filteredCatalogEntries.length" class="list" style="margin-top: 12px">
          <div v-for="entry in filteredCatalogEntries" :key="entry.model_id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div>
                  <strong>{{ entry.model_id }}</strong>
                </div>
                <div class="muted">
                  {{ entry.display_name || 'Unnamed model' }} · {{ entry.origin || 'Unknown origin' }} ·
                  {{ entry.source_label || entry.source }}
                </div>
                <div v-if="entry.lifecycle" class="muted">{{ entry.lifecycle }}</div>
                <div v-if="entry.description" class="muted">{{ entry.description }}</div>
                <div v-if="formatOutputModalities(entry.output_modalities)" class="muted">
                  Output: {{ formatOutputModalities(entry.output_modalities) }}
                </div>
                <div v-if="formatPricingSummary(entry.pricing)" class="muted">
                  Pricing: {{ formatPricingSummary(entry.pricing) }}
                </div>
                <div
                  v-if="formatSamplingSummary(entry.default_temperature, entry.default_top_p, entry.default_top_k)"
                  class="muted"
                >
                  Sampling:
                  {{ formatSamplingSummary(entry.default_temperature, entry.default_top_p, entry.default_top_k) }}
                </div>
                <div
                  v-if="entry.context_window_tokens || entry.max_input_tokens || entry.max_output_tokens"
                  class="muted mono"
                >
                  ctx={{ entry.context_window_tokens ?? 'n/a' }} · max_in={{ entry.max_input_tokens ?? 'n/a' }} ·
                  max_out={{ entry.max_output_tokens ?? 'n/a' }}
                </div>
                <div v-if="entryThinkingModeItems(entry).length" class="stack" style="margin-top: 8px">
                  <div class="muted">Thinking modes:</div>
                  <div
                    v-for="[modeName, mode] in entryThinkingModeItems(entry)"
                    :key="modeName"
                    class="list-item"
                    style="padding: 8px 10px"
                  >
                    <div>
                      <strong>{{ modeName }}</strong>
                      <span v-if="mode.display_name" class="muted"> · {{ mode.display_name }}</span>
                      <span v-if="mode.default" class="badge" style="margin-left: 8px">default</span>
                      <span v-if="mode.disabled" class="badge" style="margin-left: 8px">disabled</span>
                    </div>
                    <div v-if="mode.description" class="muted">{{ mode.description }}</div>
                    <div v-if="mode.thinking" class="muted mono">thinking {{ formatModeJson(mode.thinking) }}</div>
                  </div>
                </div>
                <div v-if="entrySpeedModeItems(entry).length" class="stack" style="margin-top: 8px">
                  <div class="muted">Speed modes:</div>
                  <div
                    v-for="[modeName, mode] in entrySpeedModeItems(entry)"
                    :key="modeName"
                    class="list-item"
                    style="padding: 8px 10px"
                  >
                    <div>
                      <strong>{{ modeName }}</strong>
                      <span v-if="mode.display_name" class="muted"> · {{ mode.display_name }}</span>
                      <span v-if="mode.default" class="badge" style="margin-left: 8px">default</span>
                      <span v-if="mode.disabled" class="badge" style="margin-left: 8px">disabled</span>
                    </div>
                    <div v-if="mode.description" class="muted">{{ mode.description }}</div>
                    <div v-if="mode.request_override" class="muted mono">
                      request {{ formatModeJson(mode.request_override) }}
                    </div>
                    <div v-if="mode.adapter_overrides" class="muted mono">
                      adapters {{ formatModeJson(mode.adapter_overrides) }}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
        <p v-else-if="catalogTotal" class="muted" style="margin-top: 12px">
          No catalog entries match the current filters.
        </p>
        <p v-else class="muted" style="margin-top: 12px">No catalog entries loaded.</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.task-status-running {
  background: rgba(14, 116, 144, 0.12);
}

.task-status-succeeded {
  background: rgba(22, 163, 74, 0.12);
}

.task-status-failed {
  background: rgba(220, 38, 38, 0.12);
}

.task-status-cancelled {
  background: rgba(100, 116, 139, 0.12);
}
</style>
