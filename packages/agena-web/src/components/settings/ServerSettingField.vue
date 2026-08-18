<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RiDeleteBinLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import {
  deleteRuntimeSetting,
  displayJsonValue,
  hasPersistedSetting,
  readRuntimeSettingSources,
  setRuntimeSetting,
  settingValue,
  type RuntimeSettingReadResponse,
  type RuntimeSettingsLayer,
  type RuntimeSettingsReadBundle,
} from '@/lib/runtimeSettings'
import type { JsonValue } from '@/types/json'

type SettingKind = 'text' | 'number' | 'boolean' | 'select'
type SettingOption = { value: string; label: string; description?: string }

const booleanOptions: SettingOption[] = [
  { value: 'true', label: 'Enabled' },
  { value: 'false', label: 'Disabled' },
]

const props = withDefaults(
  defineProps<{
    path: string
    label: string
    description?: string
    kind?: SettingKind
    options?: SettingOption[]
    defaultValue?: string | number | boolean
    placeholder?: string
    monospace?: boolean
    allowCustom?: boolean
    includeEmpty?: boolean
    emptyLabel?: string
    targetLayer?: RuntimeSettingsLayer
    reload?: boolean
    disabled?: boolean
    compact?: boolean
  }>(),
  {
    description: '',
    kind: 'text',
    options: () => [],
    defaultValue: '',
    placeholder: '',
    monospace: false,
    allowCustom: false,
    includeEmpty: false,
    emptyLabel: 'No value',
    targetLayer: 'global',
    reload: true,
    disabled: false,
    compact: false,
  },
)

const emit = defineEmits<{
  (event: 'saved', value: JsonValue): void
  (event: 'error', message: string): void
  (event: 'loaded', sources: RuntimeSettingsReadBundle): void
}>()

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const localValue = ref<string | number | boolean>(props.defaultValue)

const selectOptions = computed<SettingOption[]>(() => {
  if (props.kind !== 'select') return props.options
  const current = String(localValue.value || '').trim()
  if (!current || props.options.some((option) => option.value === current)) return props.options
  // Keep a value from an older/unavailable plugin visible and selectable. It
  // can still be saved unchanged while the user decides whether to replace
  // it, instead of silently making the field look unset.
  return [{ value: current, label: current, description: 'Current configured value' }, ...props.options]
})

const busy = computed(() => loading.value || saving.value || props.disabled)
const effectiveResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.effective || null)
const globalResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.global || null)
const workspaceResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.workspace || null)
const hasOverride = computed(() =>
  hasPersistedSetting(props.targetLayer === 'workspace' ? workspaceResponse.value : globalResponse.value),
)

function normalizeLocal(value: JsonValue): string | number | boolean {
  if (props.kind === 'boolean') {
    if (value === true) return true
    if (value === false) return false
    return props.defaultValue === '' ? '' : props.defaultValue === true
  }
  if (props.kind === 'number') {
    const numeric = typeof value === 'number' ? value : Number(value)
    return Number.isFinite(numeric) ? numeric : Number(props.defaultValue) || 0
  }
  return typeof value === 'string' ? value : value === null || value === undefined ? '' : String(value)
}

function syncLocal() {
  const effective = settingValue(effectiveResponse.value, props.defaultValue)
  localValue.value = normalizeLocal(effective as JsonValue)
}

async function refresh() {
  if (!props.path.trim()) return
  loading.value = true
  error.value = ''
  try {
    const next = await readRuntimeSettingSources(props.path)
    sources.value = next
    syncLocal()
    emit('loaded', next)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    emit('error', error.value)
  } finally {
    loading.value = false
  }
}

function serializedValue(): JsonValue {
  if (props.kind === 'boolean') return localValue.value === true
  if (props.kind === 'number') {
    const numeric = Number(localValue.value)
    return Number.isFinite(numeric) ? numeric : 0
  }
  return String(localValue.value || '')
}

async function save() {
  if (busy.value) return
  saving.value = true
  error.value = ''
  try {
    // An empty value in a TUI select means "unset". Persisting an empty
    // string would create an invalid/meaningless override for optional
    // settings such as ui.tui.theme, so remove the selected layer entry.
    const clearSelectedValue =
      (props.kind === 'select' && props.includeEmpty && !String(localValue.value || '').trim()) ||
      (props.kind === 'boolean' && props.includeEmpty && localValue.value === '')
    if (clearSelectedValue) {
      await deleteRuntimeSetting(props.path, { reload: props.reload }, props.targetLayer)
      await refresh()
      emit('saved', settingValue(effectiveResponse.value, props.defaultValue) as JsonValue)
      return
    }
    const value = serializedValue()
    await setRuntimeSetting(props.path, value, { reload: props.reload }, props.targetLayer)
    await refresh()
    emit('saved', value)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    emit('error', error.value)
  } finally {
    saving.value = false
  }
}

async function clearOverride() {
  if (busy.value || !hasOverride.value) return
  saving.value = true
  error.value = ''
  try {
    await deleteRuntimeSetting(props.path, { reload: props.reload }, props.targetLayer)
    await refresh()
    emit('saved', settingValue(effectiveResponse.value, props.defaultValue) as JsonValue)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    emit('error', error.value)
  } finally {
    saving.value = false
  }
}

function onNumberInput(event: Event) {
  const target = event.target as HTMLInputElement
  localValue.value = target.value === '' ? 0 : Number(target.value)
}

function onTextInput(event: Event) {
  localValue.value = (event.target as HTMLInputElement).value
}

function onBooleanSelect(value: string) {
  localValue.value = value === 'true' ? true : value === 'false' ? false : ''
}

function effectiveLabel(response: RuntimeSettingReadResponse | null): string {
  return displayJsonValue(response?.value)
}

watch(
  () => props.path,
  () => void refresh(),
)
watch(
  () => props.defaultValue,
  () => {
    if (!sources.value) localValue.value = props.defaultValue
  },
)

onMounted(() => void refresh())
</script>

<template>
  <div
    :class="
      compact
        ? 'grid gap-2 border-b border-border/50 py-3 last:border-b-0'
        : 'grid gap-3 rounded-lg border border-border/60 bg-background/50 p-4'
    "
  >
    <div class="flex min-w-0 items-start justify-between gap-3">
      <div class="min-w-0">
        <div class="font-medium">{{ label }}</div>
        <div v-if="description" class="mt-1 text-xs leading-5 text-muted-foreground">{{ description }}</div>
        <code class="mt-1 block break-all font-mono text-[10px] text-muted-foreground/80">{{ path }}</code>
      </div>
      <IconButton
        variant="ghost"
        size="sm"
        :tooltip="loading ? 'Refreshing setting' : 'Refresh setting'"
        :aria-label="loading ? 'Refreshing setting' : 'Refresh setting'"
        :disabled="loading || saving || disabled"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div class="flex min-w-0 flex-wrap items-end gap-2">
      <label v-if="kind === 'boolean' && !includeEmpty" class="inline-flex min-h-9 items-center gap-2 text-sm">
        <input v-model="localValue" type="checkbox" :disabled="busy" />
        <span>{{ localValue ? 'Enabled' : 'Disabled' }}</span>
      </label>
      <OptionPicker
        v-else-if="kind === 'boolean' && includeEmpty"
        :model-value="localValue === true ? 'true' : localValue === false ? 'false' : ''"
        :options="booleanOptions"
        :title="label"
        :placeholder="placeholder || 'Select value'"
        :search-placeholder="placeholder || label"
        :include-empty="true"
        :empty-label="emptyLabel"
        :disabled="busy"
        @update:model-value="onBooleanSelect"
      />
      <OptionPicker
        v-else-if="kind === 'select'"
        :model-value="String(localValue)"
        :options="selectOptions"
        :title="label"
        :placeholder="placeholder || 'Select value'"
        :search-placeholder="placeholder || label"
        :include-empty="includeEmpty"
        :empty-label="emptyLabel"
        :allow-custom="allowCustom"
        :disabled="busy"
        :monospace="monospace"
        class="min-w-[14rem] flex-1"
        @update:model-value="localValue = $event"
      />
      <input
        v-else-if="kind === 'number'"
        :value="localValue"
        type="number"
        :placeholder="placeholder"
        :disabled="busy"
        class="h-9 min-w-[10rem] flex-1 rounded-md border border-input bg-transparent px-3 text-sm outline-none focus:border-ring"
        @input="onNumberInput"
      />
      <input
        v-else
        :value="localValue"
        type="text"
        :placeholder="placeholder"
        :disabled="busy"
        :class="[
          'h-9 min-w-[14rem] flex-1 rounded-md border border-input bg-transparent px-3 text-sm outline-none focus:border-ring',
          monospace ? 'font-mono' : '',
        ]"
        @input="onTextInput"
      />
      <Button size="sm" :disabled="busy" @click="save">
        <RiSave3Line class="mr-1.5 h-4 w-4" />
        {{ saving ? 'Saving…' : 'Save' }}
      </Button>
      <Button
        v-if="hasOverride"
        variant="ghost"
        size="sm"
        :disabled="busy"
        :title="`Clear ${targetLayer} override`"
        @click="clearOverride"
      >
        <RiDeleteBinLine class="mr-1.5 h-4 w-4" />
        Clear
      </Button>
    </div>

    <div class="grid gap-x-4 gap-y-1 text-[11px] text-muted-foreground sm:grid-cols-3">
      <div>
        <span class="font-medium text-foreground/80">Effective:</span>
        <code class="break-all">{{ effectiveLabel(effectiveResponse) }}</code>
      </div>
      <div>
        <span class="font-medium text-foreground/80">Global layer:</span>
        <code class="break-all">{{ effectiveLabel(globalResponse) }}</code>
      </div>
      <div>
        <span class="font-medium text-foreground/80">Workspace layer:</span>
        <code class="break-all">{{ effectiveLabel(workspaceResponse) }}</code>
      </div>
    </div>
    <div v-if="error" class="break-words text-xs text-destructive">{{ error }}</div>
  </div>
</template>
