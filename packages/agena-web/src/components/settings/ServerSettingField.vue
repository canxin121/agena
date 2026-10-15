<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RiCheckboxCircleLine, RiDeleteBinLine, RiLoader4Line, RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import SettingsToggle from './SettingsToggle.vue'
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
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneVisibility } from '@/composables/usePaneVisibility'

type SettingKind = 'text' | 'number' | 'boolean' | 'select'
type SettingOption = { value: string; label: string; description?: string }

const booleanOptions = computed<SettingOption[]>(() => [
  { value: 'true', label: st('Enabled') },
  { value: 'false', label: st('Disabled') },
])

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
    emptyLabel: '',
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
const savedValue = ref<string | number | boolean>(props.defaultValue)
const visible = usePaneVisibility()
let autoSaveTimer: ReturnType<typeof setTimeout> | null = null
let readSequence = 0
let fieldIdentity = 0
let pendingRead = true
let readController: AbortController | undefined

const selectOptions = computed<SettingOption[]>(() => {
  if (props.kind !== 'select') return props.options
  const current = String(localValue.value || '').trim()
  if (!current || props.options.some((option) => option.value === current)) return props.options
  // Keep a value from an older/unavailable plugin visible and selectable. It
  // can still be saved unchanged while the user decides whether to replace
  // it, instead of silently making the field look unset.
  return [{ value: current, label: current, description: st('Current configured value') }, ...props.options]
})

const busy = computed(() => loading.value || saving.value || props.disabled)
const effectiveResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.effective || null)
const globalResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.global || null)
const workspaceResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.workspace || null)
const hasOverride = computed(() =>
  hasPersistedSetting(props.targetLayer === 'workspace' ? workspaceResponse.value : globalResponse.value),
)
const dirty = computed(() => JSON.stringify(localValue.value) !== JSON.stringify(savedValue.value))

function normalizeLocal(value: JsonValue): string | number | boolean {
  if (props.kind === 'boolean') {
    if (value === true) return true
    if (value === false) return false
    return props.defaultValue === '' ? '' : props.defaultValue === true
  }
  if (props.kind === 'number') {
    if (props.includeEmpty && (value === null || value === undefined || value === '')) return ''
    const numeric = typeof value === 'number' ? value : Number(value)
    return Number.isFinite(numeric) ? numeric : Number(props.defaultValue) || 0
  }
  return typeof value === 'string' ? value : value === null || value === undefined ? '' : String(value)
}

function syncLocal() {
  const effective = settingValue(effectiveResponse.value, props.defaultValue)
  localValue.value = normalizeLocal(effective as JsonValue)
  savedValue.value = localValue.value
}

async function refresh() {
  if (!props.path.trim()) return
  if (!visible.value) {
    pendingRead = true
    return
  }
  pendingRead = false
  const sequence = ++readSequence
  readController?.abort()
  const controller = new AbortController()
  readController = controller
  loading.value = true
  error.value = ''
  try {
    const next = await readRuntimeSettingSources(props.path, controller.signal)
    if (controller.signal.aborted || sequence !== readSequence) return
    sources.value = next
    syncLocal()
    emit('loaded', next)
  } catch (reason) {
    if (controller.signal.aborted || sequence !== readSequence) return
    error.value = reason instanceof Error ? reason.message : String(reason)
    emit('error', error.value)
  } finally {
    if (sequence === readSequence) {
      readController = undefined
      loading.value = false
    }
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
  if (busy.value || !dirty.value) return
  clearAutoSaveTimer()
  const identity = fieldIdentity
  const path = props.path
  const layer = props.targetLayer
  const reload = props.reload
  saving.value = true
  error.value = ''
  try {
    // An empty value in a TUI select means "unset". Persisting an empty
    // string would create an invalid/meaningless override for optional
    // settings such as ui.tui.theme, so remove the selected layer entry.
    const clearSelectedValue =
      (props.kind === 'select' && props.includeEmpty && !String(localValue.value || '').trim()) ||
      ((props.kind === 'boolean' || props.kind === 'number') && props.includeEmpty && localValue.value === '')
    if (clearSelectedValue) {
      await deleteRuntimeSetting(path, { reload }, layer)
      if (identity !== fieldIdentity) return
      await refresh()
      if (identity !== fieldIdentity) return
      emit('saved', settingValue(effectiveResponse.value, props.defaultValue) as JsonValue)
      return
    }
    const value = serializedValue()
    await setRuntimeSetting(path, value, { reload }, layer)
    if (identity !== fieldIdentity) return
    await refresh()
    if (identity !== fieldIdentity) return
    emit('saved', value)
  } catch (reason) {
    if (identity !== fieldIdentity) return
    error.value = reason instanceof Error ? reason.message : String(reason)
    emit('error', error.value)
  } finally {
    if (identity === fieldIdentity) saving.value = false
  }
}

async function clearOverride() {
  if (busy.value || !hasOverride.value) return
  clearAutoSaveTimer()
  const identity = fieldIdentity
  const path = props.path
  const layer = props.targetLayer
  const reload = props.reload
  saving.value = true
  error.value = ''
  try {
    await deleteRuntimeSetting(path, { reload }, layer)
    if (identity !== fieldIdentity) return
    await refresh()
    if (identity !== fieldIdentity) return
    emit('saved', settingValue(effectiveResponse.value, props.defaultValue) as JsonValue)
  } catch (reason) {
    if (identity !== fieldIdentity) return
    error.value = reason instanceof Error ? reason.message : String(reason)
    emit('error', error.value)
  } finally {
    if (identity === fieldIdentity) saving.value = false
  }
}

function clearAutoSaveTimer() {
  if (autoSaveTimer !== null) {
    clearTimeout(autoSaveTimer)
    autoSaveTimer = null
  }
}

function scheduleAutoSave(delay = 650) {
  clearAutoSaveTimer()
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null
    void save()
  }, delay)
}

function flushAutoSave() {
  clearAutoSaveTimer()
  void save()
}

function onNumberInput(event: Event) {
  const target = event.target as HTMLInputElement
  localValue.value = target.value === '' ? (props.includeEmpty ? '' : 0) : Number(target.value)
  scheduleAutoSave()
}

function onTextInput(event: Event) {
  localValue.value = (event.target as HTMLInputElement).value
  scheduleAutoSave()
}

function onBooleanCheckbox(value: boolean) {
  localValue.value = value
  scheduleAutoSave(0)
}

function onBooleanSelect(value: string) {
  localValue.value = value === 'true' ? true : value === 'false' ? false : ''
  scheduleAutoSave(0)
}

function onSelectValue(value: string) {
  localValue.value = value
  scheduleAutoSave(0)
}

function effectiveLabel(response: RuntimeSettingReadResponse | null): string {
  return displayJsonValue(response?.value)
}

watch(
  () => [props.path, props.targetLayer],
  () => {
    clearAutoSaveTimer()
    fieldIdentity++
    readSequence++
    readController?.abort()
    readController = undefined
    loading.value = saving.value = false
    sources.value = null
    localValue.value = savedValue.value = props.defaultValue
    error.value = ''
    void refresh()
  },
  { flush: 'sync' },
)
watch(
  visible,
  (shown) => {
    if (shown) {
      if (pendingRead) void refresh()
    } else if (readController) {
      pendingRead = true
      readSequence++
      readController.abort()
      readController = undefined
      loading.value = false
    }
  },
  { flush: 'sync' },
)
watch(
  () => props.defaultValue,
  () => {
    if (!sources.value) localValue.value = props.defaultValue
  },
)

onMounted(() => void refresh())
onBeforeUnmount(() => {
  clearAutoSaveTimer()
  fieldIdentity++
  readSequence++
  readController?.abort()
})
// A page refresh must not cancel an auto-save or replace a pending edit.
async function refreshIfClean() {
  if (!dirty.value && !busy.value) await refresh()
}
defineExpose({ refresh: refreshIfClean })
</script>

<template>
  <div
    :class="
      compact
        ? 'min-w-0 border-b border-border/50 py-4 first:pt-0 last:border-b-0 last:pb-0'
        : 'min-w-0 rounded-xl border border-border/60 bg-background/30 p-4'
    "
  >
    <div class="grid min-w-0 gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(12rem,0.65fr)] sm:items-start">
      <div class="min-w-0">
        <div class="text-sm font-medium">{{ label }}</div>
        <p v-if="description" class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ description }}</p>
      </div>
      <div class="min-w-0 space-y-2">
        <div class="flex min-w-0 items-center gap-2">
          <div v-if="kind === 'boolean' && !includeEmpty" class="flex min-h-9 flex-1 items-center gap-2 sm:justify-end">
            <span class="text-xs text-muted-foreground">{{ localValue ? $st('Enabled') : $st('Disabled') }}</span>
            <SettingsToggle
              :model-value="localValue === true"
              :label="label"
              :disabled="busy"
              @update:model-value="onBooleanCheckbox"
            />
          </div>
          <OptionPicker
            v-else-if="kind === 'boolean' && includeEmpty"
            class="min-w-0 flex-1"
            :model-value="localValue === true ? 'true' : localValue === false ? 'false' : ''"
            :options="booleanOptions"
            :title="label"
            :placeholder="placeholder || $st('Select value')"
            :search-placeholder="placeholder || label"
            :include-empty="true"
            :empty-label="emptyLabel || $st('No value')"
            :disabled="busy"
            @update:model-value="onBooleanSelect"
          />
          <OptionPicker
            v-else-if="kind === 'select'"
            class="min-w-0 flex-1"
            :model-value="String(localValue)"
            :options="selectOptions"
            :title="label"
            :placeholder="placeholder || $st('Select value')"
            :search-placeholder="placeholder || label"
            :include-empty="includeEmpty"
            :empty-label="emptyLabel || $st('No value')"
            :allow-custom="allowCustom"
            :disabled="busy"
            :monospace="monospace"
            @update:model-value="onSelectValue"
          />
          <input
            v-else-if="kind === 'number'"
            :value="localValue"
            type="number"
            :aria-label="label"
            :placeholder="placeholder"
            :disabled="busy"
            class="h-9 w-full min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm outline-none focus:border-ring"
            @input="onNumberInput"
            @blur="flushAutoSave"
            @keydown.enter="flushAutoSave"
          />
          <input
            v-else
            :value="localValue"
            type="text"
            :aria-label="label"
            :placeholder="placeholder"
            :disabled="busy"
            :class="[
              'h-9 w-full min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm outline-none focus:border-ring',
              monospace ? 'font-mono' : '',
            ]"
            @input="onTextInput"
            @blur="flushAutoSave"
            @keydown.enter="flushAutoSave"
          />
        </div>
        <div
          class="flex min-h-5 flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground sm:justify-end"
          aria-live="polite"
        >
          <RiLoader4Line v-if="saving || loading" class="h-3.5 w-3.5 animate-spin text-primary" />
          <span v-else-if="dirty || error" class="h-1.5 w-1.5 rounded-full bg-amber-500" />
          <RiCheckboxCircleLine v-else class="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
          <span>{{
            loading
              ? $st('Loading...')
              : error
                ? $st('Error')
                : saving
                  ? $st('Saving automatically…')
                  : dirty
                    ? $st('Waiting to save…')
                    : $st('Saved automatically')
          }}</span>
        </div>
      </div>
    </div>
    <details class="mt-2 text-[11px] text-muted-foreground">
      <summary
        class="w-fit cursor-pointer rounded py-1 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {{ $st('Configuration sources') }}
      </summary>
      <div class="mt-2 space-y-2 rounded-lg border border-border/50 bg-muted/20 p-3">
        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <code class="min-w-0 flex-1 break-all font-mono">{{ path }}</code>
          <IconButton
            variant="ghost"
            size="sm"
            :tooltip="$st('Refresh setting')"
            :aria-label="$st('Refresh setting')"
            :disabled="busy || dirty"
            @click="refreshIfClean"
            ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''"
          /></IconButton>
          <Button
            v-if="hasOverride"
            variant="ghost"
            size="sm"
            :disabled="busy"
            :title="$st('Clear {layer} override', { layer: targetLayer })"
            @click="clearOverride"
          >
            <RiDeleteBinLine class="mr-1.5 h-4 w-4" />{{ $st('Clear') }}
          </Button>
        </div>
        <dl class="grid gap-2 sm:grid-cols-3">
          <div>
            <dt class="font-medium">{{ $st('Effective:') }}</dt>
            <dd class="mt-1 break-all font-mono">{{ effectiveLabel(effectiveResponse) }}</dd>
          </div>
          <div>
            <dt class="font-medium">{{ $st('Global layer:') }}</dt>
            <dd class="mt-1 break-all font-mono">{{ effectiveLabel(globalResponse) }}</dd>
          </div>
          <div>
            <dt class="font-medium">{{ $st('Workspace layer:') }}</dt>
            <dd class="mt-1 break-all font-mono">{{ effectiveLabel(workspaceResponse) }}</dd>
          </div>
        </dl>
      </div>
    </details>
    <div v-if="error" role="alert" class="mt-2 break-words text-xs text-destructive">{{ error }}</div>
  </div>
</template>
