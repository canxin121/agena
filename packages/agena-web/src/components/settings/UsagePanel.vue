<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiRefreshLine, RiBarChartBoxLine, RiCoinsLine, RiTokenSwapLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneRead } from '@/composables/usePaneRead'

type UsageTotals = {
  requests: number
  sessions: number
  input_tokens: number
  output_tokens: number
  reasoning_tokens: number
  cache_read_tokens: number
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
  <div class="min-w-0 space-y-4">
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
      <dl v-else class="grid grid-cols-2 gap-3 xl:grid-cols-4">
        <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3 sm:p-4">
          <dt class="text-xs text-muted-foreground">{{ $st('Requests') }}</dt>
          <dd class="mt-2 break-all font-mono text-xl font-semibold tabular-nums">
            {{ formatInteger(stats.totals.requests) }}
          </dd>
        </div>
        <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3 sm:p-4">
          <dt class="text-xs text-muted-foreground">{{ $st('Sessions') }}</dt>
          <dd class="mt-2 break-all font-mono text-xl font-semibold tabular-nums">
            {{ formatInteger(stats.totals.sessions) }}
          </dd>
        </div>
        <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3 sm:p-4">
          <dt class="text-xs text-muted-foreground">{{ $st('Tokens') }}</dt>
          <dd class="mt-2 break-all font-mono text-xl font-semibold tabular-nums">
            {{ formatInteger(stats.totals.total_tokens) }}
          </dd>
        </div>
        <div class="min-w-0 rounded-lg border border-primary/20 bg-primary/5 p-3 sm:p-4">
          <dt class="text-xs text-muted-foreground">{{ $st('Cost') }}</dt>
          <dd class="mt-2 break-all font-mono text-xl font-semibold tabular-nums">
            {{ formatUsd(stats.totals.total_cost_usd) }}
          </dd>
        </div>
      </dl>
    </SettingsCard>
    <template v-if="stats">
      <div class="grid gap-4 xl:grid-cols-2">
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
            class="mb-4 grid grid-cols-2 gap-1 rounded-lg border border-border/50 bg-muted/20 p-1 sm:flex sm:flex-wrap"
            :aria-label="$st('Usage breakdown')"
          >
            <button
              v-for="section in sections"
              :key="section.key"
              type="button"
              class="min-w-0 rounded-md px-3 py-2 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:flex-1"
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
            class="hidden grid-cols-[minmax(0,1fr)_minmax(4rem,.35fr)_minmax(5rem,.4fr)_minmax(5rem,.4fr)] gap-3 rounded-lg bg-muted/20 px-3 py-2.5 text-xs text-muted-foreground sm:grid"
            aria-hidden="true"
          >
            <span>{{ $st('Name') }}</span
            ><span class="text-right">{{ $st('Requests') }}</span
            ><span class="text-right">{{ $st('Tokens') }}</span
            ><span class="text-right">{{ $st('Cost') }}</span>
          </div>
          <dl class="divide-y divide-border/50">
            <div
              v-for="row in activeBreakdown.rows"
              :key="rowLabel(activeBreakdown.key, row)"
              class="grid min-w-0 grid-cols-3 gap-x-3 gap-y-2 px-1 py-3 text-xs sm:grid-cols-[minmax(0,1fr)_minmax(4rem,.35fr)_minmax(5rem,.4fr)_minmax(5rem,.4fr)] sm:items-center sm:px-3"
            >
              <dt class="col-span-3 break-all font-mono font-medium sm:col-span-1">
                {{ rowLabel(activeBreakdown.key, row) }}
              </dt>
              <dd class="min-w-0 sm:text-right">
                <span class="mb-1 block text-[10px] text-muted-foreground sm:sr-only">{{ $st('Requests') }}</span
                ><span class="break-all font-mono tabular-nums">{{ formatInteger(row.requests) }}</span>
              </dd>
              <dd class="min-w-0 sm:text-right">
                <span class="mb-1 block text-[10px] text-muted-foreground sm:sr-only">{{ $st('Tokens') }}</span
                ><span class="break-all font-mono tabular-nums">{{ formatInteger(row.total_tokens) }}</span>
              </dd>
              <dd class="min-w-0 sm:text-right">
                <span class="mb-1 block text-[10px] text-muted-foreground sm:sr-only">{{ $st('Cost') }}</span
                ><span class="break-all font-mono tabular-nums">{{ formatUsd(row.total_cost_usd) }}</span>
              </dd>
            </div>
          </dl>
        </template>
      </SettingsCard>
    </template>
  </div>
</template>
