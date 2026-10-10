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

const APPROVAL_MODEL_PATH = 'permission.approval_model'

const props = withDefaults(
  defineProps<{
    scope?: 'effective' | 'global' | 'workspace'
  }>(),
  { scope: 'effective' },
)

const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const modelKey = ref('')
const thinkingMode = ref('')
const speedMode = ref('')
const verbosity = ref('')
const parallelToolCalls = ref(false)

const editable = computed(() => props.scope !== 'effective')
const selectedLayer = computed<'global' | 'workspace' | null>(() =>
  editable.value ? (props.scope as 'global' | 'workspace') : null,
)
function selectionValue(layer: 'effective' | 'global' | 'workspace'): JsonValue | undefined {
  return sources.value?.[layer]?.value
}

const layerOverridePersisted = computed(() => {
  if (!editable.value) return false
  return hasPersistedSetting(props.scope === 'workspace' ? sources.value?.workspace : sources.value?.global)
})

const modelOptions = computed(() => {
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
const selectedIdentity = computed(() => parseModelSlug(modelKey.value))
const selectedModel = computed(() => {
  const selection = selectedIdentity.value
  return modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
})

function withSelectedMode(options: ModelModeOption[], selected: string): ModelModeOption[] {
  const value = String(selected || '').trim()
  if (!value || options.some((option) => option.value === value)) return options
  return [...options, { value, label: value, description: st('Configured value'), isDefault: false }]
}

const thinkingOptions = computed(() =>
  withSelectedMode(thinkingModeOptionsForModel(selectedModel.value), thinkingMode.value),
)
const speedOptions = computed(() => withSelectedMode(speedModeOptionsForModel(selectedModel.value), speedMode.value))
const verbosityOptions = computed(() =>
  withSelectedMode(verbosityOptionsForModel(selectedModel.value), verbosity.value),
)
const supportsParallelTools = computed(() => supportsParallelToolCallsForModel(selectedModel.value))

function chooseModel(value: string) {
  modelKey.value = value
  const identity = parseModelSlug(value)
  const model = modelSelectionCatalog.modelMetaFor(identity.provider, identity.model, identity.adapter)
  thinkingMode.value = defaultModeValue(thinkingModeOptionsForModel(model))
  speedMode.value = defaultModeValue(speedModeOptionsForModel(model))
  verbosity.value = defaultModeValue(verbosityOptionsForModel(model))
  parallelToolCalls.value = false
  error.value = ''
}

function syncEditor() {
  const value = selectionValue(props.scope)
  const record = value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, JsonValue>) : {}
  const provider = String(record.provider || '').trim()
  const model = String(record.model || '').trim()
  modelKey.value =
    provider && model ? encodeModelSelectionKey({ provider, adapter: String(record.adapter || '').trim(), model }) : ''
  thinkingMode.value = String(record.thinking_mode || '').trim()
  speedMode.value = String(record.speed_mode || '').trim()
  verbosity.value = String(record.verbosity || '').trim()
  parallelToolCalls.value = record.parallel_tool_calls === true
  error.value = ''
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const [bundle] = await Promise.all([
      readRuntimeSettingSources(APPROVAL_MODEL_PATH),
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

async function save() {
  const layer = selectedLayer.value
  if (saving.value) return
  const desired = selectedIdentity.value
  const hasSelection = Boolean(desired.provider && desired.model)
  const desiredThinking = hasSelection ? thinkingMode.value.trim() : ''
  const desiredSpeed = hasSelection ? speedMode.value.trim() : ''
  const desiredVerbosity = hasSelection ? verbosity.value.trim() : ''
  const desiredParallel = hasSelection && supportsParallelTools.value ? parallelToolCalls.value : undefined
  saving.value = true
  error.value = ''
  try {
    if (!hasSelection) {
      if (!layer || !layerOverridePersisted.value) {
        error.value = st('Select a configured model.')
        return
      }
      await deleteRuntimeSetting(APPROVAL_MODEL_PATH, { reload: true }, layer)
    } else {
      if (!layer) return
      const selection: Record<string, JsonValue> = {
        provider: desired.provider,
        ...(desired.adapter ? { adapter: desired.adapter } : {}),
        model: desired.model,
        ...(desiredThinking ? { thinking_mode: desiredThinking } : {}),
        ...(desiredSpeed ? { speed_mode: desiredSpeed } : {}),
        ...(desiredVerbosity ? { verbosity: desiredVerbosity } : {}),
        ...(typeof desiredParallel === 'boolean' ? { parallel_tool_calls: desiredParallel } : {}),
      }
      await setRuntimeSetting(APPROVAL_MODEL_PATH, selection, { reload: true }, layer)
    }
    await refresh()
    const applied = layer && hasSelection ? selectionValue(layer) : undefined
    if (hasSelection && !sameServerModelIdentity(applied, desired)) {
      throw new Error(st('The server accepted the update but did not apply the automatic approval model.'))
    }
    toasts.push(
      'success',
      hasSelection ? st('Automatic approval model updated') : st('Automatic approval model cleared'),
    )
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    error.value = message
    toasts.push('error', message)
  } finally {
    saving.value = false
  }
}

watch(
  () => props.scope,
  () => syncEditor(),
)
onMounted(() => void refresh())
</script>

<template>
  <section class="grid gap-4 rounded-lg border border-border/60 p-4">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h3 class="text-sm font-semibold">{{ $st('Automatic approval model') }}</h3>
        <p class="mt-1 max-w-3xl text-xs leading-5 text-muted-foreground">
          {{ $st('Used to review permission requests in Auto mode. When unset, the session model reviews them.') }}
        </p>
      </div>
      <IconButton
        variant="ghost"
        size="sm"
        :disabled="loading || saving"
        :tooltip="$st('Reload approval model')"
        :aria-label="$st('Reload approval model')"
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
          :options="modelOptions"
          :title="$st('Automatic approval model')"
          :empty-label="$st('No dedicated approval model')"
          :placeholder="$st('Select a configured model')"
          :search-placeholder="$st('Search configured models...')"
          :disabled="loading || saving || !editable"
          monospace
          @update:model-value="chooseModel"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">{{ $st('Thinking') }}</span>
        <OptionPicker
          v-model="thinkingMode"
          :options="thinkingOptions"
          :title="$st('Approval thinking mode')"
          :empty-label="$st('Model default')"
          :disabled="loading || saving || !editable || !modelKey"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">{{ $st('Speed') }}</span>
        <OptionPicker
          v-model="speedMode"
          :options="speedOptions"
          :title="$st('Approval speed mode')"
          :empty-label="$st('Model default')"
          :disabled="loading || saving || !editable || !modelKey"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">{{ $st('Verbosity') }}</span>
        <OptionPicker
          v-model="verbosity"
          :options="verbosityOptions"
          :title="$st('Approval verbosity')"
          :empty-label="$st('Model default')"
          :disabled="loading || saving || !editable || !modelKey || verbosityOptions.length === 0"
        />
      </label>
      <label class="flex min-h-9 items-center gap-2 rounded-md border border-border/60 px-3 text-sm">
        <input
          v-model="parallelToolCalls"
          type="checkbox"
          :disabled="loading || saving || !editable || !supportsParallelTools"
        />
        <span>
          {{ $st('Parallel tool calls') }}
          <span v-if="!supportsParallelTools" class="ml-1 text-xs text-muted-foreground">
            {{ $st('not supported') }}
          </span>
        </span>
      </label>
    </div>

    <div class="flex flex-wrap items-center justify-between gap-3">
      <div v-if="error" class="break-words text-xs text-destructive">{{ error }}</div>
      <span v-else class="text-xs text-muted-foreground">
        {{ $st('All empty mode fields inherit the selected model defaults.') }}
      </span>
      <div v-if="editable" class="flex items-center gap-2">
        <Button v-if="layerOverridePersisted" variant="outline" :disabled="loading || saving" @click="modelKey = ''">
          <RiDeleteBinLine class="mr-2 h-4 w-4" />
          {{ $st('Clear approval model') }}
        </Button>
        <Button :disabled="loading || saving" @click="save">
          <RiSave3Line class="mr-2 h-4 w-4" />
          {{ saving ? $st('Saving…') : $st('Save approval model') }}
        </Button>
      </div>
    </div>
  </section>
</template>
