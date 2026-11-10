<script setup lang="ts">
// The composer's context chip opens this panel: the live context window state
// the chip already shows, plus the conversation's recorded token and cost
// totals from the server's cost projection.
import { computed, ref, watch } from 'vue'
import { RiPieChartLine } from '@remixicon/vue'

import SessionChip from './SessionChip.vue'
import { formatTokensK } from '@/pages/chat/contextUsage'
import { apiJson } from '@/lib/api'
import { settingsText as st } from '@/i18n/settingsText'

type ContextUsageView = {
  tokensValue: number | null
  tokensLabel: string
  percentUsed: number | null
  capacityLabel?: string
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
  total_tokens?: number
  total_cost_usd?: number
}

const props = defineProps<{
  sessionId: string
  usage: ContextUsageView
  open: boolean
  busy?: boolean
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

const percentFormatter = new Intl.NumberFormat(undefined, { style: 'percent', maximumFractionDigits: 1 })

/** The server reports a 0–1 ratio; show it as a percentage. */
function formatPercent(value: unknown): string {
  const ratio = Number(value)
  if (!Number.isFinite(ratio) || ratio <= 0) return '—'
  return percentFormatter.format(Math.min(1, ratio))
}

// The live snapshot describes the last request; the cost summary describes the
// whole conversation. Both are useful, so the cache line reports the live value
// first and the conversation average second.
const liveCacheRows = computed(() => {
  const usage = props.usage
  if (usage.lastCacheHitRate == null && usage.lastCacheReadTokens == null) return []
  return [
    { key: 'live-hit', label: st('Last request cache hit'), value: formatPercent(usage.lastCacheHitRate) },
    {
      key: 'live-read',
      label: st('Last request cache read'),
      value: formatTokensK(finite(usage.lastCacheReadTokens)),
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
  const total = finite(value.total_tokens) || input + output + reasoning + cacheRead + cacheWrite
  return [
    { key: 'requests', label: st('Requests'), value: String(finite(value.requests)) },
    { key: 'input', label: st('Input tokens'), value: formatTokensK(input) },
    { key: 'output', label: st('Output tokens'), value: formatTokensK(output) },
    { key: 'reasoning', label: st('Reasoning tokens'), value: formatTokensK(reasoning) },
    { key: 'cache-read', label: st('Cache read tokens'), value: formatTokensK(cacheRead) },
    { key: 'cache-write', label: st('Cache write tokens'), value: formatTokensK(cacheWrite) },
    { key: 'cache-hit', label: st('Conversation cache hit'), value: formatPercent(value.cache_hit_rate) },
    { key: 'cacheable', label: st('Cacheable input'), value: formatTokensK(finite(value.cache_input_tokens)) },
    { key: 'total', label: st('Total tokens'), value: formatTokensK(total) },
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
    summary.value = null
    if (props.open) void load()
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
    @update:open="emit('update:open', $event)"
  >
    <template #icon>
      <RiPieChartLine class="h-3 w-3" />
    </template>
    <div class="space-y-3 text-xs">
      <div class="space-y-2">
        <div class="flex items-center justify-between gap-3">
          <span class="text-muted-foreground">{{ $st('Current context') }}</span>
          <span class="font-mono tabular-nums">
            {{ usage.tokensValue !== null ? formatTokensK(usage.tokensValue) : '--' }}
          </span>
        </div>
        <div
          v-if="usage.percentUsed !== null"
          class="h-2 overflow-hidden rounded-full bg-muted"
          role="meter"
          :aria-valuenow="usage.percentUsed"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-label="$st('Context usage')"
        >
          <div
            class="h-full rounded-full transition-[width]"
            :class="usage.percentUsed >= 85 ? 'bg-destructive' : usage.percentUsed >= 60 ? 'bg-amber-500' : 'bg-primary'"
            :style="{ width: `${usage.percentUsed}%` }"
          />
        </div>
        <p v-if="usage.capacityLabel" class="text-muted-foreground">{{ usage.capacityLabel }}</p>
        <dl v-if="liveCacheRows.length" class="grid grid-cols-2 gap-x-3 gap-y-1 border-t border-border/50 pt-2">
          <template v-for="row in liveCacheRows" :key="row.key">
            <dt class="text-muted-foreground">{{ row.label }}</dt>
            <dd class="text-right font-mono tabular-nums">{{ row.value }}</dd>
          </template>
        </dl>
      </div>

      <div class="space-y-1.5 border-t border-border/50 pt-3">
        <p class="font-medium">{{ $st('Conversation totals') }}</p>
        <p v-if="error" role="alert" class="break-words text-destructive">{{ error }}</p>
        <p v-else-if="loading && !summary" class="text-muted-foreground">{{ $st('Loading…') }}</p>
        <dl v-else-if="rows.length" class="grid grid-cols-2 gap-x-3 gap-y-1">
          <template v-for="row in rows" :key="row.key">
            <dt class="text-muted-foreground">{{ row.label }}</dt>
            <dd class="text-right font-mono tabular-nums">{{ row.value }}</dd>
          </template>
        </dl>
        <p v-else class="text-muted-foreground">{{ $st('No recorded usage for this conversation yet.') }}</p>
        <div v-if="costLabel" class="flex items-center justify-between gap-3 border-t border-border/50 pt-2">
          <span class="text-muted-foreground">{{ $st('Estimated cost') }}</span>
          <span class="font-mono tabular-nums">{{ costLabel }}</span>
        </div>
      </div>
    </div>
  </SessionChip>
</template>
