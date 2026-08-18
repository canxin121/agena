<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import {
  approvalModelFromSettingsResponse,
  buildApprovalModelSettingsPatch,
  sameServerModelIdentity,
} from '@/lib/serverModelSettings'
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

const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const modelKey = ref('')
const thinkingMode = ref('')
const speedMode = ref('')
const verbosity = ref('')
const parallelToolCalls = ref(false)

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
  return [...options, { value, label: value, description: 'Configured value', isDefault: false }]
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

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const [settingsResponse] = await Promise.all([
      apiJson<JsonValue>('/api/v1/settings?source=effective&path=permission.approval_model'),
      modelSelectionCatalog.loadProvidersAndModels(),
    ])
    const approval = approvalModelFromSettingsResponse(settingsResponse)
    modelKey.value = approval
      ? encodeModelSelectionKey({
          provider: approval.identity.provider,
          adapter: approval.identity.adapter,
          model: approval.identity.model,
        })
      : ''
    thinkingMode.value = approval?.modes.thinkingMode || ''
    speedMode.value = approval?.modes.speedMode || ''
    verbosity.value = approval?.modes.verbosity || ''
    parallelToolCalls.value = approval?.modes.parallelToolCalls === true
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

async function save() {
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
    await apiJson('/api/v1/settings', {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(
        buildApprovalModelSettingsPatch(hasSelection ? desired : null, {
          ...(desiredThinking ? { thinkingMode: desiredThinking } : {}),
          ...(desiredSpeed ? { speedMode: desiredSpeed } : {}),
          ...(desiredVerbosity ? { verbosity: desiredVerbosity } : {}),
          ...(typeof desiredParallel === 'boolean' ? { parallelToolCalls: desiredParallel } : {}),
        }),
      ),
    })
    await refresh()
    const applied = selectedIdentity.value
    if (
      hasSelection !== Boolean(applied.provider && applied.model) ||
      (hasSelection && !sameServerModelIdentity(applied, desired)) ||
      (hasSelection && thinkingMode.value.trim() !== desiredThinking) ||
      (hasSelection && speedMode.value.trim() !== desiredSpeed) ||
      (hasSelection && verbosity.value.trim() !== desiredVerbosity) ||
      (typeof desiredParallel === 'boolean' && parallelToolCalls.value !== desiredParallel)
    ) {
      throw new Error('The server accepted the update but did not apply the automatic approval model.')
    }
    toasts.push('success', hasSelection ? 'Automatic approval model updated' : 'Automatic approval model cleared')
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    error.value = message
    toasts.push('error', message)
  } finally {
    saving.value = false
  }
}

onMounted(() => void refresh())
</script>

<template>
  <section class="grid gap-4 rounded-lg border border-border/60 p-4">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h3 class="text-sm font-semibold">Automatic approval model</h3>
        <p class="mt-1 max-w-3xl text-xs leading-5 text-muted-foreground">
          Used to classify permission requests in Auto mode. Clearing it makes the runtime fail closed to its normal
          approval fallback path.
        </p>
      </div>
      <IconButton
        variant="ghost"
        size="sm"
        :disabled="loading || saving"
        tooltip="Reload approval model"
        aria-label="Reload approval model"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div class="grid gap-3 xl:grid-cols-2">
      <label class="grid min-w-0 gap-1.5 xl:col-span-2">
        <span class="text-xs text-muted-foreground">Model route</span>
        <OptionPicker
          :model-value="modelKey"
          :options="modelOptions"
          title="Automatic approval model"
          empty-label="No dedicated approval model"
          placeholder="Select a configured model"
          search-placeholder="Search configured models..."
          :disabled="loading || saving"
          monospace
          @update:model-value="chooseModel"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">Thinking</span>
        <OptionPicker
          v-model="thinkingMode"
          :options="thinkingOptions"
          title="Approval thinking mode"
          empty-label="Model default"
          :disabled="loading || saving || !modelKey"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">Speed</span>
        <OptionPicker
          v-model="speedMode"
          :options="speedOptions"
          title="Approval speed mode"
          empty-label="Model default"
          :disabled="loading || saving || !modelKey"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">Verbosity</span>
        <OptionPicker
          v-model="verbosity"
          :options="verbosityOptions"
          title="Approval verbosity"
          empty-label="Model default"
          :disabled="loading || saving || !modelKey || verbosityOptions.length === 0"
        />
      </label>
      <label class="flex min-h-9 items-center gap-2 rounded-md border border-border/60 px-3 text-sm">
        <input v-model="parallelToolCalls" type="checkbox" :disabled="loading || saving || !supportsParallelTools" />
        <span>
          Parallel tool calls
          <span v-if="!supportsParallelTools" class="ml-1 text-xs text-muted-foreground">not supported</span>
        </span>
      </label>
    </div>

    <div class="flex flex-wrap items-center justify-between gap-3">
      <div v-if="error" class="break-words text-xs text-destructive">{{ error }}</div>
      <span v-else class="text-xs text-muted-foreground"
        >All empty mode fields inherit the selected model defaults.</span
      >
      <Button :disabled="loading || saving" @click="save">
        <RiSave3Line class="mr-2 h-4 w-4" />
        {{ saving ? 'Saving…' : modelKey ? 'Save approval model' : 'Clear approval model' }}
      </Button>
    </div>
  </section>
</template>
