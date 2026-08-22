<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiArrowDownSLine, RiArrowRightSLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import { buildDefaultModelSettingsPatch, sameServerModelIdentity } from '@/lib/serverModelSettings'
import {
  defaultModeValue,
  speedModeOptionsForModel,
  supportsParallelToolCallsForModel,
  thinkingModeOptionsForModel,
  useModelSelectionCatalog,
  verbosityOptionsForModel,
  type ModelModeOption,
  type ProviderModel,
} from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import { useToastsStore } from '@/stores/toasts'
import { settingsText as st } from '@/i18n/settingsText'

type PanelView = 'all' | 'defaults' | 'inventory'

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

const props = withDefaults(defineProps<{ view?: PanelView }>(), { view: 'all' })
const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()

const loading = ref(false)
const error = ref('')
const providers = ref<ProviderSummary[]>([])
const runtime = ref<RuntimeStatus | null>(null)
const catalog = ref<ModelCatalogList | null>(null)
const expandedId = ref<string | null>(null)
const expandedLoading = ref(false)
const expandedError = ref('')
const expandedAdapters = ref<ConfiguredAdapter[]>([])
const defaultModelKey = ref('')
const defaultThinkingMode = ref('')
const defaultSpeedMode = ref('')
const defaultVerbosity = ref('')
const defaultParallelToolCalls = ref(false)
const defaultSaveBusy = ref(false)
const defaultSaveError = ref('')

const showDefaults = computed(() => props.view === 'all' || props.view === 'defaults')
const showInventory = computed(() => props.view === 'all' || props.view === 'inventory')
const panelTitle = computed(() => (props.view === 'defaults' ? st('Model defaults') : st('Configured provider inventory')))
const panelDescription = computed(() =>
  props.view === 'defaults'
    ? st('Choose the one runtime-wide default model and its optional execution modes.')
    : st('Review the server’s configured providers, enabled adapters, endpoints, and model routes.'),
)

const sortedProviders = computed(() => [...providers.value].sort((a, b) => a.provider_id.localeCompare(b.provider_id)))
const catalogModelCount = computed(() => {
  const counts = [
    runtime.value?.model_catalog?.model_count,
    catalog.value?.summary?.model_count,
    catalog.value?.total,
  ].filter((value): value is number => typeof value === 'number' && Number.isFinite(value))
  return counts.length > 0 ? Math.max(...counts) : 0
})

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
  return [
    selection?.thinking_mode ? st('thinking: {thinking_mode}', { thinking_mode: selection.thinking_mode }) : '',
    selection?.speed_mode ? st('speed: {speed_mode}', { speed_mode: selection.speed_mode }) : '',
    selection?.verbosity ? st('verbosity: {verbosity}', { verbosity: selection.verbosity }) : '',
    typeof selection?.parallel_tool_calls === 'boolean'
      ? st('parallel tools: {value}', { value: selection.parallel_tool_calls ? st('on') : st('off') })
      : '',
  ]
    .filter(Boolean)
    .join(' · ')
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
  return options.sort((left, right) => `${left.description}/${left.label}`.localeCompare(`${right.description}/${right.label}`))
})

const selectedDefaultIdentity = computed(() => parseModelSlug(defaultModelKey.value))
const selectedDefaultModel = computed(() => {
  const selection = selectedDefaultIdentity.value
  return modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
})

function withSelectedMode(options: ModelModeOption[], selected: string): ModelModeOption[] {
  const value = String(selected || '').trim()
  if (!value || options.some((option) => option.value === value)) return options
  return [...options, { value, label: value, description: st('Configured value'), isDefault: false }]
}

const defaultThinkingOptions = computed(() =>
  withSelectedMode(thinkingModeOptionsForModel(selectedDefaultModel.value), defaultThinkingMode.value),
)
const defaultSpeedOptions = computed(() =>
  withSelectedMode(speedModeOptionsForModel(selectedDefaultModel.value), defaultSpeedMode.value),
)
const defaultVerbosityOptions = computed(() =>
  withSelectedMode(verbosityOptionsForModel(selectedDefaultModel.value), defaultVerbosity.value),
)
const selectedSupportsParallelTools = computed(() => supportsParallelToolCallsForModel(selectedDefaultModel.value))

function syncDefaultEditor() {
  const selection = runtime.value?.default_selection
  defaultModelKey.value = encodeModelSelectionKey({
    provider: String(selection?.provider || ''),
    adapter: String(selection?.adapter || ''),
    model: String(selection?.model || ''),
  })
  defaultThinkingMode.value = String(selection?.thinking_mode || '').trim()
  defaultSpeedMode.value = String(selection?.speed_mode || '').trim()
  defaultVerbosity.value = String(selection?.verbosity || '').trim()
  defaultParallelToolCalls.value = selection?.parallel_tool_calls === true
  defaultSaveError.value = ''
}

function chooseDefaultModel(value: string) {
  defaultModelKey.value = value
  const selection = parseModelSlug(value)
  const model = modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
  defaultThinkingMode.value = defaultModeValue(thinkingModeOptionsForModel(model))
  defaultSpeedMode.value = defaultModeValue(speedModeOptionsForModel(model))
  defaultVerbosity.value = defaultModeValue(verbosityOptionsForModel(model))
  defaultParallelToolCalls.value = false
  defaultSaveError.value = ''
}

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
      modelSelectionCatalog.loadProvidersAndModels(),
    ])
    providers.value = Array.isArray(providerData) ? providerData : []
    runtime.value = runtimeData && typeof runtimeData === 'object' ? runtimeData : null
    catalog.value = catalogData && typeof catalogData === 'object' ? catalogData : null
    syncDefaultEditor()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    providers.value = []
  } finally {
    loading.value = false
  }
}

async function saveDefaultSelection() {
  if (defaultSaveBusy.value) return
  const selected = selectedDefaultIdentity.value
  if (!selected.provider || !selected.model) {
    defaultSaveError.value = st('Select a configured model.')
    return
  }

  defaultSaveBusy.value = true
  defaultSaveError.value = ''
  const desiredThinkingMode = defaultThinkingMode.value.trim()
  const desiredSpeedMode = defaultSpeedMode.value.trim()
  const desiredVerbosity = defaultVerbosity.value.trim()
  const desiredParallelTools = selectedSupportsParallelTools.value ? defaultParallelToolCalls.value : undefined
  try {
    await apiJson('/api/v1/settings', {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(
        buildDefaultModelSettingsPatch(selected, {
          ...(desiredThinkingMode ? { thinkingMode: desiredThinkingMode } : {}),
          ...(desiredSpeedMode ? { speedMode: desiredSpeedMode } : {}),
          ...(desiredVerbosity ? { verbosity: desiredVerbosity } : {}),
          ...(typeof desiredParallelTools === 'boolean' ? { parallelToolCalls: desiredParallelTools } : {}),
        }),
      ),
    })
    await refresh()
    const applied = runtime.value?.default_selection
    if (
      !sameServerModelIdentity(applied, selected) ||
      String(applied?.thinking_mode || '').trim() !== desiredThinkingMode ||
      String(applied?.speed_mode || '').trim() !== desiredSpeedMode ||
      String(applied?.verbosity || '').trim() !== desiredVerbosity ||
      (typeof desiredParallelTools === 'boolean' && applied?.parallel_tool_calls !== desiredParallelTools)
    ) {
      throw new Error(st('The server accepted the update but did not apply the selected default.'))
    }
    toasts.push('success', st('Runtime default model updated'))
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
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
        <h2 class="text-base font-semibold">{{ panelTitle }}</h2>
        <p class="mt-1 max-w-3xl text-sm text-muted-foreground">{{ panelDescription }}</p>
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

    <dl class="grid grid-cols-1 gap-x-6 gap-y-3 border-y border-border/60 py-4 sm:grid-cols-3">
      <div>
        <dt class="text-xs text-muted-foreground">{{ $st('Model Catalog') }}</dt>
        <dd class="mt-1 font-mono text-lg font-semibold tabular-nums">{{ catalogModelCount }}</dd>
      </div>
      <div class="sm:col-span-2">
        <dt class="text-xs text-muted-foreground">{{ $st('Runtime default') }}</dt>
        <dd class="mt-1 break-all font-mono text-sm font-semibold">{{ defaultSelectionLabel }}</dd>
        <div v-if="defaultModesLabel" class="mt-0.5 text-[11px] text-muted-foreground">{{ defaultModesLabel }}</div>
      </div>
    </dl>

    <template v-if="showDefaults">
      <section class="grid gap-4 rounded-lg border border-border/60 p-4">
        <div>
          <h3 class="text-sm font-semibold">{{ $st('Runtime default model') }}</h3>
          <p class="mt-1 text-xs leading-5 text-muted-foreground">
            {{ $st('Used when a new run does not provide an explicit model. Provider settings do not define a separate default.') }}
          </p>
        </div>
        <div class="grid gap-3 xl:grid-cols-2">
          <label class="grid min-w-0 gap-1.5 xl:col-span-2">
            <span class="text-xs text-muted-foreground">{{ $st('Model route') }}</span>
            <OptionPicker
              :model-value="defaultModelKey"
              :options="defaultModelOptions"
              :title="$st('Runtime default model')"
              :placeholder="$st('Select a configured model')"
              :search-placeholder="$st('Search configured models...')"
              :include-empty="false"
              :disabled="loading || defaultSaveBusy"
              monospace
              @update:model-value="chooseDefaultModel"
            />
          </label>
          <label class="grid min-w-0 gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Thinking') }}</span>
            <OptionPicker
              v-model="defaultThinkingMode"
              :options="defaultThinkingOptions"
              :title="$st('Default thinking mode')"
              :empty-label="$st('Model default')"
              :disabled="loading || defaultSaveBusy || !defaultModelKey"
            />
          </label>
          <label class="grid min-w-0 gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Speed') }}</span>
            <OptionPicker
              v-model="defaultSpeedMode"
              :options="defaultSpeedOptions"
              :title="$st('Default speed mode')"
              :empty-label="$st('Model default')"
              :disabled="loading || defaultSaveBusy || !defaultModelKey"
            />
          </label>
          <label class="grid min-w-0 gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Verbosity') }}</span>
            <OptionPicker
              v-model="defaultVerbosity"
              :options="defaultVerbosityOptions"
              :title="$st('Default verbosity')"
              :empty-label="$st('Model default')"
              :disabled="loading || defaultSaveBusy || !defaultModelKey || defaultVerbosityOptions.length === 0"
            />
          </label>
          <label class="flex min-h-9 items-center gap-2 rounded-md border border-border/60 px-3 text-sm">
            <input
              v-model="defaultParallelToolCalls"
              type="checkbox"
              :disabled="loading || defaultSaveBusy || !selectedSupportsParallelTools"
            />
            <span>
              {{ $st('Parallel tool calls') }}
              <span v-if="!selectedSupportsParallelTools" class="ml-1 text-xs text-muted-foreground">
                {{ $st('not supported') }}
              </span>
            </span>
          </label>
        </div>
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div v-if="modelSelectionCatalog.catalogError.value" class="break-words text-xs text-destructive">
            {{ modelSelectionCatalog.catalogError.value }}
          </div>
          <div v-else-if="defaultSaveError" class="break-words text-xs text-destructive">{{ defaultSaveError }}</div>
          <span v-else class="text-xs text-muted-foreground">{{ $st('Clear a mode to inherit the model’s native default.') }}</span>
          <Button :disabled="loading || defaultSaveBusy || !defaultModelKey" @click="saveDefaultSelection">
            <RiSave3Line class="mr-2 h-4 w-4" />
            {{ defaultSaveBusy ? $st('Saving…') : $st('Save runtime default') }}
          </Button>
        </div>
      </section>

      <ApprovalModelPanel />
    </template>

    <section v-if="showInventory" class="grid gap-3">
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
