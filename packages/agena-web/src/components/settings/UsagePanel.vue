<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiRefreshLine, RiBarChartBoxLine, RiCoinsLine, RiTokenSwapLine, RiSpeedUpLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneRead } from '@/composables/usePaneRead'
import { formatDurationMs, formatTokenRate } from '@/lib/usagePerformance'
import type { UsagePerformanceTotals } from '@/types/usage'

type UsageTotals = {
  performance?: UsagePerformanceTotals | null
  requests: number
  sessions: number
  input_tokens: number
  output_tokens: number
  reasoning_tokens: number
  cache_read_tokens: number
  cache_write_tokens?: number
  cache_input_tokens?: number
  cache_hit_rate?: number
  total_tokens: number
  total_cost_usd: number
  recorded_cost_usd: number
  estimated_cost_usd: number
  unpriced_requests: number
}

type UsageBreakdown = UsageTotals & {
  date?: string
  provider_id?: string
  model_id?: string
  session_id?: number
  title?: string
}

type UsageStats = {
  generated_at: string
  period: string
  period_label: string
  from?: string | null
  to?: string | null
  totals: UsageTotals
  active_days: number
  average_cost_per_request_usd: number
  average_tokens_per_request: number
  peak_cost_date?: string | null
  peak_cost_usd: number
  peak_tokens_date?: string | null
  peak_tokens: number
  by_day: UsageBreakdown[]
  by_provider: UsageBreakdown[]
  by_model: UsageBreakdown[]
  by_session: UsageBreakdown[]
}

type UsageSection = {
  key: keyof Pick<UsageStats, 'by_day' | 'by_provider' | 'by_model' | 'by_session'>
  label: string
  rows: UsageBreakdown[]
}

const stats = ref<UsageStats | null>(null)
const selectedBreakdown = ref<UsageSection['key']>('by_day')

const sections = computed<UsageSection[]>(() => {
  const value = stats.value
  if (!value) return []
  return [
    { key: 'by_day', label: st('By day'), rows: Array.isArray(value.by_day) ? value.by_day : [] },
    {
      key: 'by_provider',
      label: st('By provider'),
      rows: Array.isArray(value.by_provider) ? value.by_provider : [],
    },
    { key: 'by_model', label: st('By model'), rows: Array.isArray(value.by_model) ? value.by_model : [] },
    {
      key: 'by_session',
      label: st('By session'),
      rows: Array.isArray(value.by_session) ? value.by_session : [],
    },
  ].filter((section) => section.rows.length > 0) as UsageSection[]
})

const activeBreakdown = computed(
  () => sections.value.find((section) => section.key === selectedBreakdown.value) || sections.value[0],
)

function finiteNumber(value: unknown): number {
  const number = Number(value)
  return Number.isFinite(number) ? number : 0
}

const integerFormatter = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 })
const percentFormatter = new Intl.NumberFormat(undefined, {
  style: 'percent',
  maximumFractionDigits: 1,
})
const usdFormatter = new Intl.NumberFormat(undefined, {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 6,
})

function formatInteger(value: unknown): string {
  return integerFormatter.format(finiteNumber(value))
}

function formatUsd(value: unknown): string {
  return usdFormatter.format(finiteNumber(value))
}

/** Cache hit rate arrives as a 0–1 ratio; render it as a percentage. */
function formatPercent(value: unknown): string {
  const ratio = Number(value)
  if (!Number.isFinite(ratio) || ratio <= 0) return '—'
  return percentFormatter.format(Math.min(1, ratio))
}

const cacheMetrics = computed(() => {
  const totals = stats.value?.totals
  if (!totals) return []
  return [
    { key: 'hit-rate', label: st('Cache hit rate'), value: formatPercent(totals.cache_hit_rate) },
    { key: 'cache-input', label: st('Cacheable input'), value: formatInteger(totals.cache_input_tokens) },
    { key: 'cache-read', label: st('Cache read'), value: formatInteger(totals.cache_read_tokens) },
    { key: 'cache-write', label: st('Cache write'), value: formatInteger(totals.cache_write_tokens) },
  ]
})

const performanceMetrics = computed(() => {
  const performance = stats.value?.totals.performance
  return [
    {
      key: 'generation',
      label: st('Generation speed'),
      value: formatTokenRate(performance?.tokens_per_second),
      samples: performance?.generation_count,
      hint: st('Output and reasoning tokens divided by the time between the first and last output chunks.'),
    },
    {
      key: 'ttft',
      label: st('Average first output'),
      value: formatDurationMs(performance?.average_ttft_ms),
      samples: performance?.ttft_count,
      hint: st('Time to the first non-empty text, reasoning, or tool-call output.'),
    },
    {
      key: 'duration',
      label: st('Average request duration'),
      value: formatDurationMs(performance?.average_duration_ms),
      samples: performance?.request_count,
      hint: st('Time from sending the model request to its completion.'),
    },
    {
      key: 'throughput',
      label: st('End-to-end speed'),
      value: formatTokenRate(performance?.end_to_end_tokens_per_second),
      samples: performance?.throughput_count,
      hint: st('Output and reasoning tokens divided by the complete model request duration.'),
    },
    {
      key: 'p95-ttft',
      label: st('P95 first output'),
      value: formatDurationMs(performance?.p95_ttft_ms),
      samples: performance?.ttft_count,
      hint: st('95% of measured requests finish this stage within this time.'),
    },
    {
      key: 'p95-duration',
      label: st('P95 request duration'),
      value: formatDurationMs(performance?.p95_duration_ms),
      samples: performance?.request_count,
      hint: st('95% of measured requests finish this stage within this time.'),
    },
  ]
})

function rowLabel(section: UsageSection['key'], row: UsageBreakdown): string {
  if (section === 'by_day') return String(row.date || '')
  if (section === 'by_provider') return String(row.provider_id || '')
  if (section === 'by_model') return [row.provider_id, row.model_id].filter(Boolean).join(' / ')
  return String(row.title || `Session ${row.session_id ?? ''}`).trim()
}

const { loading, error, refresh } = usePaneRead(
  (signal) => {
    const timezoneOffsetMinutes = -new Date().getTimezoneOffset()
    return apiJson<UsageStats>(`/api/v1/usage?period=last_30_days&timezone_offset_minutes=${timezoneOffsetMinutes}`, {
      signal,
    })
  },
  (data) => {
    stats.value = data && typeof data === 'object' ? data : null
  },
)
defineExpose({ refresh })
</script>

<template>
  <div class="settings-stack">
    <SettingsCard
      :title="$st('Usage overview')"
      :icon="RiBarChartBoxLine"
      :description="
        stats?.period_label ? `${stats.period_label} · ${stats.active_days} ${$st('active days')}` : undefined
      "
    >
      <template #actions
        ><IconButton
          variant="outline"
          size="sm"
          :tooltip="$st('Refresh usage')"
          :aria-label="$st('Refresh usage')"
          :disabled="loading"
          @click="refresh"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" /></IconButton
      ></template>
      <p
        v-if="error"
        role="alert"
        class="mb-3 break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
      >
        {{ error }}
      </p>
      <p v-if="loading && !stats" class="py-6 text-center text-sm text-muted-foreground">
        {{ $st('Loading usage...') }}
      </p>
      <p v-else-if="!stats" class="py-6 text-center text-sm text-muted-foreground">
        {{ $st('No usage data available.') }}
      </p>
      <dl v-else class="settings-control-grid">
        <div class="min-w-0 rounded-md border border-border/50 bg-muted/20 p-2.5">
          <dt class="text-xs text-muted-foreground">{{ $st('Requests') }}</dt>
          <dd class="mt-1 break-all font-mono text-lg font-semibold tabular-nums">
            {{ formatInteger(stats.totals.requests) }}
          </dd>
        </div>
        <div class="min-w-0 rounded-md border border-border/50 bg-muted/20 p-2.5">
          <dt class="text-xs text-muted-foreground">{{ $st('Sessions') }}</dt>
          <dd class="mt-1 break-all font-mono text-lg font-semibold tabular-nums">
            {{ formatInteger(stats.totals.sessions) }}
          </dd>
        </div>
        <div class="min-w-0 rounded-md border border-border/50 bg-muted/20 p-2.5">
          <dt class="text-xs text-muted-foreground">{{ $st('Tokens') }}</dt>
          <dd class="mt-1 break-all font-mono text-lg font-semibold tabular-nums">
            {{ formatInteger(stats.totals.total_tokens) }}
          </dd>
        </div>
        <div class="min-w-0 rounded-md border border-primary/20 bg-primary/5 p-2.5">
          <dt class="text-xs text-muted-foreground">{{ $st('Cost') }}</dt>
          <dd class="mt-1 break-all font-mono text-lg font-semibold tabular-nums">
            {{ formatUsd(stats.totals.total_cost_usd) }}
          </dd>
        </div>
      </dl>
    </SettingsCard>
    <template v-if="stats">
      <SettingsCard
        :title="$st('Model performance')"
        :icon="RiSpeedUpLine"
        :description="
          $st('Completed requests with timing: {count}', {
            count: formatInteger(stats.totals.performance?.request_count),
          })
        "
      >
        <dl class="settings-control-grid">
          <div
            v-for="metric in performanceMetrics"
            :key="metric.key"
            class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3"
          >
            <dt class="text-xs text-muted-foreground" :title="metric.hint">{{ metric.label }}</dt>
            <dd class="mt-2 break-words font-mono text-lg font-semibold tabular-nums">{{ metric.value }}</dd>
            <dd class="mt-1 text-[11px] text-muted-foreground">
              {{ $st('Samples: {count}', { count: formatInteger(metric.samples) }) }}
            </dd>
          </div>
        </dl>
        <p class="mt-3 text-xs leading-relaxed text-muted-foreground">
          {{
            stats.totals.performance?.request_count
              ? $st(
                  'Only completed requests with the required measurements contribute. Speeds use total output and reasoning tokens divided by total measured time; local tool execution is excluded.',
                )
              : $st('No performance samples yet. Completed model requests with timing will appear here.')
          }}
        </p>
      </SettingsCard>
      <div class="settings-columns">
        <SettingsCard :title="$st('Token details')" :icon="RiTokenSwapLine">
          <dl class="divide-y divide-border/50 text-xs">
            <div class="flex flex-wrap justify-between gap-2 pb-3">
              <dt class="text-muted-foreground">{{ $st('Input / output tokens') }}</dt>
              <dd class="break-all font-mono tabular-nums">
                {{ formatInteger(stats.totals.input_tokens) }} / {{ formatInteger(stats.totals.output_tokens) }}
              </dd>
            </div>
            <div class="flex flex-wrap justify-between gap-2 py-3">
              <dt class="text-muted-foreground">{{ $st('Reasoning / cache read') }}</dt>
              <dd class="break-all font-mono tabular-nums">
                {{ formatInteger(stats.totals.reasoning_tokens) }} / {{ formatInteger(stats.totals.cache_read_tokens) }}
              </dd>
            </div>
            <div class="flex flex-wrap justify-between gap-2 pt-3">
              <dt class="text-muted-foreground">{{ $st('Average per request') }}</dt>
              <dd class="break-all font-mono tabular-nums">
                {{ formatInteger(stats.average_tokens_per_request) }} {{ $st('tokens') }}
              </dd>
            </div>
            <div v-for="row in cacheMetrics" :key="row.key" class="flex flex-wrap justify-between gap-2 pt-3">
              <dt class="text-muted-foreground">{{ row.label }}</dt>
              <dd class="break-all font-mono tabular-nums">{{ row.value }}</dd>
            </div>
          </dl>
        </SettingsCard>
        <SettingsCard :title="$st('Cost details')" :icon="RiCoinsLine">
          <dl class="divide-y divide-border/50 text-xs">
            <div class="flex flex-wrap justify-between gap-2 pb-3">
              <dt class="text-muted-foreground">{{ $st('Recorded cost') }}</dt>
              <dd class="break-all font-mono tabular-nums">{{ formatUsd(stats.totals.recorded_cost_usd) }}</dd>
            </div>
            <div class="flex flex-wrap justify-between gap-2 py-3">
              <dt class="text-muted-foreground">{{ $st('Estimated cost') }}</dt>
              <dd class="break-all font-mono tabular-nums">{{ formatUsd(stats.totals.estimated_cost_usd) }}</dd>
            </div>
            <div class="flex flex-wrap justify-between gap-2 pt-3">
              <dt class="text-muted-foreground">{{ $st('Average per request') }}</dt>
              <dd class="break-all font-mono tabular-nums">{{ formatUsd(stats.average_cost_per_request_usd) }}</dd>
            </div>
          </dl>
          <p
            v-if="stats.totals.unpriced_requests"
            class="mt-3 rounded-lg bg-amber-500/10 px-3 py-2 text-xs leading-relaxed text-amber-800 dark:text-amber-200"
          >
            {{
              $st('{count} requests have incomplete pricing; the total may be understated.', {
                count: formatInteger(stats.totals.unpriced_requests),
              })
            }}
          </p>
        </SettingsCard>
      </div>
      <SettingsCard :title="$st('Usage breakdown')" :count="activeBreakdown?.rows.length">
        <p v-if="!sections.length" class="rounded-lg bg-muted/20 p-5 text-center text-sm text-muted-foreground">
          {{ $st('No usage breakdown is available.') }}
        </p>
        <template v-else-if="activeBreakdown">
          <div
            class="settings-toolbar mb-2.5 rounded-md border border-border/50 bg-muted/20 p-0.5"
            :aria-label="$st('Usage breakdown')"
          >
            <button
              v-for="section in sections"
              :key="section.key"
              type="button"
              class="min-w-0 rounded-md px-3 py-2 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring flex-1 basis-auto"
              :class="
                activeBreakdown.key === section.key
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground'
              "
              :aria-pressed="activeBreakdown.key === section.key"
              @click="selectedBreakdown = section.key"
            >
              {{ section.label }}
            </button>
          </div>
          <div
            class="usage-breakdown-heading rounded-md bg-muted/20 px-2.5 py-2 text-xs text-muted-foreground"
            aria-hidden="true"
          >
            <span>{{ $st('Name') }}</span
            ><span class="text-right">{{ $st('Requests') }}</span
            ><span class="text-right">{{ $st('Tokens') }}</span
            ><span class="text-right">{{ $st('Cost') }}</span
            ><span class="text-right">{{ $st('Generation speed') }}</span
            ><span class="text-right">{{ $st('Average first output') }}</span
            ><span class="text-right">{{ $st('Average request duration') }}</span>
          </div>
          <dl class="divide-y divide-border/50">
            <div
              v-for="row in activeBreakdown.rows"
              :key="rowLabel(activeBreakdown.key, row)"
              class="usage-breakdown-row px-1 py-2 text-xs"
            >
              <dt class="usage-breakdown-name break-all font-mono font-medium">
                {{ rowLabel(activeBreakdown.key, row) }}
              </dt>
              <dd class="usage-breakdown-value min-w-0">
                <span class="usage-breakdown-label mb-1 block text-[10px] text-muted-foreground">{{ $st('Requests') }}</span
                ><span class="break-all font-mono tabular-nums">{{ formatInteger(row.requests) }}</span>
              </dd>
              <dd class="usage-breakdown-value min-w-0">
                <span class="usage-breakdown-label mb-1 block text-[10px] text-muted-foreground">{{ $st('Tokens') }}</span
                ><span class="break-all font-mono tabular-nums">{{ formatInteger(row.total_tokens) }}</span>
              </dd>
              <dd class="usage-breakdown-value min-w-0">
                <span class="usage-breakdown-label mb-1 block text-[10px] text-muted-foreground">{{ $st('Cost') }}</span
                ><span class="break-all font-mono tabular-nums">{{ formatUsd(row.total_cost_usd) }}</span>
              </dd>
              <dd class="usage-breakdown-value min-w-0">
                <span class="usage-breakdown-label mb-1 block text-[10px] text-muted-foreground">{{
                  $st('Generation speed')
                }}</span>
                <span class="break-words font-mono tabular-nums">{{
                  formatTokenRate(row.performance?.tokens_per_second)
                }}</span>
                <span class="mt-1 block text-[10px] text-muted-foreground">{{
                  $st('Samples: {count}', { count: formatInteger(row.performance?.generation_count) })
                }}</span>
              </dd>
              <dd class="usage-breakdown-value min-w-0">
                <span class="usage-breakdown-label mb-1 block text-[10px] text-muted-foreground">{{
                  $st('Average first output')
                }}</span>
                <span class="break-words font-mono tabular-nums">{{
                  formatDurationMs(row.performance?.average_ttft_ms)
                }}</span>
                <span class="mt-1 block text-[10px] text-muted-foreground">{{
                  $st('Samples: {count}', { count: formatInteger(row.performance?.ttft_count) })
                }}</span>
              </dd>
              <dd class="usage-breakdown-value min-w-0">
                <span class="usage-breakdown-label mb-1 block text-[10px] text-muted-foreground">{{
                  $st('Average request duration')
                }}</span>
                <span class="break-words font-mono tabular-nums">{{
                  formatDurationMs(row.performance?.average_duration_ms)
                }}</span>
                <span class="mt-1 block text-[10px] text-muted-foreground">{{
                  $st('Samples: {count}', { count: formatInteger(row.performance?.request_count) })
                }}</span>
              </dd>
            </div>
          </dl>
        </template>
      </SettingsCard>
    </template>
  </div>
</template>

<style scoped>
@reference '../../style.css';

.usage-breakdown-heading {
  display: none;
}

.usage-breakdown-row {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0.5rem;
}

.usage-breakdown-name {
  grid-column: 1 / -1;
}

@container settings-card (min-width: 56rem) {
  .usage-breakdown-heading,
  .usage-breakdown-row {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) repeat(6, minmax(0, 0.7fr));
    align-items: center;
    gap: 0.625rem;
    padding-inline: 0.625rem;
  }

  .usage-breakdown-name {
    grid-column: 1;
  }

  .usage-breakdown-value {
    text-align: end;
  }

  .usage-breakdown-label {
    @apply sr-only;
  }
}
</style>
