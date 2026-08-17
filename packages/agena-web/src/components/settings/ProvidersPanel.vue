<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiArrowDownSLine, RiArrowRightSLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'
import { apiJson } from '@/lib/api'
import {
  buildProviderDefaultSettingsPatch,
  normalizeServerModelIdentity,
  sameServerModelIdentity,
} from '@/lib/serverModelSettings'
import {
  defaultModeValue,
  speedModeOptionsForModel,
  thinkingModeOptionsForModel,
  useModelSelectionCatalog,
  type ModelModeOption,
  type ProviderModel,
} from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import { useToastsStore } from '@/stores/toasts'

type ProviderAdapterSummary = {
  adapter_id: string
  enabled: boolean
  configured_model_count: number
}

type ProviderSummary = {
  provider_id: string
  defaults: { adapter?: string | null; model: string }
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
  default_selection?: {
    provider?: string | null
    adapter?: string | null
    model?: string | null
    thinking_mode?: string | null
    speed_mode?: string | null
    verbosity?: string | null
    parallel_tool_calls?: boolean | null
  } | null
}

type ModelCatalogList = {
  summary?: ModelCatalogSummary
  total?: number
}

const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()

const loading = ref(false)
const error = ref('')
const providers = ref<ProviderSummary[]>([])
const runtime = ref<RuntimeStatus | null>(null)
const catalog = ref<ModelCatalogList | null>(null)
const catalogRefreshBusy = ref(false)
const expandedId = ref<string | null>(null)
const expandedLoading = ref(false)
const expandedError = ref('')
const expandedAdapters = ref<ConfiguredAdapter[]>([])
const defaultModelKey = ref('')
const defaultThinkingMode = ref('')
const defaultSpeedMode = ref('')
const defaultSaveBusy = ref(false)
const defaultSaveError = ref('')

const sortedProviders = computed(() => [...providers.value].sort((a, b) => a.provider_id.localeCompare(b.provider_id)))

const catalogModelCount = computed(() => {
  const counts = [
    runtime.value?.model_catalog?.model_count,
    catalog.value?.summary?.model_count,
    catalog.value?.total,
  ].filter((value): value is number => typeof value === 'number' && Number.isFinite(value))
  return counts.length > 0 ? Math.max(...counts) : 0
})

const catalogRefreshing = computed(() =>
  Boolean(runtime.value?.model_catalog?.refreshing || catalog.value?.summary?.refreshing || catalogRefreshBusy.value),
)

const defaultSelectionLabel = computed(() => {
  const selection = runtime.value?.default_selection
  const provider = String(selection?.provider || '').trim()
  const adapter = String(selection?.adapter || '').trim()
  const model = String(selection?.model || '').trim()
  if (!provider || !model) return 'Unset'
  return [provider, adapter, model].filter(Boolean).join(' / ')
})

const defaultModesLabel = computed(() => {
  const selection = runtime.value?.default_selection
  const values = [
    selection?.thinking_mode ? `thinking: ${selection.thinking_mode}` : '',
    selection?.speed_mode ? `speed: ${selection.speed_mode}` : '',
  ].filter(Boolean)
  return values.join(' · ')
})

const defaultModelOptions = computed(() => {
  const options: Array<{ value: string; label: string; description: string }> = []
  for (const provider of modelSelectionCatalog.providers.value) {
    for (const model of provider.models) {
      const adapter = String(model.adapter_id || '').trim()
      const value = encodeModelSelectionKey({ provider: provider.id, adapter, model: model.id })
      if (!value) continue
      options.push({
        value,
        label: String(model.display_name || model.id),
        description: [provider.id, adapter, model.id].filter(Boolean).join(' / '),
      })
    }
  }
  return options.sort((left, right) =>
    `${left.description}/${left.label}`.localeCompare(`${right.description}/${right.label}`),
  )
})

const selectedDefaultIdentity = computed(() => parseModelSlug(defaultModelKey.value))
const selectedDefaultModel = computed(() => {
  const selection = selectedDefaultIdentity.value
  return modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
})

function withSelectedMode(options: ModelModeOption[], selected: string): ModelModeOption[] {
  const value = String(selected || '').trim()
  if (!value || options.some((option) => option.value === value)) return options
  return [...options, { value, label: value, description: '', isDefault: false }]
}

const defaultThinkingOptions = computed(() =>
  withSelectedMode(thinkingModeOptionsForModel(selectedDefaultModel.value), defaultThinkingMode.value),
)
const defaultSpeedOptions = computed(() =>
  withSelectedMode(speedModeOptionsForModel(selectedDefaultModel.value), defaultSpeedMode.value),
)

function syncDefaultEditor() {
  const selection = runtime.value?.default_selection
  defaultModelKey.value = encodeModelSelectionKey({
    provider: String(selection?.provider || ''),
    adapter: String(selection?.adapter || ''),
    model: String(selection?.model || ''),
  })
  defaultThinkingMode.value = String(selection?.thinking_mode || '').trim()
  defaultSpeedMode.value = String(selection?.speed_mode || '').trim()
  defaultSaveError.value = ''
}

function chooseDefaultModel(value: string) {
  defaultModelKey.value = value
  const selection = parseModelSlug(value)
  const model = modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
  defaultThinkingMode.value = defaultModeValue(thinkingModeOptionsForModel(model))
  defaultSpeedMode.value = defaultModeValue(speedModeOptionsForModel(model))
  defaultSaveError.value = ''
}

function configuredModelCount(provider: ProviderSummary): number {
  return (Array.isArray(provider.adapters) ? provider.adapters : []).reduce((total, adapter) => {
    const count = Number(adapter.configured_model_count)
    return total + (Number.isFinite(count) && count > 0 ? Math.floor(count) : 0)
  }, 0)
}

function providerDefaultLabel(provider: ProviderSummary): string {
  return [provider.defaults?.adapter, provider.defaults?.model].filter(Boolean).join(' / ') || 'Unset'
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
      modelSelectionCatalog.loadProvidersAndModels(),
    ])
    providers.value = Array.isArray(providerData) ? providerData : []
    runtime.value = runtimeData && typeof runtimeData === 'object' ? runtimeData : null
    catalog.value = catalogData && typeof catalogData === 'object' ? catalogData : null
    syncDefaultEditor()
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    providers.value = []
  } finally {
    loading.value = false
  }
}

async function saveDefaultSelection() {
  if (defaultSaveBusy.value) return
  const selected = selectedDefaultIdentity.value
  if (!selected.provider || !selected.model) {
    defaultSaveError.value = 'Select a configured model.'
    return
  }

  defaultSaveBusy.value = true
  defaultSaveError.value = ''
  const previous = normalizeServerModelIdentity(runtime.value?.default_selection)
  const preserveRuntimeModes = sameServerModelIdentity(previous, selected)
  const desiredThinkingMode = defaultThinkingMode.value.trim()
  const desiredSpeedMode = defaultSpeedMode.value.trim()
  try {
    await apiJson('/api/v1/settings', {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(
        buildProviderDefaultSettingsPatch(selected, {
          ...(desiredThinkingMode ? { thinkingMode: desiredThinkingMode } : {}),
          ...(desiredSpeedMode ? { speedMode: desiredSpeedMode } : {}),
          ...(preserveRuntimeModes && runtime.value?.default_selection?.verbosity
            ? { verbosity: runtime.value.default_selection.verbosity }
            : {}),
          ...(preserveRuntimeModes && typeof runtime.value?.default_selection?.parallel_tool_calls === 'boolean'
            ? { parallelToolCalls: runtime.value.default_selection.parallel_tool_calls }
            : {}),
        }),
      ),
    })
    await refresh()
    const applied = runtime.value?.default_selection
    if (
      !sameServerModelIdentity(applied, selected) ||
      String(applied?.thinking_mode || '').trim() !== desiredThinkingMode ||
      String(applied?.speed_mode || '').trim() !== desiredSpeedMode
    ) {
      throw new Error('The server accepted the update but did not apply the selected default.')
    }
    toasts.push('success', 'Runtime default model updated')
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    defaultSaveError.value = message
    toasts.push('error', message)
  } finally {
    defaultSaveBusy.value = false
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
  } catch (err) {
    expandedError.value = err instanceof Error ? err.message : String(err)
    toasts.push('error', expandedError.value)
  } finally {
    expandedLoading.value = false
  }
}

async function refreshCatalog() {
  if (catalogRefreshBusy.value) return
  catalogRefreshBusy.value = true
  try {
    await apiJson('/api/v1/model-catalog/refresh', { method: 'POST' })
    toasts.push('success', 'Model catalog refresh started')
    await refresh()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    catalogRefreshBusy.value = false
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="space-y-6">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">Providers</div>
        <div class="mt-1 text-sm text-muted-foreground">Models configured on the Agena server.</div>
      </div>
      <div class="flex items-center gap-2">
        <Button variant="outline" size="sm" :disabled="catalogRefreshBusy" @click="refreshCatalog">
          <RiRefreshLine class="mr-2 h-4 w-4" :class="catalogRefreshBusy ? 'animate-spin' : ''" />
          Refresh catalog
        </Button>
        <IconButton
          variant="outline"
          size="md"
          :tooltip="loading ? 'Refreshing providers' : 'Refresh providers'"
          :aria-label="loading ? 'Refreshing providers' : 'Refresh providers'"
          :disabled="loading"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </div>
    </div>

    <dl class="grid grid-cols-1 gap-x-6 gap-y-3 border-y border-border/60 py-4 sm:grid-cols-3">
      <div>
        <dt class="text-xs text-muted-foreground">Model Catalog</dt>
        <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">{{ catalogModelCount }}</dd>
        <div v-if="catalogRefreshing" class="mt-0.5 text-[11px] text-muted-foreground">Refreshing</div>
      </div>
      <div class="sm:col-span-2">
        <dt class="text-xs text-muted-foreground">Runtime default</dt>
        <dd class="mt-1 break-all font-mono text-sm font-semibold">{{ defaultSelectionLabel }}</dd>
        <div v-if="defaultModesLabel" class="mt-0.5 text-[11px] text-muted-foreground">{{ defaultModesLabel }}</div>
      </div>
    </dl>

    <section class="grid gap-3 border-b border-border/60 pb-6">
      <div>
        <h2 class="text-sm font-medium">Runtime default model</h2>
        <p class="mt-1 text-xs text-muted-foreground">
          Used when a new run does not provide an explicit model selection.
        </p>
      </div>
      <div class="grid gap-3 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)_auto] lg:items-end">
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">Model</span>
          <OptionPicker
            :model-value="defaultModelKey"
            :options="defaultModelOptions"
            title="Runtime default model"
            placeholder="Select a configured model"
            search-placeholder="Search configured models..."
            :include-empty="false"
            :disabled="loading || defaultSaveBusy"
            monospace
            @update:model-value="chooseDefaultModel"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">Thinking</span>
          <OptionPicker
            v-model="defaultThinkingMode"
            :options="defaultThinkingOptions"
            title="Default thinking mode"
            empty-label="Model default"
            :include-empty="true"
            :disabled="loading || defaultSaveBusy || !defaultModelKey"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">Speed</span>
          <OptionPicker
            v-model="defaultSpeedMode"
            :options="defaultSpeedOptions"
            title="Default speed mode"
            empty-label="Model default"
            :include-empty="true"
            :disabled="loading || defaultSaveBusy || !defaultModelKey"
          />
        </label>
        <Button class="h-10" :disabled="loading || defaultSaveBusy || !defaultModelKey" @click="saveDefaultSelection">
          <RiSave3Line class="mr-2 h-4 w-4" />
          {{ defaultSaveBusy ? 'Saving...' : 'Save default' }}
        </Button>
      </div>
      <div v-if="modelSelectionCatalog.catalogError.value" class="break-words text-xs text-destructive">
        {{ modelSelectionCatalog.catalogError.value }}
      </div>
      <div v-if="defaultSaveError" class="break-words text-xs text-destructive">{{ defaultSaveError }}</div>
    </section>

    <ApprovalModelPanel />

    <div class="grid gap-3">
      <div v-if="loading" class="text-sm text-muted-foreground">Loading providers...</div>
      <div
        v-else-if="error"
        class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {{ error }}
      </div>
      <div v-else-if="sortedProviders.length === 0" class="text-sm text-muted-foreground">No providers configured.</div>

      <div v-else class="space-y-2">
        <div
          v-for="provider in sortedProviders"
          :key="provider.provider_id"
          class="rounded-md border border-border/60 bg-background/50"
        >
          <button
            type="button"
            class="flex w-full min-w-0 items-center justify-between gap-3 px-3 py-2.5 text-left"
            @click="toggleExpanded(provider.provider_id)"
          >
            <span class="flex min-w-0 items-center gap-2">
              <RiArrowDownSLine
                v-if="expandedId === provider.provider_id"
                class="h-4 w-4 shrink-0 text-muted-foreground"
              />
              <RiArrowRightSLine v-else class="h-4 w-4 shrink-0 text-muted-foreground" />
              <span class="min-w-0 truncate font-mono text-sm font-semibold">{{ provider.provider_id }}</span>
            </span>
            <span class="flex shrink-0 items-center gap-3 text-[11px] text-muted-foreground">
              <span class="hidden font-mono sm:inline">default: {{ providerDefaultLabel(provider) }}</span>
              <span class="rounded bg-muted px-2 py-0.5 font-medium tabular-nums">
                {{ configuredModelCount(provider) }} models
              </span>
            </span>
          </button>

          <div v-if="expandedId === provider.provider_id" class="border-t border-border/60 px-4 py-3">
            <div v-if="expandedLoading" class="text-xs text-muted-foreground">Loading configured models...</div>
            <div v-else-if="expandedError" class="break-words text-xs text-destructive">{{ expandedError }}</div>
            <div v-else class="space-y-3">
              <div
                v-for="adapter in expandedAdapters"
                :key="adapter.adapter_id"
                class="border-b border-border/50 pb-3 last:border-b-0 last:pb-0"
              >
                <div class="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
                  <span class="font-mono font-semibold">{{ adapter.adapter_id }}</span>
                  <span :class="adapter.enabled ? 'text-success' : 'text-muted-foreground'">
                    {{ adapter.enabled ? 'enabled' : 'disabled' }}
                  </span>
                  <span v-if="adapter.resolved_base_url" class="break-all font-mono text-[11px] text-muted-foreground">
                    {{ adapter.resolved_base_url }}
                  </span>
                </div>
                <div v-if="adapterFailure(adapter)" class="mt-1 break-words text-xs text-destructive">
                  {{ adapterFailure(adapter) }}
                </div>
                <ul v-if="adapter.models?.length" class="mt-2 grid gap-1">
                  <li v-for="model in adapter.models" :key="model.id" class="flex min-w-0 gap-2 text-xs">
                    <span class="min-w-0 break-words">{{ model.display_name || model.id }}</span>
                    <span
                      v-if="model.display_name && model.display_name !== model.id"
                      class="break-all font-mono text-muted-foreground"
                    >
                      {{ model.id }}
                    </span>
                  </li>
                </ul>
                <div v-else-if="!adapterFailure(adapter)" class="mt-1 text-xs text-muted-foreground">
                  No configured models.
                </div>
              </div>
              <div v-if="expandedAdapters.length === 0" class="text-xs text-muted-foreground">
                No adapters reported.
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
