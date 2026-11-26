<script setup lang="ts">
// The composer's context chip opens this panel: the live context window state
// the chip already shows, plus the conversation's recorded token and cost
// totals from the server's cost projection.
import { computed, ref, watch } from 'vue'
import { RiPieChartLine } from '@remixicon/vue'

import SessionChip from './SessionChip.vue'
import { apiJson } from '@/lib/api'
import { settingsText as st } from '@/i18n/settingsText'
import { formatUsageCount, formatUsagePercent } from '@/lib/usageFormatting'
import { formatDurationMs, formatTokenRate, requestPerformanceMetrics } from '@/lib/usagePerformance'
import type { RequestPerformance } from '@/types/usage'

type ContextUsageView = {
  tokensValue?: number | null
  tokensLabel: string
  percentUsed: number | null
  capacityLabel?: string
  lastCacheReadTokens?: number | null
  lastCacheInputTokens?: number | null
  lastCacheHitRate?: number | null
  lastPerformance?: RequestPerformance | null
}

type CostSummary = {
  requests?: number
  input_tokens?: number
  output_tokens?: number
  reasoning_tokens?: number
  cache_read_tokens?: number
  cache_write_tokens?: number
  cache_input_tokens?: number
  cache_hit_rate?: number
  other_tokens?: number
  total_tokens?: number
  total_cost_usd?: number
}

const props = defineProps<{
  sessionId: string
  usage: ContextUsageView
  open: boolean
  busy?: boolean
  contextOnly?: boolean
}>()
const emit = defineEmits<{ (event: 'update:open', value: boolean): void }>()

const chipLabel = computed(() =>
  props.usage.percentUsed !== null ? `${props.usage.percentUsed}%` : props.usage.tokensLabel,
)

const summary = ref<CostSummary | null>(null)
const loading = ref(false)
const error = ref('')
let generation = 0

function finite(value: unknown): number {
  const number = Number(value)
  return Number.isFinite(number) && number >= 0 ? number : 0
}

// Keep the last request's cache measurements beside the live context state;
// the separate conversation grid contains the cumulative measurements.
const liveCacheRows = computed(() => {
  const usage = props.usage
  if (usage.lastCacheHitRate == null && usage.lastCacheReadTokens == null) return []
  return [
    { key: 'live-hit', label: st('Last request cache hit'), value: formatUsagePercent(usage.lastCacheHitRate) },
    {
      key: 'live-read',
      label: st('Last request cache read'),
      value: usage.lastCacheReadTokens == null ? '—' : formatUsageCount(usage.lastCacheReadTokens),
    },
  ]
})

const performanceRows = computed(() => {
  const metrics = requestPerformanceMetrics(props.usage.lastPerformance)
  return [
    {
      key: 'generation',
      label: st('Generation speed'),
      value: formatTokenRate(metrics.tokensPerSecond),
      hint: st('Output and reasoning tokens divided by the time between the first and last output chunks.'),
    },
    {
      key: 'ttft',
      label: st('First output latency'),
      value: formatDurationMs(metrics.ttftMs),
      hint: st('Time to the first non-empty text, reasoning, or tool-call output.'),
    },
    {
      key: 'duration',
      label: st('Request duration'),
      value: formatDurationMs(metrics.durationMs),
      hint: st('Time from sending the model request to its completion.'),
    },
    {
      key: 'throughput',
      label: st('End-to-end speed'),
      value: formatTokenRate(metrics.endToEndTokensPerSecond),
      hint: st('Output and reasoning tokens divided by the complete model request duration.'),
    },
    {
      key: 'generation-duration',
      label: st('Generation duration'),
      value: formatDurationMs(metrics.generationMs),
      hint: st('Time between the first and last output chunks, excluding trailing usage delivery.'),
    },
  ]
})

const rows = computed(() => {
  const value = summary.value
  if (!value) return []
  const input = finite(value.input_tokens)
  const output = finite(value.output_tokens)
  const reasoning = finite(value.reasoning_tokens)
  const cacheRead = finite(value.cache_read_tokens)
  const cacheWrite = finite(value.cache_write_tokens)
  const other = finite(value.other_tokens)
  const total = finite(value.total_tokens) || input + output + reasoning + cacheRead + cacheWrite + other
  return [
    { key: 'input', label: st('Uncached input tokens'), value: formatUsageCount(input) },
    { key: 'output', label: st('Output tokens'), value: formatUsageCount(output) },
    { key: 'reasoning', label: st('Reasoning tokens'), value: formatUsageCount(reasoning) },
    { key: 'cache-read', label: st('Cache read tokens'), value: formatUsageCount(cacheRead) },
    { key: 'cache-write', label: st('Cache write tokens'), value: formatUsageCount(cacheWrite) },
    ...(other > 0 ? [{ key: 'other', label: st('Other tokens'), value: formatUsageCount(other) }] : []),
    {
      key: 'cache-hit',
      label: st('Conversation cache hit'),
      value: formatUsagePercent(value.cache_input_tokens ? value.cache_hit_rate : null),
    },
    { key: 'requests', label: st('Requests'), value: formatUsageCount(value.requests) },
    { key: 'cacheable', label: st('Cacheable input'), value: formatUsageCount(value.cache_input_tokens) },
    { key: 'total', label: st('Total tokens'), value: formatUsageCount(total) },
  ]
})

const costLabel = computed(() => {
  const total = summary.value?.total_cost_usd
  if (typeof total !== 'number' || !Number.isFinite(total)) return ''
  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 6,
  }).format(total)
})

async function load() {
  if (props.contextOnly) return
  const sid = String(props.sessionId || '').trim()
  if (!sid) return
  const request = ++generation
  loading.value = true
  error.value = ''
  try {
    const value = await apiJson<CostSummary>(`/api/v1/sessions/${encodeURIComponent(sid)}/cost`)
    if (request !== generation) return
    summary.value = value && typeof value === 'object' ? value : null
  } catch (reason) {
    if (request !== generation) return
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (request === generation) loading.value = false
  }
}

// Refresh each time the panel opens so the totals match the conversation now,
// not whatever they were the first time the user looked.
watch(
  () => props.open,
  (open) => {
    if (open) void load()
  },
  { immediate: true },
)
watch(
  () => props.sessionId,
  () => {
    generation += 1
    summary.value = null
    loading.value = false
    error.value = ''
    if (props.open) void load()
  },
)
watch(
  () => props.busy,
  (busy, wasBusy) => {
    if (wasBusy && !busy && props.open) void load()
  },
)
</script>

<template>
  <SessionChip
    :title="$st('Context usage')"
    :label="chipLabel"
    :hint="usage.capacityLabel || $st('Context usage')"
    :open="open"
    :busy="busy || loading"
    panel-class="w-[min(92vw,34rem)]"
    @update:open="emit('update:open', $event)"
  >
    <template #icon>
      <RiPieChartLine class="h-3 w-3" />
    </template>
    <div class="context-usage-panel space-y-2.5 text-xs">
      <div class="space-y-2">
        <div class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
          <span class="text-muted-foreground">{{ $st('Current context') }}</span>
          <div class="flex min-w-0 flex-wrap items-center gap-2">
            <span class="font-mono tabular-nums [overflow-wrap:anywhere]">
              {{ usage.capacityLabel || (usage.tokensValue != null ? formatUsageCount(usage.tokensValue) : '—') }}
            </span>
            <span
              v-if="usage.percentUsed !== null"
              class="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] font-medium tabular-nums"
            >
              {{ usage.percentUsed }}%
            </span>
          </div>
        </div>
        <div
          v-if="usage.percentUsed !== null"
          class="h-1.5 overflow-hidden rounded-full bg-muted"
          role="meter"
          :aria-valuenow="usage.percentUsed"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-label="$st('Context usage')"
        >
          <div
            class="h-full rounded-full transition-[width]"
            :class="
              usage.percentUsed >= 85 ? 'bg-destructive' : usage.percentUsed >= 60 ? 'bg-amber-500' : 'bg-primary'
            "
            :style="{ width: `${usage.percentUsed}%` }"
          />
        </div>
        <dl v-if="liveCacheRows.length" class="grid grid-cols-2 gap-x-4 rounded-lg bg-muted/30 px-2.5 py-1.5">
          <div v-for="row in liveCacheRows" :key="row.key" class="flex min-w-0 items-baseline justify-between gap-2">
            <dt class="min-w-0 leading-4 text-muted-foreground [overflow-wrap:anywhere]">{{ row.label }}</dt>
            <dd class="max-w-[60%] shrink-0 text-right font-mono font-medium tabular-nums [overflow-wrap:anywhere]">
              {{ row.value }}
            </dd>
          </div>
        </dl>
      </div>

      <section
        v-if="!contextOnly"
        class="space-y-2 border-t border-border/50 pt-2.5"
        :aria-label="$st('Last completed request')"
      >
        <h3 class="font-medium">{{ $st('Last completed request') }}</h3>
        <dl v-if="usage.lastPerformance" class="context-performance-grid grid grid-cols-2 gap-x-4 gap-y-2">
          <div
            v-for="row in performanceRows"
            :key="row.key"
            class="min-w-0"
            :class="row.key === 'generation' ? 'context-performance-primary rounded-lg bg-primary/5' : 'space-y-0.5'"
          >
            <dt class="leading-4 text-muted-foreground [overflow-wrap:anywhere]" :title="row.hint">
              {{ row.label }}
            </dt>
            <dd
              class="font-mono tabular-nums [overflow-wrap:anywhere]"
              :class="row.key === 'generation' ? 'text-lg font-semibold text-primary' : 'text-sm font-medium leading-5'"
            >
              {{ row.value }}
            </dd>
          </div>
        </dl>
        <p v-else class="text-muted-foreground">{{ $st('Performance appears after a completed model request.') }}</p>
      </section>

      <section
        v-if="!contextOnly"
        class="space-y-2 border-t border-border/50 pt-2.5"
        :aria-label="$st('Conversation totals')"
      >
        <h3 class="font-medium">{{ $st('Conversation totals') }}</h3>
        <p v-if="error" role="alert" class="break-words text-destructive">{{ error }}</p>
        <p v-else-if="loading && !summary" class="text-muted-foreground">{{ $st('Loading…') }}</p>
        <dl v-else-if="rows.length" class="context-usage-grid grid grid-cols-2 gap-x-4 gap-y-2">
          <div v-for="row in rows" :key="row.key" class="flex min-w-0 items-baseline justify-between gap-2">
            <dt class="min-w-0 leading-4 text-muted-foreground [overflow-wrap:anywhere]">{{ row.label }}</dt>
            <dd
              class="max-w-[60%] shrink-0 text-right font-mono text-sm leading-5 tabular-nums [overflow-wrap:anywhere]"
              :class="row.key === 'total' ? 'font-semibold' : 'font-medium'"
            >
              {{ row.value }}
            </dd>
          </div>
        </dl>
        <p v-else class="text-muted-foreground">{{ $st('No recorded usage for this conversation yet.') }}</p>
        <div
          v-if="costLabel"
          class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 border-t border-border/50 pt-2"
        >
          <span class="text-muted-foreground">{{ $st('Estimated cost') }}</span>
          <span class="min-w-0 font-mono text-sm font-semibold tabular-nums [overflow-wrap:anywhere]">{{
            costLabel
          }}</span>
        </div>
      </section>
    </div>
  </SessionChip>
</template>

<style scoped>
.context-usage-panel {
  container: context-usage / inline-size;
}

.context-performance-primary {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  padding: 0.625rem 0.75rem;
}

/* Respond to the panel's usable width, including the user's font scale. */
@container context-usage (min-width: 25rem) {
  .context-performance-grid,
  .context-usage-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .context-performance-primary {
    grid-column: auto;
    grid-row: span 2;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
  }
}
</style>
