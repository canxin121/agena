<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RiCpuLine, RiRefreshLine, RiShieldCheckLine } from '@remixicon/vue'
import SettingsCard from './SettingsCard.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
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
  thinkingModeOptionsForModel,
  useModelSelectionCatalog,
  verbosityOptionsForModel,
  type ModelModeOption,
} from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{ scope: string; purpose: 'default' | 'approval' | 'auxiliary' }>()
const scope = computed(() => (props.scope === 'global' || props.scope === 'workspace' ? props.scope : 'effective'))
const approval = computed(() => props.purpose === 'approval')
const auxiliary = computed(() => props.purpose === 'auxiliary')
const path = computed(() =>
  auxiliary.value
    ? 'runtime.auxiliary_model'
    : approval.value
      ? 'permission.approval_model'
      : 'providers.default_selection',
)
const title = computed(() =>
  auxiliary.value ? st('Auxiliary model') : approval.value ? st('Automatic approval model') : st('Default model'),
)
const editable = computed(() => scope.value !== 'effective')
const catalog = useModelSelectionCatalog()
const toasts = useToastsStore()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const saveError = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const modelKey = ref('')
const thinkingMode = ref('')
const speedMode = ref('')
const verbosity = ref('')
const savedState = ref('')
const resetPending = ref(false)
let controller: AbortController | null = null
let disposed = false

function record(value: JsonValue | undefined): Record<string, JsonValue> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}
function fingerprint() {
  return JSON.stringify([modelKey.value, thinkingMode.value, speedMode.value, verbosity.value, resetPending.value])
}
const dirty = computed(() => editable.value && !!savedState.value && fingerprint() !== savedState.value)
const disabled = computed(() => !editable.value || loading.value || saving.value || !sources.value)
const persisted = computed(() => editable.value && hasPersistedSetting(sources.value?.[scope.value]))
const identity = computed(() => parseModelSlug(modelKey.value))
const selectedModel = computed(() =>
  catalog.modelMetaFor(identity.value.provider, identity.value.model, identity.value.adapter),
)
const modelOptions = computed(() => {
  const options = catalog.providers.value
    .flatMap((provider) =>
      provider.models.flatMap((model) => {
        const adapter = String(model.adapter_id || '').trim()
        const value = encodeModelSelectionKey({ provider: provider.id, adapter, model: model.id })
        return value
          ? [
              {
                value,
                label: String(model.display_name || model.id),
                description: [provider.id, adapter, model.id].filter(Boolean).join(' / '),
              },
            ]
          : []
      }),
    )
    .sort((a, b) => a.description.localeCompare(b.description))
  // An unavailable configured route remains visible and is never silently cleared.
  if (modelKey.value && !options.some((option) => option.value === modelKey.value)) {
    options.unshift({ value: modelKey.value, label: identity.value.model, description: st('Configured value') })
  }
  return options
})
function withSelected(options: ModelModeOption[], value: string) {
  return value && !options.some((option) => option.value === value)
    ? [...options, { value, label: value, description: st('Configured value'), isDefault: false }]
    : options
}
const thinkingOptions = computed(() =>
  withSelected(thinkingModeOptionsForModel(selectedModel.value), thinkingMode.value),
)
const speedOptions = computed(() => withSelected(speedModeOptionsForModel(selectedModel.value), speedMode.value))
const verbosityOptions = computed(() => withSelected(verbosityOptionsForModel(selectedModel.value), verbosity.value))
const modelName = computed(() => modelOptions.value.find((option) => option.value === modelKey.value)?.label || '')

function syncEditor() {
  const value = record(sources.value?.[scope.value]?.value)
  modelKey.value = encodeModelSelectionKey({
    provider: String(value.provider || ''),
    adapter: String(value.adapter || ''),
    model: String(value.model || ''),
  })
  thinkingMode.value = String(value.thinking_mode || '')
  speedMode.value = String(value.speed_mode || '')
  verbosity.value = String(value.verbosity || '')
  resetPending.value = false
  savedState.value = fingerprint()
  saveError.value = ''
}
function chooseModel(value: string) {
  modelKey.value = value
  resetPending.value = !value && persisted.value
  const model = selectedModel.value
  thinkingMode.value = value ? defaultModeValue(thinkingModeOptionsForModel(model)) : ''
  speedMode.value = value ? defaultModeValue(speedModeOptionsForModel(model)) : ''
  verbosity.value = value ? defaultModeValue(verbosityOptionsForModel(model)) : ''
  saveError.value = ''
}
async function refresh() {
  if (saving.value) return
  controller?.abort()
  const request = new AbortController()
  controller = request
  loading.value = true
  error.value = ''
  try {
    const [bundle] = await Promise.all([
      readRuntimeSettingSources(path.value, request.signal),
      catalog.loadProvidersAndModels(),
    ])
    if (disposed || request.signal.aborted) return
    sources.value = bundle
    if (!dirty.value) syncEditor()
  } catch (reason) {
    if (!disposed && !request.signal.aborted) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (controller === request) loading.value = false
  }
}
async function save() {
  const layer = scope.value
  if (disabled.value || layer === 'effective' || !dirty.value) return
  const selected = identity.value
  if (modelKey.value && (!selected.provider || !selected.model)) {
    saveError.value = st('Select a configured model.')
    return
  }
  saving.value = true
  saveError.value = ''
  controller?.abort()
  try {
    if (!modelKey.value) {
      await deleteRuntimeSetting(path.value, { reload: true }, layer)
    } else {
      const value = { ...record(sources.value?.[layer]?.value) }
      for (const key of ['provider', 'adapter', 'model', 'thinking_mode', 'speed_mode', 'verbosity']) delete value[key]
      Object.assign(
        value,
        { provider: selected.provider, model: selected.model },
        selected.adapter ? { adapter: selected.adapter } : {},
        thinkingMode.value ? { thinking_mode: thinkingMode.value } : {},
        speedMode.value ? { speed_mode: speedMode.value } : {},
        verbosity.value ? { verbosity: verbosity.value } : {},
      )
      await setRuntimeSetting(path.value, value, { reload: true }, layer)
    }
    const bundle = await readRuntimeSettingSources(path.value)
    if (disposed) return
    if (
      modelKey.value ? !sameServerModelIdentity(bundle[layer]?.value, selected) : hasPersistedSetting(bundle[layer])
    ) {
      throw new Error(st('The server did not apply the model selection. Your draft has been kept.'))
    }
    sources.value = bundle
    syncEditor()
    toasts.push('success', st('Model settings saved'))
  } catch (reason) {
    if (!disposed) saveError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}
watch(scope, syncEditor)
onMounted(() => void refresh())
onBeforeUnmount(() => {
  disposed = true
  controller?.abort()
})
defineExpose({ refresh, dirty, saving, discard: syncEditor })
</script>

<template>
  <SettingsCard
    :title="title"
    :icon="approval ? RiShieldCheckLine : RiCpuLine"
    :description="
      auxiliary
        ? $st('Used for short background tasks such as session naming. When unset, the session model is used.')
        : approval
          ? $st('Used to review permission requests in Auto mode. When unset, the session model reviews them.')
          : $st('Choose the model used for new runs when no model is explicitly selected.')
    "
    :data-model-selection="purpose"
    :aria-busy="loading || saving"
  >
    <template #actions>
      <div class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground" role="status">
        <span>{{
          loading
            ? $st('Loading…')
            : !editable
              ? $st('Read-only')
              : dirty
                ? $st('You have unsaved changes.')
                : $st('All changes are saved.')
        }}</span>
        <IconButton
          variant="ghost"
          size="sm"
          :disabled="loading || saving"
          :tooltip="$st('Refresh model settings')"
          :aria-label="$st('Refresh model settings')"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </div>
    </template>
    <div class="grid min-w-0 gap-4">
      <div
        v-if="error || catalog.catalogError.value"
        class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
        role="alert"
      >
        {{ error || catalog.catalogError.value }}
      </div>
      <div class="grid min-w-0 gap-4">
        <div v-if="editable" class="grid min-w-0 gap-2">
          <label class="grid min-w-0 gap-1.5">
            <span class="text-xs font-medium">{{ $st('Model route') }}</span>
            <OptionPicker
              :model-value="modelKey"
              :options="modelOptions"
              :title="title"
              :empty-label="$st('Inherit model selection')"
              :search-placeholder="$st('Search configured models...')"
              :disabled="disabled"
              @update:model-value="chooseModel"
            />
          </label>
          <p class="text-xs leading-relaxed text-muted-foreground">
            {{
              modelKey
                ? $st('This selection applies only to the selected configuration scope.')
                : $st('No override in this scope. The inherited model selection is used.')
            }}
          </p>
        </div>
        <div v-else class="text-base font-medium">
          {{ modelName || (approval ? $st('Use the session model') : $st('No default model configured')) }}
        </div>
        <dl v-if="modelKey" class="grid min-w-0 gap-3 rounded-lg bg-muted/25 p-3 sm:grid-cols-3">
          <div
            v-for="item in [
              { label: $st('Provider'), value: identity.provider },
              { label: $st('Adapter'), value: identity.adapter },
              { label: $st('Model ID'), value: identity.model },
            ]"
            :key="item.label"
            class="min-w-0"
          >
            <dt class="text-xs text-muted-foreground">{{ item.label }}</dt>
            <dd class="mt-1 break-all font-mono text-xs leading-5">{{ item.value || '—' }}</dd>
          </div>
        </dl>
        <p v-if="!loading && !modelOptions.length" class="text-xs leading-relaxed text-muted-foreground">
          {{ $st('Add a provider and enable its models before choosing a model here.') }}
        </p>
        <div
          v-if="editable && persisted"
          class="flex flex-wrap items-center justify-between gap-2 border-t border-border/50 pt-3"
        >
          <span class="text-xs text-muted-foreground">{{
            resetPending ? $st('Save to remove this scope’s override.') : $st('This scope has its own model selection.')
          }}</span>
          <Button variant="ghost" size="sm" :disabled="disabled || resetPending" @click="chooseModel('')">{{
            approval ? $st('Clear approval model') : $st('Reset to inherited')
          }}</Button>
        </div>
      </div>
      <section class="grid min-w-0 gap-4 border-t border-border/60 pt-4" :aria-label="$st('Response preferences')">
        <div class="space-y-1">
          <h3 class="text-sm font-semibold">{{ $st('Response preferences') }}</h3>
          <p class="text-xs leading-relaxed text-muted-foreground">
            {{ $st('Empty mode fields use the selected model’s defaults. Available options depend on the model.') }}
          </p>
        </div>
        <div class="grid min-w-0 gap-4 md:grid-cols-3">
          <label class="grid min-w-0 content-start gap-2">
            <span class="text-sm font-medium">{{ $st('Thinking') }}</span>
            <OptionPicker
              v-model="thinkingMode"
              :options="thinkingOptions"
              :title="$st('Thinking')"
              :empty-label="$st('Model default')"
              :disabled="disabled || !modelKey"
            />
            <span class="text-xs leading-relaxed text-muted-foreground">{{
              $st('How much reasoning the model uses before responding.')
            }}</span>
          </label>
          <label class="grid min-w-0 content-start gap-2">
            <span class="text-sm font-medium">{{ $st('Speed') }}</span>
            <OptionPicker
              v-model="speedMode"
              :options="speedOptions"
              :title="$st('Speed')"
              :empty-label="$st('Model default')"
              :disabled="disabled || !modelKey"
            />
            <span class="text-xs leading-relaxed text-muted-foreground">{{
              $st('Choose a response speed supported by this model.')
            }}</span>
          </label>
          <label class="grid min-w-0 content-start gap-2">
            <span class="text-sm font-medium">{{ $st('Verbosity') }}</span>
            <OptionPicker
              v-model="verbosity"
              :options="verbosityOptions"
              :title="$st('Verbosity')"
              :empty-label="$st('Model default')"
              :disabled="disabled || !modelKey || !verbosityOptions.length"
            />
            <span class="text-xs leading-relaxed text-muted-foreground">{{
              $st('Control the level of detail in the model’s response.')
            }}</span>
          </label>
        </div>
      </section>
      <SettingsSaveBar
        v-if="dirty || saving"
        :dirty="dirty"
        :saving="saving"
        :disabled="loading || !sources"
        :error="saveError"
        :save-label="approval ? $st('Save approval model') : $st('Save default model')"
        @save="save"
        @discard="syncEditor"
      />
    </div>
  </SettingsCard>
</template>
