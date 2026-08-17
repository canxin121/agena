<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
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
  thinkingModeOptionsForModel,
  useModelSelectionCatalog,
  type ModelModeOption,
} from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'

const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()

const approvalLoading = ref(false)
const approvalSaveBusy = ref(false)
const approvalError = ref('')
const approvalModelKey = ref('')
const approvalThinkingMode = ref('')
const approvalSpeedMode = ref('')

const approvalModelOptions = computed(() => {
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

const selectedApprovalIdentity = computed(() => parseModelSlug(approvalModelKey.value))
const selectedApprovalModel = computed(() => {
  const selection = selectedApprovalIdentity.value
  return modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
})

function withSelectedMode(options: ModelModeOption[], selected: string): ModelModeOption[] {
  const value = String(selected || '').trim()
  if (!value || options.some((option) => option.value === value)) return options
  return [...options, { value, label: value, description: '', isDefault: false }]
}

const approvalThinkingOptions = computed(() =>
  withSelectedMode(thinkingModeOptionsForModel(selectedApprovalModel.value), approvalThinkingMode.value),
)
const approvalSpeedOptions = computed(() =>
  withSelectedMode(speedModeOptionsForModel(selectedApprovalModel.value), approvalSpeedMode.value),
)

function chooseApprovalModel(value: string) {
  approvalModelKey.value = value
  const selection = parseModelSlug(value)
  const model = modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
  approvalThinkingMode.value = defaultModeValue(thinkingModeOptionsForModel(model))
  approvalSpeedMode.value = defaultModeValue(speedModeOptionsForModel(model))
  approvalError.value = ''
}

async function refreshApprovalModel() {
  approvalLoading.value = true
  approvalError.value = ''
  try {
    const [settingsResponse] = await Promise.all([
      apiJson<JsonValue>('/api/v1/settings?source=effective&path=permission.approval_model'),
      modelSelectionCatalog.loadProvidersAndModels(),
    ])
    const approval = approvalModelFromSettingsResponse(settingsResponse)
    approvalModelKey.value = approval
      ? encodeModelSelectionKey({
          provider: approval.identity.provider,
          adapter: approval.identity.adapter,
          model: approval.identity.model,
        })
      : ''
    approvalThinkingMode.value = approval?.modes.thinkingMode || ''
    approvalSpeedMode.value = approval?.modes.speedMode || ''
  } catch (reason) {
    approvalError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    approvalLoading.value = false
  }
}

async function saveApprovalModel() {
  if (approvalSaveBusy.value) return
  const desired = selectedApprovalIdentity.value
  const hasSelection = Boolean(desired.provider && desired.model)
  const desiredThinkingMode = hasSelection ? approvalThinkingMode.value.trim() : ''
  const desiredSpeedMode = hasSelection ? approvalSpeedMode.value.trim() : ''
  approvalSaveBusy.value = true
  approvalError.value = ''
  try {
    await apiJson('/api/v1/settings', {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(
        buildApprovalModelSettingsPatch(hasSelection ? desired : null, {
          ...(desiredThinkingMode ? { thinkingMode: desiredThinkingMode } : {}),
          ...(desiredSpeedMode ? { speedMode: desiredSpeedMode } : {}),
        }),
      ),
    })
    await refreshApprovalModel()
    const applied = selectedApprovalIdentity.value
    if (
      hasSelection !== Boolean(applied.provider && applied.model) ||
      (hasSelection && !sameServerModelIdentity(applied, desired)) ||
      (hasSelection && approvalThinkingMode.value.trim() !== desiredThinkingMode) ||
      (hasSelection && approvalSpeedMode.value.trim() !== desiredSpeedMode)
    ) {
      throw new Error('The server accepted the update but did not apply the automatic approval model.')
    }
    toasts.push('success', hasSelection ? 'Automatic approval model updated' : 'Automatic approval model cleared')
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    approvalError.value = message
    toasts.push('error', message)
  } finally {
    approvalSaveBusy.value = false
  }
}

onMounted(() => void refreshApprovalModel())
</script>

<template>
  <section class="grid gap-3 border-b border-border/60 py-4">
    <div>
      <h2 class="text-sm font-medium">Automatic approval model</h2>
      <p class="mt-1 text-xs text-muted-foreground">
        Used only to classify permission requests in Auto mode. Without a dedicated model, Agena falls back to the run
        or runtime default model.
      </p>
    </div>
    <div class="grid gap-3 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)_auto] lg:items-end">
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">Model</span>
        <OptionPicker
          :model-value="approvalModelKey"
          :options="approvalModelOptions"
          title="Automatic approval model"
          empty-label="Use run/default model"
          placeholder="Select a configured model"
          search-placeholder="Search configured models..."
          :include-empty="true"
          :disabled="approvalLoading || approvalSaveBusy"
          monospace
          @update:model-value="chooseApprovalModel"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">Thinking</span>
        <OptionPicker
          v-model="approvalThinkingMode"
          :options="approvalThinkingOptions"
          title="Approval thinking mode"
          empty-label="Model default"
          :include-empty="true"
          :disabled="approvalLoading || approvalSaveBusy || !approvalModelKey"
        />
      </label>
      <label class="grid min-w-0 gap-1.5">
        <span class="text-xs text-muted-foreground">Speed</span>
        <OptionPicker
          v-model="approvalSpeedMode"
          :options="approvalSpeedOptions"
          title="Approval speed mode"
          empty-label="Model default"
          :include-empty="true"
          :disabled="approvalLoading || approvalSaveBusy || !approvalModelKey"
        />
      </label>
      <Button class="h-10" :disabled="approvalLoading || approvalSaveBusy" @click="saveApprovalModel">
        <RiSave3Line class="mr-2 h-4 w-4" />
        {{ approvalSaveBusy ? 'Saving...' : 'Save model' }}
      </Button>
    </div>
    <div v-if="modelSelectionCatalog.catalogError.value" class="break-words text-xs text-destructive">
      {{ modelSelectionCatalog.catalogError.value }}
    </div>
    <div v-if="approvalError" class="break-words text-xs text-destructive">{{ approvalError }}</div>
  </section>
</template>
