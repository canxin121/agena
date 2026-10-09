<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import ApprovalModelPanel from '@/components/settings/ApprovalModelPanel.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import { mutateModelConfiguration } from '@/lib/modelConfigurationApi'
import { buildDefaultModelSettingsPatch, sameServerModelIdentity } from '@/lib/serverModelSettings'
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
import { settingsText as st } from '@/i18n/settingsText'

type RuntimeStatus = {
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

const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()

const loading = ref(false)
const error = ref('')
const runtime = ref<RuntimeStatus | null>(null)
const defaultModelKey = ref('')
const defaultThinkingMode = ref('')
const defaultSpeedMode = ref('')
const defaultVerbosity = ref('')
const defaultParallelToolCalls = ref(false)
const defaultSaveBusy = ref(false)
const defaultSaveError = ref('')

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

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const [runtimeData] = await Promise.all([
      apiJson<RuntimeStatus>('/api/v1/runtime'),
      modelSelectionCatalog.loadProvidersAndModels(),
    ])
    runtime.value = runtimeData && typeof runtimeData === 'object' ? runtimeData : null
    syncDefaultEditor()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
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
    await mutateModelConfiguration('/api/v1/settings', {
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

    <section class="grid gap-4 rounded-lg border border-border/60 p-4">
      <div>
        <h2 class="text-sm font-semibold">{{ $st('Runtime default model') }}</h2>
        <p class="mt-1 text-xs leading-5 text-muted-foreground">
          {{
            $st(
              'Used when a new run does not provide an explicit model. Provider settings do not define a separate default.',
            )
          }}
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
        <span v-else class="text-xs text-muted-foreground">{{
          $st('Clear a mode to inherit the model’s native default.')
        }}</span>
        <div class="flex items-center gap-2">
          <IconButton
            variant="outline"
            size="md"
            :tooltip="loading ? $st('Refreshing model settings') : $st('Refresh model settings')"
            :aria-label="loading ? $st('Refreshing model settings') : $st('Refresh model settings')"
            :disabled="loading || defaultSaveBusy"
            @click="refresh"
          >
            <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
          </IconButton>
          <Button :disabled="loading || defaultSaveBusy || !defaultModelKey" @click="saveDefaultSelection">
            <RiSave3Line class="mr-2 h-4 w-4" />
            {{ defaultSaveBusy ? $st('Saving…') : $st('Save runtime default') }}
          </Button>
        </div>
      </div>
    </section>

    <ApprovalModelPanel />
  </div>
</template>
