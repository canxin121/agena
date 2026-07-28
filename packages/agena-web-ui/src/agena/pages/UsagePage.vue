<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { fetchUsageStats, type UsagePeriod, type UsageStats, type UsageTotals } from '@/agena/lib/agenaApi'
import {
  formatUsageCost,
  formatUsageInteger,
  formatUsagePercent,
  hasUsage,
  isUsagePeriod,
  usageFactLine,
  usageHeadline,
  usagePeriodOptions,
} from './usageStatsModel'

const activePeriod = ref<UsagePeriod>('last_7_days')
const providerFilter = ref('')
const modelFilter = ref('')
const includeSubagents = ref(true)
const sortBy = ref<'cost' | 'tokens' | 'runs'>('cost')
const loading = ref(false)
const error = ref('')
const stats = ref<UsageStats | null>(null)

const headline = computed(() => usageHeadline(stats.value))
const facts = computed(() => (stats.value ? usageFactLine(stats.value.totals) : []))
const route = useRoute()
const router = useRouter()

function sortUsageRows<T extends UsageTotals>(rows: T[]): T[] {
  const field = sortBy.value === 'tokens' ? 'total_tokens' : sortBy.value === 'runs' ? 'runs' : 'total_cost_usd'
  return [...rows].sort((left, right) => Number(right[field]) - Number(left[field]))
}

const topProviders = computed(() => sortUsageRows(stats.value?.by_provider || []).slice(0, 10))
const topModels = computed(() => sortUsageRows(stats.value?.by_model || []).slice(0, 10))
const topSessions = computed(() => sortUsageRows(stats.value?.by_session || []).slice(0, 10))
const dailyRows = computed(() => stats.value?.by_day.slice(-14) || [])

async function loadUsage(period = activePeriod.value) {
  activePeriod.value = period
  loading.value = true
  error.value = ''
  try {
    stats.value = await fetchUsageStats({
      period,
      providerIds: providerFilter.value.trim() ? [providerFilter.value.trim()] : [],
      modelIds: modelFilter.value.trim() ? [modelFilter.value.trim()] : [],
      includeSubagents: includeSubagents.value,
      timezoneOffsetMinutes: -new Date().getTimezoneOffset(),
    })
    const query: Record<string, string> = { period }
    if (providerFilter.value.trim()) query.provider = providerFilter.value.trim()
    if (modelFilter.value.trim()) query.model = modelFilter.value.trim()
    if (!includeSubagents.value) query.include_subagents = 'false'
    await router.replace({ path: '/usage', query })
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

function tokenSummary(totals: UsageTotals): string {
  return [
    `in ${formatUsageInteger(totals.input_tokens)}`,
    `out ${formatUsageInteger(totals.output_tokens)}`,
    `reasoning ${formatUsageInteger(totals.reasoning_tokens)}`,
    `cache read ${formatUsageInteger(totals.cache_read_tokens)}`,
    `cache write ${formatUsageInteger(totals.cache_write_tokens)} (${formatUsageInteger(totals.cache_write_5m_tokens)} 5m / ${formatUsageInteger(totals.cache_write_1h_tokens)} 1h)`,
    `tool ${formatUsageInteger(totals.tool_use_tokens)}`,
    `other ${formatUsageInteger(totals.other_tokens)}`,
  ].join(' · ')
}

onMounted(() => {
  const routePeriod = typeof route.query.period === 'string' ? route.query.period : ''
  if (isUsagePeriod(routePeriod)) activePeriod.value = routePeriod
  providerFilter.value = typeof route.query.provider === 'string' ? route.query.provider.trim() : ''
  modelFilter.value = typeof route.query.model === 'string' ? route.query.model.trim() : ''
  includeSubagents.value = route.query.include_subagents !== 'false'
  void loadUsage()
})
</script>

<template>
  <div class="page">
    <section class="page-header">
      <div>
        <h1>Usage</h1>
        <p class="muted">{{ headline }}</p>
      </div>
      <div class="button-row">
        <button
          v-for="option in usagePeriodOptions"
          :key="option.id"
          class="button"
          :class="{ primary: activePeriod === option.id, ghost: activePeriod !== option.id }"
          :disabled="loading"
          @click="loadUsage(option.id)"
        >
          {{ option.label }}
        </button>
      </div>
    </section>

    <div v-if="error" class="notice">{{ error }}</div>

    <section class="card">
      <div class="page-header" style="align-items: flex-end">
        <div class="form-grid" style="flex: 1">
          <div class="field">
            <label class="label" for="usage-provider-filter">Provider filter</label>
            <input id="usage-provider-filter" v-model="providerFilter" class="input mono" placeholder="all providers" />
          </div>
          <div class="field">
            <label class="label" for="usage-model-filter">Model filter</label>
            <input id="usage-model-filter" v-model="modelFilter" class="input mono" placeholder="all models" />
          </div>
          <div class="field">
            <label class="label" for="usage-sort">Sort breakdowns</label>
            <select id="usage-sort" v-model="sortBy" class="select">
              <option value="cost">Cost</option>
              <option value="tokens">Tokens</option>
              <option value="runs">Requests</option>
            </select>
          </div>
          <label class="field checkbox-field">
            <input v-model="includeSubagents" type="checkbox" />
            <span>Include subagent sessions</span>
          </label>
        </div>
        <button class="button primary" :disabled="loading" @click="loadUsage()">Apply Filters</button>
      </div>
    </section>

    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3 style="margin: 0">Overview</h3>
          <p class="muted mono">{{ facts.join(' · ') || 'No recorded provider requests in this period.' }}</p>
        </div>
        <button class="button ghost" :disabled="loading" @click="loadUsage()">Refresh</button>
      </div>
      <div class="grid four">
        <div class="field">
          <label class="label">Effective Cost</label>
          <div class="muted mono">{{ formatUsageCost(stats?.totals.total_cost_usd || 0) }}</div>
          <div class="muted mono">
            recorded {{ formatUsageCost(stats?.totals.recorded_cost_usd || 0) }} · estimated
            {{ formatUsageCost(stats?.totals.estimated_cost_usd || 0) }}
          </div>
        </div>
        <div class="field">
          <label class="label">Provider Requests</label>
          <div class="muted mono">{{ formatUsageInteger(stats?.totals.runs || 0) }}</div>
        </div>
        <div class="field">
          <label class="label">Sessions</label>
          <div class="muted mono">{{ formatUsageInteger(stats?.totals.sessions || 0) }}</div>
        </div>
        <div class="field">
          <label class="label">Cache Hit Rate</label>
          <div class="muted mono">{{ formatUsagePercent(stats?.totals.cache_hit_rate || 0) }}</div>
        </div>
        <div class="field">
          <label class="label">Input Tokens</label>
          <div class="muted mono">{{ formatUsageInteger(stats?.totals.input_tokens || 0) }}</div>
        </div>
        <div class="field">
          <label class="label">Output Tokens</label>
          <div class="muted mono">{{ formatUsageInteger(stats?.totals.output_tokens || 0) }}</div>
        </div>
        <div class="field">
          <label class="label">Reasoning Tokens</label>
          <div class="muted mono">{{ formatUsageInteger(stats?.totals.reasoning_tokens || 0) }}</div>
        </div>
        <div class="field">
          <label class="label">Cache Read / Write</label>
          <div class="muted mono">
            {{ formatUsageInteger(stats?.totals.cache_read_tokens || 0) }} /
            {{ formatUsageInteger(stats?.totals.cache_write_tokens || 0) }}
          </div>
        </div>
      </div>
    </section>

    <section class="grid two">
      <div class="card">
        <h3 style="margin-top: 0">By Provider</h3>
        <div v-if="topProviders.length" class="list">
          <div v-for="provider in topProviders" :key="provider.provider_id" class="list-item">
            <div>
              <strong>{{ provider.provider_id }}</strong>
              <div class="muted mono">{{ tokenSummary(provider) }}</div>
            </div>
            <div class="stack" style="justify-items: end">
              <span class="badge">{{ formatUsageCost(provider.total_cost_usd) }}</span>
              <span class="muted mono">{{ formatUsageInteger(provider.runs) }} requests</span>
            </div>
          </div>
        </div>
        <p v-else class="muted">No provider usage for this period.</p>
      </div>

      <div class="card">
        <h3 style="margin-top: 0">By Model</h3>
        <div v-if="topModels.length" class="list">
          <div v-for="model in topModels" :key="`${model.provider_id}/${model.model_id}`" class="list-item">
            <div>
              <strong>{{ model.provider_id }}/{{ model.model_id }}</strong>
              <div class="muted mono">{{ tokenSummary(model) }}</div>
            </div>
            <div class="stack" style="justify-items: end">
              <span class="badge">{{ formatUsageCost(model.total_cost_usd) }}</span>
              <span class="muted mono">{{ formatUsagePercent(model.cache_hit_rate) }} cache</span>
            </div>
          </div>
        </div>
        <p v-else class="muted">No model usage for this period.</p>
      </div>

      <div class="card">
        <h3 style="margin-top: 0">By Day</h3>
        <div v-if="dailyRows.length" class="list">
          <div v-for="day in dailyRows" :key="day.date" class="list-item">
            <div>
              <strong>{{ day.date }}</strong>
              <div class="muted mono">
                requests {{ formatUsageInteger(day.runs) }} · sessions {{ formatUsageInteger(day.sessions) }}
              </div>
            </div>
            <div class="stack" style="justify-items: end">
              <span class="badge">{{ formatUsageCost(day.total_cost_usd) }}</span>
              <span class="muted mono">{{ formatUsageInteger(day.total_tokens) }} tokens</span>
            </div>
          </div>
        </div>
        <p v-else class="muted">No daily usage for this period.</p>
      </div>
    </section>

    <section v-if="stats?.totals.billable_units?.length" class="card">
      <h3 style="margin-top: 0">Non-token Billable Units</h3>
      <div class="list">
        <div v-for="item in stats.totals.billable_units" :key="`${item.kind}/${item.unit}`" class="list-item">
          <div>
            <strong>{{ item.kind }}</strong>
            <div class="muted mono">{{ formatUsageInteger(item.quantity) }} {{ item.unit }}</div>
          </div>
          <div class="stack" style="justify-items: end">
            <span class="badge">{{ formatUsageCost(item.estimated_cost_usd) }}</span>
            <span v-if="item.unpriced_quantity > 0" class="muted mono"
              >{{ formatUsageInteger(item.unpriced_quantity) }} unpriced</span
            >
          </div>
        </div>
      </div>
    </section>

    <section class="card">
      <h3 style="margin-top: 0">Top Sessions</h3>
      <div v-if="topSessions.length" class="list">
        <div v-for="session in topSessions" :key="session.session_id" class="list-item">
          <div>
            <strong>#{{ session.session_id }} · {{ session.title }}</strong>
            <div class="muted mono">
              requests {{ formatUsageInteger(session.runs) }} · {{ tokenSummary(session) }}
              <span v-if="session.is_subagent"> · subagent</span>
            </div>
          </div>
          <div class="stack" style="justify-items: end">
            <span class="badge">{{ formatUsageCost(session.total_cost_usd) }}</span>
            <span class="muted mono">{{ formatUsagePercent(session.cache_hit_rate) }} cache</span>
          </div>
        </div>
      </div>
      <p v-else-if="!hasUsage(stats)" class="muted">No session usage for this period.</p>
    </section>
  </div>
</template>
