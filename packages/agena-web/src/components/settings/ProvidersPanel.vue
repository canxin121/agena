<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiArrowDownSLine, RiArrowRightSLine, RiRefreshLine } from '@remixicon/vue'

import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import type { ProviderModel } from '@/pages/chat/modelSelectionCatalog'
import { useToastsStore } from '@/stores/toasts'

type ProviderAdapterSummary = {
  adapter_id: string
  enabled: boolean
  configured_model_count: number
}

type ProviderSummary = {
  provider_id: string
  adapters?: ProviderAdapterSummary[]
}

type ConfiguredAdapter = {
  adapter_id: string
  enabled: boolean
  resolved_base_url?: string | null
  models?: ProviderModel[]
  failure?: { message?: string; rendered?: string; user?: { fallback?: string } } | null
}

type ModelCatalogSummary = {
  refreshing?: boolean
  model_count?: number
  last_refresh_at?: string | null
  last_failure?: { message?: string; rendered?: string } | null
}

type RuntimeStatus = {
  model_catalog?: ModelCatalogSummary | null
}

type ModelCatalogList = {
  summary?: ModelCatalogSummary
  total?: number
}

const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const providers = ref<ProviderSummary[]>([])
const runtime = ref<RuntimeStatus | null>(null)
const catalog = ref<ModelCatalogList | null>(null)
const expandedId = ref<string | null>(null)
const expandedLoading = ref(false)
const expandedError = ref('')
const expandedAdapters = ref<ConfiguredAdapter[]>([])

const sortedProviders = computed(() => [...providers.value].sort((a, b) => a.provider_id.localeCompare(b.provider_id)))
const catalogModelCount = computed(() => {
  const counts = [
    runtime.value?.model_catalog?.model_count,
    catalog.value?.summary?.model_count,
    catalog.value?.total,
  ].filter((value): value is number => typeof value === 'number' && Number.isFinite(value))
  return counts.length > 0 ? Math.max(...counts) : 0
})

function configuredModelCount(provider: ProviderSummary): number {
  return (Array.isArray(provider.adapters) ? provider.adapters : []).reduce((total, adapter) => {
    const count = Number(adapter.configured_model_count)
    return total + (Number.isFinite(count) && count > 0 ? Math.floor(count) : 0)
  }, 0)
}

function adapterFailure(adapter: ConfiguredAdapter): string {
  return String(adapter.failure?.user?.fallback || adapter.failure?.rendered || adapter.failure?.message || '').trim()
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const [providerData, runtimeData, catalogData] = await Promise.all([
      apiJson<ProviderSummary[]>('/api/v1/providers'),
      apiJson<RuntimeStatus>('/api/v1/runtime'),
      apiJson<ModelCatalogList>('/api/v1/model-catalog?offset=0&limit=1'),
    ])
    providers.value = Array.isArray(providerData) ? providerData : []
    runtime.value = runtimeData && typeof runtimeData === 'object' ? runtimeData : null
    catalog.value = catalogData && typeof catalogData === 'object' ? catalogData : null
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    providers.value = []
  } finally {
    loading.value = false
  }
}

async function toggleExpanded(id: string) {
  if (!id) return
  if (expandedId.value === id) {
    expandedId.value = null
    expandedAdapters.value = []
    expandedError.value = ''
    return
  }
  expandedId.value = id
  expandedAdapters.value = []
  expandedError.value = ''
  expandedLoading.value = true
  try {
    const data = await apiJson<ConfiguredAdapter[]>(`/api/v1/providers/${encodeURIComponent(id)}/configured-models`)
    expandedAdapters.value = Array.isArray(data) ? data : []
  } catch (reason) {
    expandedError.value = reason instanceof Error ? reason.message : String(reason)
    toasts.push('error', expandedError.value)
  } finally {
    expandedLoading.value = false
  }
}

onMounted(() => void refresh())
</script>

<template>
  <div class="grid min-w-0 gap-6">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-semibold">{{ $st('Configured provider inventory') }}</h2>
        <p class="mt-1 max-w-3xl text-sm text-muted-foreground">
          {{ $st('Review the server’s configured providers, enabled adapters, endpoints, and model routes.') }}
        </p>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? $st('Refreshing model settings') : $st('Refresh model settings')"
        :aria-label="loading ? $st('Refreshing model settings') : $st('Refresh model settings')"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <dl class="grid grid-cols-1 gap-x-6 gap-y-3 border-y border-border/60 py-4 sm:grid-cols-2">
      <div>
        <dt class="text-xs text-muted-foreground">{{ $st('Model Catalog') }}</dt>
        <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">{{ catalogModelCount }}</dd>
      </div>
      <div>
        <dt class="text-xs text-muted-foreground">{{ $st('Configured providers') }}</dt>
        <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">{{ sortedProviders.length }}</dd>
      </div>
    </dl>

    <ApprovalModelPanel />

    <section class="grid gap-3">
      <div v-if="loading && sortedProviders.length === 0" class="text-sm text-muted-foreground">
        {{ $st('Loading providers…') }}
      </div>
      <div v-else-if="sortedProviders.length === 0" class="text-sm text-muted-foreground">
        {{ $st('No providers configured.') }}
      </div>
      <div v-else class="grid gap-2">
        <article
          v-for="provider in sortedProviders"
          :key="provider.provider_id"
          class="overflow-hidden rounded-lg border border-border/60 bg-background/50"
        >
          <button
            type="button"
            class="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-3 text-left hover:bg-muted/30"
            @click="toggleExpanded(provider.provider_id)"
          >
            <span class="flex min-w-0 items-center gap-2">
              <RiArrowDownSLine
                v-if="expandedId === provider.provider_id"
                class="h-4 w-4 shrink-0 text-muted-foreground"
              />
              <RiArrowRightSLine v-else class="h-4 w-4 shrink-0 text-muted-foreground" />
              <span class="min-w-0">
                <span class="block truncate font-mono text-sm font-semibold">{{ provider.provider_id }}</span>
                <span class="mt-0.5 block truncate font-mono text-[11px] text-muted-foreground">
                  {{ $st('configured routes') }}
                </span>
              </span>
            </span>
            <span class="shrink-0 rounded bg-muted px-2 py-0.5 text-[11px] font-medium tabular-nums">
              {{ configuredModelCount(provider) }} {{ $st('models') }}
            </span>
          </button>

          <div v-if="expandedId === provider.provider_id" class="border-t border-border/60 px-4 py-3">
            <div v-if="expandedLoading" class="text-xs text-muted-foreground">
              {{ $st('Loading configured models…') }}
            </div>
            <div v-else-if="expandedError" class="break-words text-xs text-destructive">{{ expandedError }}</div>
            <div v-else class="grid gap-4">
              <section
                v-for="adapter in expandedAdapters"
                :key="adapter.adapter_id"
                class="grid gap-2 border-b border-border/50 pb-4 last:border-b-0 last:pb-0"
              >
                <div class="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
                  <span class="font-mono font-semibold">{{ adapter.adapter_id }}</span>
                  <span :class="adapter.enabled ? 'text-success' : 'text-muted-foreground'">
                    {{ adapter.enabled ? $st('enabled') : $st('disabled') }}
                  </span>
                  <span v-if="adapter.resolved_base_url" class="break-all font-mono text-[11px] text-muted-foreground">
                    {{ adapter.resolved_base_url }}
                  </span>
                </div>
                <div v-if="adapterFailure(adapter)" class="break-words text-xs text-destructive">
                  {{ adapterFailure(adapter) }}
                </div>
                <ul v-if="adapter.models?.length" class="grid gap-1 sm:grid-cols-2">
                  <li
                    v-for="model in adapter.models"
                    :key="model.id"
                    class="min-w-0 rounded bg-muted/20 px-2 py-1.5 text-xs"
                  >
                    <span class="block truncate">{{ model.display_name || model.id }}</span>
                    <code
                      v-if="model.display_name && model.display_name !== model.id"
                      class="block truncate font-mono text-[10px] text-muted-foreground"
                    >
                      {{ model.id }}
                    </code>
                  </li>
                </ul>
                <div v-else-if="!adapterFailure(adapter)" class="text-xs text-muted-foreground">
                  {{ $st('No configured models.') }}
                </div>
              </section>
              <div v-if="expandedAdapters.length === 0" class="text-xs text-muted-foreground">
                {{ $st('No adapters reported.') }}
              </div>
            </div>
          </div>
        </article>
      </div>
    </section>
  </div>
</template>
