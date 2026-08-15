<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '../../lib/api'

type UsagePayload = Record<string, unknown>

const BY_SECTIONS: Array<{ key: string; label: string }> = [
  { key: 'by_day', label: 'By day' },
  { key: 'by_provider', label: 'By provider' },
  { key: 'by_model', label: 'By model' },
  { key: 'by_session', label: 'By session' },
]

const loading = ref(false)
const error = ref('')
const payload = ref<UsagePayload | null>(null)

const period = computed(() => {
  const value = payload.value?.period
  if (typeof value === 'string' || typeof value === 'number') return String(value)
  return ''
})

type Section = { key: string; label: string; rows: Array<{ key: string; value: string }> }

const sections = computed<Section[]>(() => {
  const source = payload.value
  if (!source) return []
  const out: Section[] = []
  for (const { key, label } of BY_SECTIONS) {
    const value = source[key]
    if (!value || typeof value !== 'object' || Array.isArray(value)) continue
    const rows = Object.entries(value as Record<string, unknown>)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([rowKey, rowValue]) => ({ key: rowKey, value: scalarString(rowValue) }))
    if (rows.length > 0) {
      out.push({ key, label, rows })
    }
  }
  return out
})

function scalarString(value: unknown): string {
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') return String(value)
  if (value === null || value === undefined) return ''
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

const hasData = computed(() => sections.value.length > 0)

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const data = await apiJson<UsagePayload>('/api/v1/usage')
    payload.value = data && typeof data === 'object' && !Array.isArray(data) ? data : null
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    payload.value = null
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
    <div>
      <div class="text-lg font-medium">Usage</div>
      <div class="mt-1 text-sm text-muted-foreground">Server-side usage counts.</div>
      <div v-if="period" class="mt-1 text-[11px] font-mono text-muted-foreground">period: {{ period }}</div>
    </div>

    <div class="grid gap-3">
      <div v-if="loading" class="text-sm text-muted-foreground">Loading usage...</div>
      <div v-else-if="error" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {{ error }}
      </div>
      <div v-else-if="!hasData" class="text-sm text-muted-foreground">No usage data available.</div>

      <div v-else class="space-y-4">
        <div v-for="section in sections" :key="section.key" class="rounded-md border border-border/60 bg-background/50">
          <div class="border-b border-border/60 px-3 py-2 text-xs font-semibold text-muted-foreground">
            {{ section.label }}
          </div>
          <div class="overflow-x-auto">
            <table class="min-w-full text-sm">
              <tbody>
                <tr v-for="row in section.rows" :key="row.key" class="border-t border-border/50 first:border-t-0">
                  <td class="px-3 py-1.5 font-mono text-xs text-foreground/80 break-all align-top">{{ row.key }}</td>
                  <td class="px-3 py-1.5 text-right font-mono text-xs text-foreground whitespace-nowrap">{{ row.value }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? 'Refreshing...' : 'Refresh'"
        :aria-label="loading ? 'Refreshing...' : 'Refresh'"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
      <Button variant="outline" size="sm" :disabled="loading" @click="refresh">
        {{ loading ? 'Refreshing...' : 'Refresh' }}
      </Button>
    </div>
  </div>
</template>
