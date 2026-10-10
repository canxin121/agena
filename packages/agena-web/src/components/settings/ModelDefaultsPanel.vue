<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RiDeleteBinLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { sameServerModelIdentity } from '@/lib/serverModelSettings'
import {
  deleteRuntimeSetting,
  hasPersistedSetting,
  readRuntimeSettingSources,
  setRuntimeSetting,
  type RuntimeSettingsReadBundle,
} from '@/lib/runtimeSettings'
import {
  defaultModeValue,
  speedModeOptionsForModel,
  supportsParallelToolCallsForModel,
  thinkingModeOptionsForModel,
  useModelSelectionCatalog,
  verbosityOptionsForModel,
  type ModelModeOption,
} from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

type ModelDefaultScope = 'effective' | 'global' | 'workspace'

type ModelSelectionValue = {
  provider?: string | null
  adapter?: string | null
  model?: string | null
  thinking_mode?: string | null
  speed_mode?: string | null
  verbosity?: string | null
  parallel_tool_calls?: boolean | null
}

const DEFAULT_SELECTION_PATH = 'providers.default_selection'

const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()

const scope = ref<ModelDefaultScope>('effective')
const loading = ref(false)
const error = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const modelKey = ref('')
const thinkingMode = ref('')
const speedMode = ref('')
const verbosity = ref('')
const parallelToolCalls = ref(false)
const saveBusy = ref(false)
const saveError = ref('')

const editable = computed(() => scope.value !== 'effective')
const selectedLayer = computed<'global' | 'workspace' | null>(() =>
  editable.value ? (scope.value as 'global' | 'workspace') : null,
)

function asSelection(value: JsonValue | undefined): ModelSelectionValue {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  return value as ModelSelectionValue
}

const layerSelections = computed(() => ({
  effective: asSelection(sources.value?.effective?.value),
  global: asSelection(sources.value?.global?.value),
  workspace: asSelection(sources.value?.workspace?.value),
}))

function selectionLabel(selection: ModelSelectionValue): string {
  const provider = String(selection.provider || '').trim()
  const model = String(selection.model || '').trim()
  if (!provider || !model) return st('Unset')
  return [provider, String(selection.adapter || '').trim(), model].filter(Boolean).join(' / ')
}

const scopeOptions = computed(() => [
  {
    value: 'effective' as const,
    label: st('Effective default'),
    description: st('Read-only merged default.'),
    summary: selectionLabel(layerSelections.value.effective),
  },
  {
    value: 'global' as const,
    label: st('Global default'),
    description: st('Baseline for every workspace.'),
    summary: selectionLabel(layerSelections.value.global),
  },
  {
    value: 'workspace' as const,
    label: st('Workspace default'),
    description: st('Overrides the global default for this workspace.'),
    summary: selectionLabel(layerSelections.value.workspace),
  },
])

const activeScopeOption = computed(
  () => scopeOptions.value.find((option) => option.value === scope.value) || scopeOptions.value[0],
)

const layerOverridePersisted = computed(() => {
  if (!editable.value) return false
  const response = scope.value === 'workspace' ? sources.value?.workspace : sources.value?.global
  return hasPersistedSetting(response)
})

const layerOverrideLabel = computed(() => (scope.value === 'workspace' ? st('Workspace') : st('Global')))

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

const selectedDefaultIdentity = computed(() => parseModelSlug(modelKey.value))
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
  withSelectedMode(thinkingModeOptionsForModel(selectedDefaultModel.value), thinkingMode.value),
)
const defaultSpeedOptions = computed(() =>
  withSelectedMode(speedModeOptionsForModel(selectedDefaultModel.value), speedMode.value),
)
const defaultVerbosityOptions = computed(() =>
  withSelectedMode(verbosityOptionsForModel(selectedDefaultModel.value), verbosity.value),
)
const selectedSupportsParallelTools = computed(() => supportsParallelToolCallsForModel(selectedDefaultModel.value))

function syncEditor() {
  const selection = layerSelections.value[scope.value]
  modelKey.value = encodeModelSelectionKey({
    provider: String(selection.provider || ''),
    adapter: String(selection.adapter || ''),
    model: String(selection.model || ''),
  })
  thinkingMode.value = String(selection.thinking_mode || '').trim()
  speedMode.value = String(selection.speed_mode || '').trim()
  verbosity.value = String(selection.verbosity || '').trim()
  parallelToolCalls.value = selection.parallel_tool_calls === true
  saveError.value = ''
}

function chooseDefaultModel(value: string) {
  modelKey.value = value
  const selection = parseModelSlug(value)
  const model = modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
  thinkingMode.value = defaultModeValue(thinkingModeOptionsForModel(model))
  speedMode.value = defaultModeValue(speedModeOptionsForModel(model))
  verbosity.value = defaultModeValue(verbosityOptionsForModel(model))
  parallelToolCalls.value = false
  saveError.value = ''
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const [bundle] = await Promise.all([
      readRuntimeSettingSources(DEFAULT_SELECTION_PATH),
      modelSelectionCatalog.loadProvidersAndModels(),
    ])
    sources.value = bundle
    syncEditor()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

async function saveDefaultSelection() {
  const layer = selectedLayer.value
  if (!layer || saveBusy.value) return
  const selected = selectedDefaultIdentity.value
  if (!selected.provider || !selected.model) {
    saveError.value = st('Select a configured model.')
    return
  }

  saveBusy.value = true
  saveError.value = ''
  const device = { thinking_mode: thinkingMode.value.trim(), speed_mode: speedMode.value.trim() }
  const selection: Record<string, JsonValue> = {
    provider: selected.provider,
    ...(selected.adapter ? { adapter: selected.adapter } : {}),
    model: selected.model,
    ...(device.thinking_mode ? { thinking_mode: device.thinking_mode } : {}),
    ...(device.speed_mode ? { speed_mode: device.speed_mode } : {}),
    ...(verbosity.value.trim() ? { verbosity: verbosity.value.trim() } : {}),
    ...(selectedSupportsParallelTools.value ? { parallel_tool_calls: parallelToolCalls.value } : {}),
  }
  try {
    await setRuntimeSetting(DEFAULT_SELECTION_PATH, selection, { reload: true }, layer)
    await refresh()
    const applied = layerSelections.value[layer]
    if (!sameServerModelIdentity(applied, selected)) {
      throw new Error(st('The server accepted the update but did not apply the selected default.'))
    }
    toasts.push('success', st('Default model updated'))
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    saveError.value = message
    toasts.push('error', message)
  } finally {
    saveBusy.value = false
  }
}

async function clearLayerOverride() {
  const layer = selectedLayer.value
  if (!layer || saveBusy.value) return
  saveBusy.value = true
  saveError.value = ''
  try {
    await deleteRuntimeSetting(DEFAULT_SELECTION_PATH, { reload: true }, layer)
    await refresh()
    toasts.push(
      'success',
      st('{layer} override cleared: {path}', { layer: layerOverrideLabel.value, path: DEFAULT_SELECTION_PATH }),
    )
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    saveError.value = message
    toasts.push('error', message)
  } finally {
    saveBusy.value = false
  }
}

watch(scope, () => syncEditor())
onMounted(() => void refresh())
</script>

<template>
  <div class="grid min-w-0 gap-4">
    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <label class="grid max-w-xl gap-1.5">
      <span class="text-xs font-medium">{{ $st('Model default source') }}</span>
      <OptionPicker
        :model-value="scope"
        :options="scopeOptions"
        :title="$st('Model default source')"
        :include-empty="false"
        @update:model-value="scope = $event as ModelDefaultScope"
      />
      <span class="break-words font-mono text-xs text-muted-foreground">{{ activeScopeOption.summary }}</span>
    </label>
    <div class="text-xs text-muted-foreground">{{ activeScopeOption.description }}</div>

    <section class="grid gap-4 rounded-lg border border-border/60 p-4">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 class="text-sm font-semibold">{{ $st('Default model') }}</h2>
          <p class="mt-1 max-w-3xl text-xs leading-5 text-muted-foreground">
            {{
              $st(
                'Used when a new run does not provide an explicit model. Provider settings do not define a separate default.',
              )
            }}
          </p>
        </div>
        <IconButton
          variant="outline"
          size="md"
          :tooltip="loading ? $st('Refreshing model settings') : $st('Refresh model settings')"
          :aria-label="loading ? $st('Refreshing model settings') : $st('Refresh model settings')"
          :disabled="loading || saveBusy"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </div>
      <div class="grid gap-3 xl:grid-cols-2">
        <label class="grid min-w-0 gap-1.5 xl:col-span-2">
          <span class="text-xs text-muted-foreground">{{ $st('Model route') }}</span>
          <OptionPicker
            :model-value="modelKey"
            :options="defaultModelOptions"
            :title="$st('Default model')"
            :placeholder="$st('Select a configured model')"
            :search-placeholder="$st('Search configured models...')"
            :include-empty="false"
            :disabled="loading || saveBusy || !editable"
            monospace
            @update:model-value="chooseDefaultModel"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Thinking') }}</span>
          <OptionPicker
            v-model="thinkingMode"
            :options="defaultThinkingOptions"
            :title="$st('Default thinking mode')"
            :empty-label="$st('Model default')"
            :disabled="loading || saveBusy || !editable || !modelKey"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Speed') }}</span>
          <OptionPicker
            v-model="speedMode"
            :options="defaultSpeedOptions"
            :title="$st('Default speed mode')"
            :empty-label="$st('Model default')"
            :disabled="loading || saveBusy || !editable || !modelKey"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Verbosity') }}</span>
          <OptionPicker
            v-model="verbosity"
            :options="defaultVerbosityOptions"
            :title="$st('Default verbosity')"
            :empty-label="$st('Model default')"
            :disabled="loading || saveBusy || !editable || !modelKey || defaultVerbosityOptions.length === 0"
          />
        </label>
        <label class="flex min-h-9 items-center gap-2 rounded-md border border-border/60 px-3 text-sm">
          <input
            v-model="parallelToolCalls"
            type="checkbox"
            :disabled="loading || saveBusy || !editable || !selectedSupportsParallelTools"
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
        <div v-else-if="saveError" class="break-words text-xs text-destructive">{{ saveError }}</div>
        <span v-else class="text-xs text-muted-foreground">
          {{ $st('Clear a mode to inherit the model’s native default.') }}
        </span>
        <div v-if="editable" class="flex items-center gap-2">
          <Button
            v-if="layerOverridePersisted"
            variant="outline"
            :disabled="loading || saveBusy"
            @click="clearLayerOverride"
          >
            <RiDeleteBinLine class="mr-2 h-4 w-4" />
            {{ $st('Clear {layer} override', { layer: layerOverrideLabel }) }}
          </Button>
          <Button :disabled="loading || saveBusy || !modelKey" @click="saveDefaultSelection">
            <RiSave3Line class="mr-2 h-4 w-4" />
            {{ saveBusy ? $st('Saving…') : $st('Save default model') }}
          </Button>
        </div>
      </div>
    </section>
  </div>
</template>
