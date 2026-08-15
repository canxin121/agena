<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiArrowDownSLine, RiArrowRightSLine, RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '../../lib/api'
import { useToastsStore } from '../../stores/toasts'

type ProviderRecord = {
  provider_id?: string
  defaults?: Record<string, unknown> | null
  adapters?: unknown[] | null
}

type ProviderModelsResponse = {
  provider_id?: string
  models?: unknown[]
}

const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const providers = ref<ProviderRecord[]>([])
const expandedId = ref<string | null>(null)
const expandedLoading = ref(false)
const expandedError = ref('')
const expandedModels = ref<unknown[]>([])

const sortedProviders = computed(() =>
  [...providers.value].sort((a, b) => String(a.provider_id || '').localeCompare(String(b.provider_id || ''))),
)

const expandedModelLabels = computed(() =>
  expandedModels.value.map((model) => {
    if (model && typeof model === 'object') {
      const record = model as Record<string, unknown>
      const id = record.id
      if (typeof id === 'string' && id.trim()) return id
    }
    try {
      return JSON.stringify(model)
    } catch {
      return String(model)
    }
  }),
)

function modelCount(provider: ProviderRecord): number {
  const defaults = provider.defaults
  if (defaults && Array.isArray(defaults.models)) {
    return defaults.models.length
  }
  if (Array.isArray(provider.adapters)) {
    return provider.adapters.length
  }
  return 0
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const data = await apiJson<ProviderRecord[]>('/api/v1/providers')
    providers.value = Array.isArray(data) ? data : []
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    providers.value = []
  } finally {
    loading.value = false
  }
}

async function toggleExpanded(id: string) {
  if (expandedId.value === id) {
    expandedId.value = null
    expandedModels.value = []
    expandedError.value = ''
    return
  }
  expandedId.value = id
  expandedModels.value = []
  expandedError.value = ''
  expandedLoading.value = true
  try {
    const data = await apiJson<ProviderModelsResponse>(`/api/v1/providers/${encodeURIComponent(id)}/models`)
    expandedModels.value = Array.isArray(data?.models) ? data.models : []
  } catch (err) {
    expandedError.value = err instanceof Error ? err.message : String(err)
    toasts.push('error', expandedError.value)
  } finally {
    expandedLoading.value = false
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="space-y-6">
    <div>
      <div class="text-lg font-medium">Providers</div>
      <div class="mt-1 text-sm text-muted-foreground">Model providers registered with the Agena server.</div>
    </div>

    <div class="grid gap-3">
      <div v-if="loading" class="text-sm text-muted-foreground">Loading providers...</div>
      <div v-else-if="error" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {{ error }}
      </div>
      <div v-else-if="sortedProviders.length === 0" class="text-sm text-muted-foreground">No providers configured.</div>

      <div v-else class="space-y-2">
        <div
          v-for="provider in sortedProviders"
          :key="provider.provider_id"
          class="rounded-md border border-border/60 bg-background/50"
        >
          <div class="flex items-center justify-between gap-3 px-3 py-2.5">
            <button
              type="button"
              class="flex min-w-0 flex-1 items-center gap-2 text-left"
              @click="toggleExpanded(String(provider.provider_id || ''))"
            >
              <RiArrowDownSLine v-if="expandedId === provider.provider_id" class="h-4 w-4 shrink-0 text-muted-foreground" />
              <RiArrowRightSLine v-else class="h-4 w-4 shrink-0 text-muted-foreground" />
              <span class="min-w-0 truncate font-mono text-sm font-semibold">{{ provider.provider_id }}</span>
              <span
                class="inline-flex h-5 shrink-0 items-center rounded-full bg-muted px-2 text-[11px] font-medium text-muted-foreground"
              >
                {{ modelCount(provider) }} models
              </span>
            </button>
          </div>

          <div v-if="expandedId === provider.provider_id" class="border-t border-border/60 px-4 py-3">
            <div v-if="expandedLoading" class="text-xs text-muted-foreground">Loading models...</div>
            <div v-else-if="expandedError" class="text-xs text-destructive break-words">{{ expandedError }}</div>
            <div v-else-if="expandedModels.length === 0" class="text-xs text-muted-foreground">No models reported.</div>
            <ul v-else class="grid gap-1">
              <li v-for="(label, index) in expandedModelLabels" :key="`${expandedId}-${index}`" class="font-mono text-xs text-foreground/80 break-all">
                {{ label }}
              </li>
            </ul>
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
