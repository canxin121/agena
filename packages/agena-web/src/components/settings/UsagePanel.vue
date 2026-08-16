<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'

import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'

type UsageTotals = {
  runs: number
  sessions: number
  input_tokens: number
  output_tokens: number
  reasoning_tokens: number
  cache_read_tokens: number
  total_tokens: number
  total_cost_usd: number
  recorded_cost_usd: number
  estimated_cost_usd: number
  unpriced_runs: number
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
  average_cost_per_run_usd: number
  average_tokens_per_run: number
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

const loading = ref(false)
const error = ref('')
const stats = ref<UsageStats | null>(null)

const sections = computed<UsageSection[]>(() => {
  const value = stats.value
  if (!value) return []
  return [
    { key: 'by_day', label: 'By day', rows: Array.isArray(value.by_day) ? value.by_day : [] },
    {
      key: 'by_provider',
      label: 'By provider',
      rows: Array.isArray(value.by_provider) ? value.by_provider : [],
    },
    { key: 'by_model', label: 'By model', rows: Array.isArray(value.by_model) ? value.by_model : [] },
    {
      key: 'by_session',
      label: 'By session',
      rows: Array.isArray(value.by_session) ? value.by_session : [],
    },
  ].filter((section) => section.rows.length > 0) as UsageSection[]
})

function finiteNumber(value: unknown): number {
  const number = Number(value)
  return Number.isFinite(number) ? number : 0
}

function formatInteger(value: unknown): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(finiteNumber(value))
}

function formatUsd(value: unknown): string {
  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 6,
  }).format(finiteNumber(value))
}

function rowLabel(section: UsageSection['key'], row: UsageBreakdown): string {
  if (section === 'by_day') return String(row.date || '')
  if (section === 'by_provider') return String(row.provider_id || '')
  if (section === 'by_model') return [row.provider_id, row.model_id].filter(Boolean).join(' / ')
  return String(row.title || `Session ${row.session_id ?? ''}`).trim()
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const timezoneOffsetMinutes = -new Date().getTimezoneOffset()
    const data = await apiJson<UsageStats>(
      `/api/v1/usage?period=last_30_days&timezone_offset_minutes=${timezoneOffsetMinutes}`,
    )
    stats.value = data && typeof data === 'object' ? data : null
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    stats.value = null
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">Usage</div>
        <div class="mt-1 text-sm text-muted-foreground">Provider usage recorded by the Agena server.</div>
        <div v-if="stats?.period_label" class="mt-1 text-[11px] text-muted-foreground">
          {{ stats.period_label }} · {{ stats.active_days }} active days
        </div>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? 'Refreshing usage' : 'Refresh usage'"
        :aria-label="loading ? 'Refreshing usage' : 'Refresh usage'"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div v-if="loading" class="text-sm text-muted-foreground">Loading usage...</div>
    <div
      v-else-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>
    <div v-else-if="!stats" class="text-sm text-muted-foreground">No usage data available.</div>

    <template v-else>
      <dl class="grid grid-cols-2 gap-x-6 gap-y-4 border-y border-border/60 py-4 sm:grid-cols-4">
        <div>
          <dt class="text-xs text-muted-foreground">Requests</dt>
          <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">{{ formatInteger(stats.totals.runs) }}</dd>
        </div>
        <div>
          <dt class="text-xs text-muted-foreground">Sessions</dt>
          <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">{{ formatInteger(stats.totals.sessions) }}</dd>
        </div>
        <div>
          <dt class="text-xs text-muted-foreground">Tokens</dt>
          <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">
            {{ formatInteger(stats.totals.total_tokens) }}
          </dd>
        </div>
        <div>
          <dt class="text-xs text-muted-foreground">Cost</dt>
          <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">
            {{ formatUsd(stats.totals.total_cost_usd) }}
          </dd>
        </div>
      </dl>

      <div class="grid gap-4 sm:grid-cols-3">
        <div>
          <div class="text-xs text-muted-foreground">Input / output tokens</div>
          <div class="mt-1 font-mono text-sm">
            {{ formatInteger(stats.totals.input_tokens) }} / {{ formatInteger(stats.totals.output_tokens) }}
          </div>
        </div>
        <div>
          <div class="text-xs text-muted-foreground">Reasoning / cache read</div>
          <div class="mt-1 font-mono text-sm">
            {{ formatInteger(stats.totals.reasoning_tokens) }} / {{ formatInteger(stats.totals.cache_read_tokens) }}
          </div>
        </div>
        <div>
          <div class="text-xs text-muted-foreground">Average per request</div>
          <div class="mt-1 font-mono text-sm">
            {{ formatInteger(stats.average_tokens_per_run) }} · {{ formatUsd(stats.average_cost_per_run_usd) }}
          </div>
        </div>
      </div>

      <div v-if="sections.length === 0" class="text-sm text-muted-foreground">No usage breakdown is available.</div>
      <div v-else class="space-y-4">
        <section
          v-for="section in sections"
          :key="section.key"
          class="overflow-hidden rounded-md border border-border/60 bg-background/50"
        >
          <h3 class="border-b border-border/60 px-3 py-2 text-xs font-semibold text-muted-foreground">
            {{ section.label }}
          </h3>
          <div class="overflow-x-auto">
            <table class="min-w-full text-xs">
              <thead class="bg-muted/20 text-muted-foreground">
                <tr>
                  <th class="px-3 py-2 text-left font-medium">Name</th>
                  <th class="px-3 py-2 text-right font-medium">Requests</th>
                  <th class="px-3 py-2 text-right font-medium">Tokens</th>
                  <th class="px-3 py-2 text-right font-medium">Cost</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="row in section.rows" :key="rowLabel(section.key, row)" class="border-t border-border/50">
                  <td class="max-w-[22rem] break-words px-3 py-2 font-mono text-foreground/80">
                    {{ rowLabel(section.key, row) }}
                  </td>
                  <td class="whitespace-nowrap px-3 py-2 text-right font-mono tabular-nums">
                    {{ formatInteger(row.runs) }}
                  </td>
                  <td class="whitespace-nowrap px-3 py-2 text-right font-mono tabular-nums">
                    {{ formatInteger(row.total_tokens) }}
                  </td>
                  <td class="whitespace-nowrap px-3 py-2 text-right font-mono tabular-nums">
                    {{ formatUsd(row.total_cost_usd) }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>
    </template>
  </div>
</template>
