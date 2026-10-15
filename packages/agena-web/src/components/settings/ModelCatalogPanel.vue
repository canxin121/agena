<script setup lang="ts">
import { computed, onMounted, onScopeDispose, ref, watch } from 'vue'
import { RiArrowLeftSLine, RiArrowRightSLine, RiRefreshLine, RiRestartLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { apiJson } from '@/lib/api'
import { createRevalidator } from '@/lib/revalidation'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'
import { orderedThinkingModeMap } from '@/lib/thinkingModeOrdering'

type UserProblem = {
  message?: string
  rendered?: string
  user?: { fallback?: string }
}

type ModelCatalogSummary = {
  refreshing?: boolean
  last_refresh_at?: string | null
  last_successful_source?: string | null
  last_failure?: UserProblem | null
  model_count?: number
}

type CatalogModel = {
  model_id: string
  source: string
  source_label?: string | null
  display_name?: string | null
  origin?: string | null
  lifecycle?: string | null
  context_window_tokens?: number | null
  max_input_tokens?: number | null
  max_output_tokens?: number | null
  description?: string | null
  knowledge_cutoff?: string | null
  release_date?: string | null
  last_updated?: string | null
  open_weights?: boolean | null
  supports_parallel_tool_calls?: boolean | null
  supports_verbosity?: boolean | null
  default_verbosity?: string | null
  default_temperature?: string | null
  default_top_p?: string | null
  default_top_k?: number | null
  assistant_reasoning_interleaved?: boolean | null
  assistant_reasoning_field?: string | null
  output_modalities?: string[]
  pricing?: JsonValue
  thinking_modes?: JsonValue
  speed_modes?: JsonValue
  [key: string]: JsonValue
}

type ModelCatalogListResponse = {
  summary?: ModelCatalogSummary
  total?: number
  offset?: number
  limit?: number
  available_origins?: string[]
  items?: CatalogModel[]
}

const PAGE_SIZE = 50
const toasts = useToastsStore()
const loading = ref(false)
const requestingRefresh = ref(false)
const refreshing = computed(() => requestingRefresh.value || summary.value?.refreshing === true)
const error = ref('')
const query = ref('')
const appliedQuery = ref('')
const origin = ref('')
const offset = ref(0)
const response = ref<ModelCatalogListResponse | null>(null)
const selectedModelId = ref('')

const items = computed(() => (Array.isArray(response.value?.items) ? response.value.items : []))
const total = computed(() => Math.max(0, Number(response.value?.total) || 0))
const effectiveLimit = computed(() => Math.max(1, Number(response.value?.limit) || PAGE_SIZE))
const pageNumber = computed(() => Math.floor(offset.value / effectiveLimit.value) + 1)
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / effectiveLimit.value)))
const canPrevious = computed(() => offset.value > 0 && !loading.value)
const canNext = computed(() => offset.value + effectiveLimit.value < total.value && !loading.value)
const selectedModel = computed(
  () => items.value.find((item) => item.model_id === selectedModelId.value) || items.value[0] || null,
)
const originOptions = computed(() =>
  (response.value?.available_origins || []).map((value) => ({ value, label: value })),
)
const summary = computed(() => response.value?.summary || null)
const failureText = computed(() => {
  const failure = summary.value?.last_failure
  return String(failure?.user?.fallback || failure?.rendered || failure?.message || '').trim()
})

function compactNumber(value: unknown): string {
  if (value === null || value === undefined || value === '') return '—'
  const number = Number(value)
  if (!Number.isFinite(number)) return '—'
  return Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(number)
}

function yesNo(value: unknown): string {
  if (value === true) return st('Yes')
  if (value === false) return st('No')
  return '—'
}

function text(value: unknown): string {
  return typeof value === 'number' ? String(value) : typeof value === 'string' && value.trim() ? value.trim() : '—'
}

function pretty(value: JsonValue): string {
  if (value === undefined || value === null) return '—'
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function record(value: JsonValue): Record<string, JsonValue> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}
const capabilityLabels = computed<Record<string, string>>(() => ({
  text: st('Text'),
  image: st('Image'),
  document: st('Document'),
  audio: st('Audio'),
  video: st('Video'),
  file: st('File'),
  tool_calling: st('Tool calling'),
  streaming: st('Streaming'),
  reasoning: st('Reasoning'),
  structured_output: st('Structured output'),
  temperature: st('Temperature'),
}))
function capabilities(value: JsonValue) {
  const selection = record(value)
  const supported = Array.isArray(value) ? value : Array.isArray(selection.supported) ? selection.supported : []
  const unsupported = Array.isArray(selection.unsupported) ? selection.unsupported : []
  return [
    ...supported.map((key) => ({ key: String(key), supported: true })),
    ...unsupported.map((key) => ({ key: String(key), supported: false })),
  ]
}
function modes(value: JsonValue, thinking = false) {
  const source = record((thinking ? orderedThinkingModeMap(value) : value) as JsonValue)
  return Object.entries(source)
    .filter(([key]) => key !== 'default')
    .map(([key, value]) => ({
      key,
      label: text(record(value).display_name) === '—' ? key : text(record(value).display_name),
      description: record(value).description,
      isDefault: source.default === key,
    }))
}
const priceLabels = computed(() => [
  { key: 'input_usd_per_million_tokens', label: st('Input') },
  { key: 'output_usd_per_million_tokens', label: st('Output') },
  { key: 'cache_read_usd_per_million_tokens', label: st('Cache read') },
  { key: 'cache_write_usd_per_million_tokens', label: st('Cache write') },
])
const pricing = computed(() => record(selectedModel.value?.pricing))
const pricingTiers = computed(() => (Array.isArray(pricing.value.tiers) ? pricing.value.tiers.map(record) : []))
const mobileModelOptions = computed(() =>
  items.value.map((model) => ({
    value: model.model_id,
    label: model.display_name || model.model_id,
    description: model.model_id,
  })),
)
function refreshTime(value: string) {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

function modelSubtitle(model: CatalogModel): string {
  return [
    model.origin || '',
    model.lifecycle || '',
    model.context_window_tokens
      ? st('{context_window_tokens} context', { context_window_tokens: compactNumber(model.context_window_tokens) })
      : '',
    model.max_output_tokens
      ? st('{max_output_tokens} output', { max_output_tokens: compactNumber(model.max_output_tokens) })
      : '',
  ]
    .filter(Boolean)
    .join(' · ')
}

const visibleItems = computed(() => items.value)
const visible = usePaneVisibility()
let pendingList = false
let listController: AbortController | null = null
let monitorController: AbortController | null = null
let monitoringRefresh = false
let disposed = false

async function load(options: { reset?: boolean } = {}) {
  if (disposed) return
  if (options.reset) offset.value = 0
  if (!visible.value) {
    pendingList = true
    return
  }
  pendingList = false
  listController?.abort()
  const request = new AbortController()
  listController = request
  loading.value = true
  error.value = ''
  try {
    const params = new URLSearchParams({
      offset: String(offset.value),
      limit: String(PAGE_SIZE),
    })
    if (appliedQuery.value.trim()) params.set('q', appliedQuery.value.trim())
    if (origin.value.trim()) params.set('origin', origin.value.trim())
    const next = await apiJson<ModelCatalogListResponse>(`/api/v1/model-catalog?${params.toString()}`, {
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(15_000)]),
    })
    if (request.signal.aborted || listController !== request || disposed) return
    response.value = next && typeof next === 'object' ? next : null
    const availableIds = new Set(items.value.map((item) => item.model_id))
    if (!selectedModelId.value || !availableIds.has(selectedModelId.value)) {
      selectedModelId.value = items.value[0]?.model_id || ''
    }
    if (next.summary?.refreshing) {
      monitoringRefresh = true
      refreshMonitor.invalidate(1500)
    }
  } catch (reason) {
    if (request.signal.aborted || listController !== request || disposed) return
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (listController === request) {
      listController = null
      loading.value = false
    }
  }
}

// Monitor only an active asynchronous refresh. Probe a one-row response for
// its summary, and fetch the displayed page once the task has finished.
const refreshMonitor = createRevalidator(
  async () => {
    const request = new AbortController()
    monitorController = request
    try {
      const status = await apiJson<ModelCatalogListResponse>('/api/v1/model-catalog?limit=1', {
        signal: AbortSignal.any([request.signal, AbortSignal.timeout(15_000)]),
      })
      if (disposed || request.signal.aborted) return
      if (response.value) response.value = { ...response.value, summary: status.summary }
      if (status.summary?.refreshing) refreshMonitor.invalidate(1500)
      else {
        monitoringRefresh = false
        await load()
      }
    } finally {
      if (monitorController === request) monitorController = null
    }
  },
  { intervalMs: 250, retryMs: 5000, enabled: () => !disposed && monitoringRefresh && visible.value },
)

const visibility = () => {
  if (visible.value) {
    if (pendingList) void load()
    refreshMonitor.resume()
  } else {
    refreshMonitor.pause()
    monitorController?.abort()
    if (listController) {
      pendingList = true
      listController.abort()
    }
  }
}
watch(visible, visibility, { flush: 'sync' })
onScopeDispose(() => {
  disposed = true
  listController?.abort()
  monitorController?.abort()
  refreshMonitor.dispose()
})

function search() {
  appliedQuery.value = query.value.trim()
  void load({ reset: true })
}

function clearSearch() {
  query.value = ''
  appliedQuery.value = ''
  void load({ reset: true })
}

function clearFilters() {
  query.value = ''
  appliedQuery.value = ''
  if (origin.value) origin.value = ''
  else void load({ reset: true })
}

function previousPage() {
  if (!canPrevious.value) return
  offset.value = Math.max(0, offset.value - effectiveLimit.value)
  void load()
}

function nextPage() {
  if (!canNext.value) return
  offset.value += effectiveLimit.value
  void load()
}

async function refreshCatalog() {
  if (refreshing.value) return
  requestingRefresh.value = true
  error.value = ''
  try {
    await apiJson('/api/v1/model-catalog/refresh', { method: 'POST' })
    toasts.push('success', st('Model Catalog refresh started'))
    await load({ reset: true })
    monitoringRefresh = true
    refreshMonitor.invalidate(0)
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    error.value = message
    toasts.push('error', message)
  } finally {
    requestingRefresh.value = false
  }
}

watch(origin, () => void load({ reset: true }))
onMounted(() => void load())
defineExpose({ refresh: () => load() })
</script>

<template>
  <div class="flex h-full min-h-0 min-w-0 flex-col gap-4" data-model-catalog>
    <div
      v-if="error"
      class="shrink-0 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
      role="alert"
    >
      {{ error }}
    </div>
    <div
      v-if="failureText"
      class="shrink-0 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-sm text-amber-800 dark:text-amber-200"
    >
      {{ $st('Last refresh issue:') }} {{ failureText }}
    </div>
    <section class="grid shrink-0 gap-3 rounded-xl border border-border/70 bg-card/30 p-3 sm:p-4">
      <div class="flex min-w-0 flex-wrap items-center gap-2">
        <SearchInput
          v-model="query"
          class="min-w-0 basis-full sm:basis-48 sm:flex-1"
          :placeholder="$st('Search model id, display name, origin, lifecycle, or description')"
          :input-aria-label="$st('Search Model Catalog')"
          @search="search"
          @clear="clearSearch"
        />
        <div class="min-w-0 flex-1 sm:max-w-48">
          <OptionPicker
            v-model="origin"
            :options="originOptions"
            :title="$st('Catalog origin')"
            :empty-label="$st('All origins')"
            :search-placeholder="$st('Search origins...')"
          />
        </div>
        <Button variant="outline" size="sm" :disabled="refreshing || loading" @click="refreshCatalog"
          ><RiRestartLine class="mr-1.5 h-4 w-4" :class="refreshing ? 'animate-spin' : ''" />{{
            $st('Refresh source')
          }}</Button
        >
        <IconButton
          variant="ghost"
          size="sm"
          :disabled="loading"
          :tooltip="$st('Reload current page')"
          :aria-label="$st('Reload Model Catalog page')"
          @click="load()"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''"
        /></IconButton>
      </div>
      <div
        class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-xs leading-relaxed text-muted-foreground"
      >
        <span>{{
          $st('{matching} matching models · {total} total catalog entries · {source}', {
            matching: total,
            total: summary?.model_count ?? total,
            source: summary?.last_successful_source || $st('source not reported'),
          })
        }}</span>
        <span v-if="summary?.last_refresh_at">{{
          $st('Last refreshed {time}', { time: refreshTime(summary.last_refresh_at) })
        }}</span>
      </div>
    </section>

    <div
      class="grid min-w-0 gap-4 lg:min-h-0 lg:flex-1 lg:grid-cols-[minmax(13rem,28%)_minmax(0,1fr)] lg:grid-rows-[minmax(0,1fr)]"
    >
      <section
        class="flex min-w-0 flex-col overflow-hidden rounded-xl border border-border/70 bg-card/30 lg:min-h-0"
        :aria-label="$st('Catalog models')"
      >
        <div
          class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 bg-muted/20 px-3 py-3 text-xs"
        >
          <span class="font-medium">{{ $st('Catalog models') }}</span
          ><span class="tabular-nums text-muted-foreground">{{
            $st('{count} shown', { count: visibleItems.length })
          }}</span>
        </div>
        <div class="min-w-0 p-3 lg:hidden">
          <OptionPicker
            :model-value="selectedModel?.model_id || ''"
            :options="mobileModelOptions"
            :include-empty="false"
            :title="$st('Catalog models')"
            :placeholder="$st('Select a catalog model to inspect it.')"
            :disabled="loading || !visibleItems.length"
            @update:model-value="selectedModelId = $event"
          />
        </div>
        <div class="hidden overflow-y-auto p-2 lg:block lg:min-h-0 lg:max-h-none lg:flex-1">
          <button
            v-for="model in visibleItems"
            :key="model.model_id"
            type="button"
            class="mb-1 grid w-full min-w-0 gap-1 rounded-lg border px-3 py-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="
              selectedModel?.model_id === model.model_id
                ? 'border-primary/25 bg-primary/8'
                : 'border-transparent hover:bg-muted/40'
            "
            :aria-current="selectedModel?.model_id === model.model_id ? 'true' : undefined"
            @click="selectedModelId = model.model_id"
          >
            <span class="truncate text-sm font-medium">{{ model.display_name || model.model_id }}</span>
            <code class="truncate font-mono text-[11px] text-muted-foreground">{{ model.model_id }}</code>
            <span class="line-clamp-2 text-xs leading-relaxed text-muted-foreground">{{
              modelSubtitle(model) || $st('No summary')
            }}</span>
          </button>
        </div>
        <div v-if="loading && !items.length" class="p-4 text-center text-xs text-muted-foreground" role="status">
          {{ $st('Loading Model Catalog…') }}
        </div>
        <div v-if="!loading && !visibleItems.length" class="grid gap-3 p-4 text-center text-xs text-muted-foreground">
          <span>{{ $st('No catalog models match this page and filter.') }}</span>
          <Button v-if="appliedQuery || origin" variant="outline" size="sm" @click="clearFilters">{{
            $st('Clear filters')
          }}</Button>
        </div>
        <div class="flex items-center justify-between gap-1 border-t border-border/60 p-2">
          <IconButton
            variant="ghost"
            size="sm"
            :disabled="!canPrevious"
            :tooltip="$st('Previous')"
            :aria-label="$st('Previous')"
            @click="previousPage"
            ><RiArrowLeftSLine class="h-4 w-4"
          /></IconButton>
          <span class="text-xs tabular-nums text-muted-foreground">{{
            $st('Page {page} of {pages}', { page: pageNumber, pages: pageCount })
          }}</span>
          <IconButton
            variant="ghost"
            size="sm"
            :disabled="!canNext"
            :tooltip="$st('Next')"
            :aria-label="$st('Next')"
            @click="nextPage"
            ><RiArrowRightSLine class="h-4 w-4"
          /></IconButton>
        </div>
      </section>
      <article
        v-if="selectedModel"
        :key="selectedModel.model_id"
        class="min-w-0 space-y-4 lg:min-h-0 lg:overflow-y-auto lg:pr-1"
        :aria-label="selectedModel.display_name || selectedModel.model_id"
      >
        <header class="rounded-xl border border-border/70 bg-card/30 p-4 sm:p-5">
          <div class="mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span v-if="selectedModel.origin">{{ selectedModel.origin }}</span
            ><span v-if="selectedModel.lifecycle" class="rounded-md bg-muted/60 px-2 py-0.5">{{
              selectedModel.lifecycle
            }}</span>
          </div>
          <h2 class="break-words text-xl font-semibold">{{ selectedModel.display_name || selectedModel.model_id }}</h2>
          <code class="mt-1 block break-all font-mono text-xs text-muted-foreground">{{ selectedModel.model_id }}</code>
          <p
            v-if="selectedModel.description"
            class="mt-3 whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground"
          >
            {{ selectedModel.description }}
          </p>
          <dl class="mt-4 grid grid-cols-3 gap-3 border-t border-border/60 pt-4">
            <div
              v-for="item in [
                { label: $st('Context window'), value: selectedModel.context_window_tokens },
                { label: $st('Max input'), value: selectedModel.max_input_tokens },
                { label: $st('Max output'), value: selectedModel.max_output_tokens },
              ]"
              :key="item.label"
              class="min-w-0"
            >
              <dt class="text-xs leading-relaxed text-muted-foreground">{{ item.label }}</dt>
              <dd
                class="mt-1 text-lg font-semibold tabular-nums sm:text-xl"
                :title="item.value == null ? '' : String(item.value)"
              >
                {{ compactNumber(item.value) }}
              </dd>
            </div>
          </dl>
        </header>
        <SettingsCard :title="$st('Capabilities')">
          <div class="grid gap-4">
            <div
              v-for="group in [
                { label: $st('Features'), value: selectedModel.features },
                { label: $st('Input modalities'), value: selectedModel.input },
              ]"
              :key="group.label"
              class="grid gap-2"
            >
              <h3 class="text-xs text-muted-foreground">{{ group.label }}</h3>
              <div class="flex flex-wrap gap-1.5">
                <span
                  v-for="capability in capabilities(group.value)"
                  :key="capability.key"
                  class="rounded-md border px-2 py-1 text-xs"
                  :class="
                    capability.supported ? 'border-border/60 bg-muted/30' : 'border-border/40 text-muted-foreground'
                  "
                  >{{ capabilityLabels[capability.key] || capability.key
                  }}<span v-if="!capability.supported"> · {{ $st('Not supported') }}</span></span
                >
                <span v-if="!capabilities(group.value).length" class="text-xs text-muted-foreground">{{
                  $st('Not reported')
                }}</span>
              </div>
            </div>
            <dl class="grid gap-3 border-t border-border/50 pt-3 sm:grid-cols-3">
              <div>
                <dt class="text-xs text-muted-foreground">{{ $st('Output modalities') }}</dt>
                <dd class="mt-1 text-sm">
                  {{ selectedModel.output_modalities?.map((key) => capabilityLabels[key] || key).join(', ') || '—' }}
                </dd>
              </div>
              <div>
                <dt class="text-xs text-muted-foreground">{{ $st('Parallel tools') }}</dt>
                <dd class="mt-1 text-sm">{{ yesNo(selectedModel.supports_parallel_tool_calls) }}</dd>
              </div>
              <div>
                <dt class="text-xs text-muted-foreground">{{ $st('Open weights') }}</dt>
                <dd class="mt-1 text-sm">{{ yesNo(selectedModel.open_weights) }}</dd>
              </div>
            </dl>
          </div>
        </SettingsCard>
        <SettingsCard :title="$st('Response modes')">
          <div class="grid gap-4 sm:grid-cols-2">
            <div
              v-for="group in [
                { label: $st('Thinking modes'), items: modes(selectedModel.thinking_modes, true) },
                { label: $st('Speed modes'), items: modes(selectedModel.speed_modes) },
              ]"
              :key="group.label"
              class="min-w-0"
            >
              <h3 class="mb-2 text-xs text-muted-foreground">{{ group.label }}</h3>
              <div class="flex flex-wrap gap-1.5">
                <span
                  v-for="mode in group.items"
                  :key="mode.key"
                  :title="typeof mode.description === 'string' ? mode.description : undefined"
                  class="max-w-full break-words rounded-md border border-border/60 bg-muted/20 px-2 py-1 text-xs"
                  >{{ mode.label
                  }}<span v-if="mode.isDefault" class="ml-1 text-primary">· {{ $st('Default') }}</span></span
                ><span v-if="!group.items.length" class="text-xs text-muted-foreground">{{ $st('Not reported') }}</span>
              </div>
            </div>
          </div>
        </SettingsCard>
        <SettingsCard
          :title="$st('Pricing')"
          :description="$st('USD per million tokens. Missing prices are not reported by the catalog.')"
        >
          <dl class="grid grid-cols-2 gap-4 xl:grid-cols-4">
            <div v-for="price in priceLabels" :key="price.key" class="min-w-0">
              <dt class="text-xs text-muted-foreground">{{ price.label }}</dt>
              <dd class="mt-1 break-all text-base font-medium tabular-nums">
                {{ text(pricing[price.key]) === '—' ? '—' : '$' + text(pricing[price.key]) }}
              </dd>
            </div>
          </dl>
          <details v-if="pricingTiers.length" class="mt-4 border-t border-border/60 pt-3">
            <summary class="cursor-pointer text-xs font-medium">{{ $st('Pricing tiers') }}</summary>
            <div v-for="(tier, index) in pricingTiers" :key="index" class="mt-3 rounded-lg bg-muted/20 p-3">
              <div class="mb-2 text-xs font-medium">
                {{ text(tier.tier_type) }} · {{ compactNumber(tier.size_tokens) }} {{ $st('tokens') }}
              </div>
              <dl class="grid grid-cols-2 gap-3">
                <div v-for="price in priceLabels" :key="price.key">
                  <dt class="text-xs text-muted-foreground">{{ price.label }}</dt>
                  <dd class="text-sm">{{ text(tier[price.key]) === '—' ? '—' : '$' + text(tier[price.key]) }}</dd>
                </div>
              </dl>
            </div>
          </details>
        </SettingsCard>
        <SettingsCard :title="$st('Model information')">
          <dl class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
            <div
              v-for="item in [
                { label: $st('Source'), value: selectedModel.source_label || selectedModel.source },
                { label: $st('Knowledge cutoff'), value: selectedModel.knowledge_cutoff },
                { label: $st('Release date'), value: selectedModel.release_date },
                { label: $st('Last updated'), value: selectedModel.last_updated },
              ]"
              :key="item.label"
              class="min-w-0"
            >
              <dt class="text-xs text-muted-foreground">{{ item.label }}</dt>
              <dd class="mt-1 break-words text-sm">{{ text(item.value) }}</dd>
            </div>
          </dl>
        </SettingsCard>
        <details class="min-w-0 rounded-xl border border-border/70 bg-card/30">
          <summary class="cursor-pointer px-4 py-3 text-sm font-medium">{{ $st('Advanced model details') }}</summary>
          <div class="grid min-w-0 gap-4 border-t border-border/60 p-4">
            <dl class="grid gap-4 sm:grid-cols-2">
              <div
                v-for="item in [
                  { label: $st('Verbosity'), value: yesNo(selectedModel.supports_verbosity) },
                  { label: $st('Default verbosity'), value: selectedModel.default_verbosity },
                  { label: $st('Temperature'), value: selectedModel.default_temperature },
                  { label: 'Top P', value: selectedModel.default_top_p },
                  { label: 'Top K', value: selectedModel.default_top_k },
                  { label: $st('Reasoning interleaved'), value: yesNo(selectedModel.assistant_reasoning_interleaved) },
                  { label: $st('Reasoning field'), value: selectedModel.assistant_reasoning_field },
                ]"
                :key="item.label"
                class="min-w-0"
              >
                <dt class="text-xs text-muted-foreground">{{ item.label }}</dt>
                <dd class="mt-1 break-all font-mono text-xs">{{ text(item.value) }}</dd>
              </div>
            </dl>
            <h3 class="text-xs text-muted-foreground">{{ $st('Raw catalog entry') }}</h3>
            <pre class="max-h-96 min-w-0 overflow-auto rounded-lg bg-muted/25 p-3 font-mono text-xs leading-5">{{
              pretty(selectedModel)
            }}</pre>
          </div>
        </details>
      </article>
      <div
        v-else
        class="grid min-h-48 place-content-center rounded-xl border border-dashed border-border/70 p-6 text-center text-sm text-muted-foreground"
      >
        {{ $st('Select a catalog model to inspect it.') }}
      </div>
    </div>
  </div>
</template>
