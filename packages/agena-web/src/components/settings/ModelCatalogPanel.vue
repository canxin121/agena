<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RiArrowLeftSLine, RiArrowRightSLine, RiRefreshLine, RiRestartLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'

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
const KNOWN_MODEL_KEYS = new Set([
  'model_id',
  'source',
  'source_label',
  'display_name',
  'origin',
  'lifecycle',
  'context_window_tokens',
  'max_input_tokens',
  'max_output_tokens',
  'description',
  'knowledge_cutoff',
  'release_date',
  'last_updated',
  'open_weights',
  'supports_parallel_tool_calls',
  'supports_verbosity',
  'default_verbosity',
  'default_temperature',
  'default_top_p',
  'default_top_k',
  'assistant_reasoning_interleaved',
  'assistant_reasoning_field',
  'output_modalities',
  'pricing',
  'thinking_modes',
  'speed_modes',
])

const toasts = useToastsStore()
const loading = ref(false)
const refreshing = ref(false)
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
  const number = Number(value)
  if (!Number.isFinite(number)) return '—'
  return Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(number)
}

function yesNo(value: unknown): string {
  if (value === true) return 'Yes'
  if (value === false) return 'No'
  return '—'
}

function text(value: unknown): string {
  return typeof value === 'string' && value.trim() ? value.trim() : '—'
}

function pretty(value: JsonValue): string {
  if (value === undefined || value === null) return '—'
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function capabilityValue(model: CatalogModel | null): JsonValue {
  if (!model) return null
  return Object.fromEntries(Object.entries(model).filter(([key]) => !KNOWN_MODEL_KEYS.has(key))) as JsonValue
}

function modelSubtitle(model: CatalogModel): string {
  return [
    model.origin || '',
    model.lifecycle || '',
    model.context_window_tokens ? `${compactNumber(model.context_window_tokens)} context` : '',
    model.max_output_tokens ? `${compactNumber(model.max_output_tokens)} output` : '',
  ]
    .filter(Boolean)
    .join(' · ')
}

const visibleItems = computed(() => items.value)

async function load(options: { reset?: boolean } = {}) {
  if (loading.value) return
  if (options.reset) offset.value = 0
  loading.value = true
  error.value = ''
  try {
    const params = new URLSearchParams({
      offset: String(offset.value),
      limit: String(PAGE_SIZE),
    })
    if (appliedQuery.value.trim()) params.set('q', appliedQuery.value.trim())
    if (origin.value.trim()) params.set('origin', origin.value.trim())
    const next = await apiJson<ModelCatalogListResponse>(`/api/v1/model-catalog?${params.toString()}`)
    response.value = next && typeof next === 'object' ? next : null
    const availableIds = new Set(items.value.map((item) => item.model_id))
    if (!selectedModelId.value || !availableIds.has(selectedModelId.value)) {
      selectedModelId.value = items.value[0]?.model_id || ''
    }
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

function search() {
  appliedQuery.value = query.value.trim()
  void load({ reset: true })
}

function clearSearch() {
  query.value = ''
  appliedQuery.value = ''
  void load({ reset: true })
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
  refreshing.value = true
  error.value = ''
  try {
    await apiJson('/api/v1/model-catalog/refresh', { method: 'POST' })
    toasts.push('success', 'Model Catalog refresh started')
    await load({ reset: true })
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    error.value = message
    toasts.push('error', message)
  } finally {
    refreshing.value = false
  }
}

watch(origin, () => void load({ reset: true }))
onMounted(() => void load())
</script>

<template>
  <div class="grid min-w-0 gap-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-semibold">Resolved Model Catalog</h2>
        <p class="mt-1 max-w-3xl text-sm text-muted-foreground">
          Search the runtime’s merged catalog rather than guessing model capabilities from provider names.
        </p>
      </div>
      <div class="flex items-center gap-2">
        <Button variant="outline" size="sm" :disabled="refreshing || loading" @click="refreshCatalog">
          <RiRestartLine class="mr-2 h-4 w-4" :class="refreshing ? 'animate-spin' : ''" />
          Refresh source
        </Button>
        <IconButton
          variant="outline"
          size="md"
          :disabled="loading"
          :tooltip="loading ? 'Loading catalog' : 'Reload current page'"
          aria-label="Reload Model Catalog page"
          @click="load()"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </div>
    </div>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>
    <div
      v-if="failureText"
      class="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-800 dark:text-amber-200"
    >
      Last refresh issue: {{ failureText }}
    </div>

    <section class="grid gap-3 rounded-lg border border-border/60 bg-muted/10 p-4">
      <div class="grid gap-3 md:grid-cols-[minmax(0,1fr)_16rem]">
        <SearchInput
          v-model="query"
          placeholder="Search model id, display name, origin, lifecycle, or description"
          input-aria-label="Search Model Catalog"
          @search="search"
          @clear="clearSearch"
        />
        <OptionPicker
          v-model="origin"
          :options="originOptions"
          title="Catalog origin"
          empty-label="All origins"
          search-placeholder="Search origins..."
        />
      </div>
      <div class="flex flex-wrap items-center justify-between gap-3 text-xs text-muted-foreground">
        <span>
          {{ total }} matching models · {{ summary?.model_count ?? total }} total catalog entries ·
          {{ summary?.last_successful_source || 'source not reported' }}
        </span>
        <span v-if="summary?.last_refresh_at">Last refreshed {{ summary.last_refresh_at }}</span>
      </div>
    </section>

    <div
      class="grid min-h-[38rem] min-w-0 overflow-hidden rounded-lg border border-border/60 lg:grid-cols-[22rem_minmax(0,1fr)]"
    >
      <div class="min-w-0 border-b border-border/60 bg-muted/10 lg:border-b-0 lg:border-r">
        <div
          class="flex items-center justify-between border-b border-border/60 px-3 py-2 text-xs text-muted-foreground"
        >
          <span>Page {{ pageNumber }} of {{ pageCount }}</span>
          <span>{{ visibleItems.length }} shown</span>
        </div>
        <div class="max-h-[43rem] overflow-y-auto p-2">
          <button
            v-for="model in visibleItems"
            :key="model.model_id"
            type="button"
            class="grid w-full min-w-0 gap-1 rounded-md px-3 py-2.5 text-left transition-colors"
            :class="
              selectedModel?.model_id === model.model_id ? 'bg-primary/10 ring-1 ring-primary/20' : 'hover:bg-muted/60'
            "
            @click="selectedModelId = model.model_id"
          >
            <span class="truncate text-sm font-medium">{{ model.display_name || model.model_id }}</span>
            <code class="truncate font-mono text-[11px] text-muted-foreground">{{ model.model_id }}</code>
            <span class="line-clamp-2 text-[11px] text-muted-foreground">{{
              modelSubtitle(model) || 'No summary'
            }}</span>
          </button>
          <div
            v-if="!loading && visibleItems.length === 0"
            class="px-4 py-12 text-center text-sm text-muted-foreground"
          >
            No catalog models match this page and filter.
          </div>
          <div v-if="loading && items.length === 0" class="px-4 py-12 text-center text-sm text-muted-foreground">
            Loading Model Catalog…
          </div>
        </div>
        <div class="flex items-center justify-between border-t border-border/60 p-2">
          <Button variant="ghost" size="sm" :disabled="!canPrevious" @click="previousPage">
            <RiArrowLeftSLine class="mr-1 h-4 w-4" /> Previous
          </Button>
          <Button variant="ghost" size="sm" :disabled="!canNext" @click="nextPage">
            Next <RiArrowRightSLine class="ml-1 h-4 w-4" />
          </Button>
        </div>
      </div>

      <article v-if="selectedModel" class="min-w-0 overflow-y-auto p-4 lg:max-h-[47rem] lg:p-5">
        <header class="border-b border-border/60 pb-4">
          <h3 class="break-words text-lg font-semibold">{{ selectedModel.display_name || selectedModel.model_id }}</h3>
          <code class="mt-1 block break-all font-mono text-xs text-muted-foreground">{{ selectedModel.model_id }}</code>
          <p v-if="selectedModel.description" class="mt-3 whitespace-pre-wrap text-sm leading-6 text-muted-foreground">
            {{ selectedModel.description }}
          </p>
        </header>

        <dl class="grid gap-x-6 gap-y-4 py-5 sm:grid-cols-2 xl:grid-cols-3">
          <div>
            <dt class="text-xs text-muted-foreground">Origin</dt>
            <dd class="mt-1 text-sm">{{ text(selectedModel.origin) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Lifecycle</dt>
            <dd class="mt-1 text-sm">{{ text(selectedModel.lifecycle) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Source</dt>
            <dd class="mt-1 text-sm">{{ selectedModel.source_label || selectedModel.source }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Context window</dt>
            <dd class="mt-1 font-mono text-sm">{{ compactNumber(selectedModel.context_window_tokens) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Max input</dt>
            <dd class="mt-1 font-mono text-sm">{{ compactNumber(selectedModel.max_input_tokens) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Max output</dt>
            <dd class="mt-1 font-mono text-sm">{{ compactNumber(selectedModel.max_output_tokens) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Knowledge cutoff</dt>
            <dd class="mt-1 text-sm">{{ text(selectedModel.knowledge_cutoff) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Release date</dt>
            <dd class="mt-1 text-sm">{{ text(selectedModel.release_date) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Last updated</dt>
            <dd class="mt-1 text-sm">{{ text(selectedModel.last_updated) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Open weights</dt>
            <dd class="mt-1 text-sm">{{ yesNo(selectedModel.open_weights) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Parallel tools</dt>
            <dd class="mt-1 text-sm">{{ yesNo(selectedModel.supports_parallel_tool_calls) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Verbosity</dt>
            <dd class="mt-1 text-sm">
              {{ yesNo(selectedModel.supports_verbosity) }} · default {{ text(selectedModel.default_verbosity) }}
            </dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Temperature</dt>
            <dd class="mt-1 font-mono text-sm">{{ text(selectedModel.default_temperature) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Top P</dt>
            <dd class="mt-1 font-mono text-sm">{{ text(selectedModel.default_top_p) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Top K</dt>
            <dd class="mt-1 font-mono text-sm">{{ selectedModel.default_top_k ?? '—' }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Reasoning interleaved</dt>
            <dd class="mt-1 text-sm">{{ yesNo(selectedModel.assistant_reasoning_interleaved) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Reasoning field</dt>
            <dd class="mt-1 font-mono text-sm">{{ text(selectedModel.assistant_reasoning_field) }}</dd>
          </div>
          <div>
            <dt class="text-xs text-muted-foreground">Output modalities</dt>
            <dd class="mt-1 text-sm">{{ selectedModel.output_modalities?.join(', ') || '—' }}</dd>
          </div>
        </dl>

        <div class="grid gap-3">
          <details class="rounded-md border border-border/60" open>
            <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Capabilities</summary>
            <pre class="max-h-72 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
              pretty(capabilityValue(selectedModel))
            }}</pre>
          </details>
          <details class="rounded-md border border-border/60">
            <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Thinking modes</summary>
            <pre class="max-h-72 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
              pretty(selectedModel.thinking_modes)
            }}</pre>
          </details>
          <details class="rounded-md border border-border/60">
            <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Speed modes</summary>
            <pre class="max-h-72 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
              pretty(selectedModel.speed_modes)
            }}</pre>
          </details>
          <details class="rounded-md border border-border/60">
            <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Pricing</summary>
            <pre class="max-h-72 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
              pretty(selectedModel.pricing)
            }}</pre>
          </details>
          <details class="rounded-md border border-border/60">
            <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Raw catalog entry</summary>
            <pre class="max-h-[32rem] overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
              pretty(selectedModel)
            }}</pre>
          </details>
        </div>
      </article>
      <div v-else class="flex min-h-[28rem] items-center justify-center p-8 text-sm text-muted-foreground">
        Select a catalog model to inspect it.
      </div>
    </div>
  </div>
</template>
