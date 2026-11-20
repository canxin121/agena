<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RiCheckboxCircleLine, RiLoader4Line, RiRefreshLine } from '@remixicon/vue'

import SettingsToggle from './SettingsToggle.vue'
import SettingsSourceControl from './SettingsSourceControl.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import {
  deleteRuntimeSetting,
  displayJsonValue,
  readRuntimeSettingSources,
  setRuntimeSetting,
  settingValue,
  subscribeRuntimeSettingsChanges,
  type RuntimeSettingReadResponse,
  type RuntimeSettingsLayer,
  type RuntimeSettingsReadBundle,
} from '@/lib/runtimeSettings'
import { runtimeSettingLayer } from '@/lib/runtimeSettingLayer'
import { confirmAction } from '@/lib/appConfirm'
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
const sourceMode = ref<'inherit' | 'local'>('inherit')
const savedSourceMode = ref<'inherit' | 'local'>('inherit')
const localValueReady = ref(false)
let suspendedLocalDraft: { value: string | number | boolean; ready: boolean } | null = null
const confirmingSource = ref(false)
const confirmedOverridePath = ref('')
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

const busy = computed(() => loading.value || saving.value || confirmingSource.value || props.disabled)
const effectiveResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.effective || null)
const globalResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.global || null)
const workspaceResponse = computed<RuntimeSettingReadResponse | null>(() => sources.value?.workspace || null)
const layerState = computed(() => runtimeSettingLayer(sources.value, props.targetLayer, undefined))
const groupOverridePath = computed(() => {
  const path = sources.value?.[props.targetLayer]?.override_path
  return layerState.value.hasOverride && path && path !== props.path ? path : ''
})
const requiresGroupReset = computed(
  () => Boolean(groupOverridePath.value) && sources.value?.[props.targetLayer]?.requires_group_reset !== false,
)
const inheritedReadOnly = computed(() => props.targetLayer !== 'global' && sourceMode.value === 'inherit')
const valueDisabled = computed(() => busy.value || requiresGroupReset.value || inheritedReadOnly.value)
const dirty = computed(
  () =>
    sourceMode.value !== savedSourceMode.value ||
    (sourceMode.value === 'local' && JSON.stringify(localValue.value) !== JSON.stringify(savedValue.value)),
)
const inheritedLabel = computed(() =>
  layerState.value.inheritedKnown
    ? props.kind === 'select' && props.includeEmpty && layerState.value.inherited == null
      ? props.emptyLabel || st('No value')
      : valueLabel(layerState.value.inherited)
    : st('Unavailable'),
)
const allowsEmptyChoice = computed(() => props.includeEmpty && props.targetLayer !== 'global')
const emptyValueDisplayLabel = computed(() =>
  inheritedReadOnly.value && localValue.value === ''
    ? inheritedLabel.value
    : sources.value && !dirty.value && !layerState.value.valueKnown
      ? st('Unavailable')
      : props.targetLayer === 'global' && props.includeEmpty && localValue.value === ''
        ? layerState.value.hasOverride || layerState.value.inheritedKnown
          ? emptyOptionLabel.value
          : st('Unavailable')
        : undefined,
)
const sourceValueHeading = computed(() =>
  props.targetLayer === 'global' ? st('Default value') : st('Inherited value'),
)
const inheritedSource = computed(() =>
  props.targetLayer === 'workspace' ? st('Global layer') : st('Built-in default'),
)
const localDisabled = computed(() => false)

const emptyOptionLabel = computed(() =>
  sourceMode.value === 'local' && layerState.value.value === null && !dirty.value
    ? st('No value')
    : props.emptyLabel || st('No value'),
)

function valueLabel(value: JsonValue): string {
  if (value === true) return st('Enabled')
  if (value === false) return st('Disabled')
  if (value === null || value === undefined) return st('No value')
  if (value === '') return props.emptyLabel || st('No value')
  return props.options.find((option) => option.value === String(value))?.label || displayJsonValue(value)
}

function normalizeLocal(value: JsonValue): string | number | boolean {
  if (value === null || value === undefined) return ''
  if (props.kind === 'boolean') {
    if (value === true) return true
    if (value === false) return false
    return ''
  }
  if (props.kind === 'number') {
    if (props.includeEmpty && (value === null || value === undefined || value === '')) return ''
    const numeric = typeof value === 'number' ? value : Number(value)
    return Number.isFinite(numeric) ? numeric : ''
  }
  return typeof value === 'string' ? value : value === null || value === undefined ? '' : String(value)
}

function syncLocal() {
  sourceMode.value = savedSourceMode.value = layerState.value.hasOverride ? 'local' : 'inherit'
  const known = inheritedReadOnly.value ? layerState.value.inheritedKnown : layerState.value.valueKnown
  const value = inheritedReadOnly.value ? layerState.value.inherited : layerState.value.value
  localValue.value = known ? normalizeLocal(value) : ''
  savedValue.value = localValue.value
  localValueReady.value = sourceMode.value === 'local'
  suspendedLocalDraft = null
  confirmedOverridePath.value = ''
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
  if (sourceMode.value === 'local' && !localValueReady.value) return
  if (
    groupOverridePath.value &&
    (sourceMode.value === 'inherit'
      ? confirmedOverridePath.value !== groupOverridePath.value
      : requiresGroupReset.value)
  )
    return
  clearAutoSaveTimer()
  const identity = fieldIdentity
  const path = sourceMode.value === 'inherit' ? groupOverridePath.value || props.path : props.path
  const layer = props.targetLayer
  const reload = props.reload
  saving.value = true
  error.value = ''
  try {
    // An empty value in a TUI select means "unset". Persisting an empty
    // string would create an invalid/meaningless override for optional
    // settings such as ui.tui.theme, so remove the selected layer entry.
    const clearSelectedValue =
      sourceMode.value === 'inherit' ||
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

async function chooseSource(mode: 'inherit' | 'local') {
  if (props.targetLayer === 'global' && mode === 'local') return
  if (busy.value || !sources.value || mode === sourceMode.value || (mode === 'local' && localDisabled.value)) return
  if (mode === 'local' && groupOverridePath.value) {
    clearAutoSaveTimer()
    const draft = suspendedLocalDraft
    syncLocal()
    if (draft) {
      localValue.value = draft.value
      localValueReady.value = draft.ready
    }
    error.value = ''
    if (localValueReady.value && dirty.value) scheduleAutoSave(0)
    return
  }
  if (mode === 'inherit' && groupOverridePath.value) {
    const identity = fieldIdentity
    const path = groupOverridePath.value
    confirmingSource.value = true
    let accepted = false
    try {
      accepted = await confirmAction({
        title:
          props.targetLayer === 'global'
            ? st('Restore defaults for {path}?', { path })
            : st('Restore inheritance for {path}?', { path }),
        description:
          props.targetLayer === 'global'
            ? st('This also restores the default values of the other settings in this group.')
            : st('This also restores the other settings in this group.'),
        confirmText: props.targetLayer === 'global' ? st('Reset to defaults') : st('Use inherited value'),
        cancelText: st('Cancel'),
        variant: 'default',
      })
    } finally {
      if (identity === fieldIdentity) confirmingSource.value = false
    }
    if (!accepted || identity !== fieldIdentity || props.disabled || path !== groupOverridePath.value) return
    confirmedOverridePath.value = path
  }
  if (mode === 'inherit') {
    suspendedLocalDraft = { value: localValue.value, ready: localValueReady.value }
    localValue.value = layerState.value.inheritedKnown ? normalizeLocal(layerState.value.inherited) : ''
    localValueReady.value = false
  } else if (suspendedLocalDraft) {
    localValue.value = suspendedLocalDraft.value
    localValueReady.value = suspendedLocalDraft.ready
    suspendedLocalDraft = null
  } else {
    const value = layerState.value.inherited
    localValue.value = layerState.value.inheritedKnown ? normalizeLocal(value) : ''
    localValueReady.value =
      layerState.value.inheritedKnown &&
      value !== null &&
      value !== undefined &&
      (props.kind !== 'boolean' || typeof localValue.value === 'boolean') &&
      (props.kind !== 'number' || (typeof localValue.value === 'number' && Number.isFinite(localValue.value))) &&
      !(props.includeEmpty && ['select', 'boolean', 'number'].includes(props.kind) && localValue.value === '')
  }
  sourceMode.value = mode
  error.value = ''
  clearAutoSaveTimer()
  if (mode === 'inherit' || localValueReady.value) scheduleAutoSave(0)
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
  if (valueDisabled.value) return
  const target = event.target as HTMLInputElement
  if (props.includeEmpty && target.value === '') {
    return chooseSource('inherit')
  }
  localValue.value = target.value === '' ? '' : Number(target.value)
  sourceMode.value = 'local'
  localValueReady.value = typeof localValue.value === 'number' && Number.isFinite(localValue.value)
  if (localValueReady.value) scheduleAutoSave()
  else clearAutoSaveTimer()
}

function onTextInput(event: Event) {
  if (valueDisabled.value) return
  localValue.value = (event.target as HTMLInputElement).value
  sourceMode.value = 'local'
  localValueReady.value = true
  scheduleAutoSave()
}

function onBooleanCheckbox(value: boolean) {
  if (valueDisabled.value) return
  localValue.value = value
  sourceMode.value = 'local'
  localValueReady.value = true
  scheduleAutoSave(0)
}

function onBooleanSelect(value: string) {
  if (valueDisabled.value) return
  if (props.targetLayer === 'global' && !value) return
  if (props.includeEmpty && !value) {
    return chooseSource('inherit')
  }
  localValue.value = value === 'true' ? true : value === 'false' ? false : ''
  sourceMode.value = localValue.value === '' ? 'inherit' : 'local'
  localValueReady.value = typeof localValue.value === 'boolean'
  scheduleAutoSave(0)
}

function onSelectValue(value: string) {
  if (valueDisabled.value) return
  if (props.targetLayer === 'global' && props.includeEmpty && !value.trim()) return
  if (props.includeEmpty && !value.trim()) {
    return chooseSource('inherit')
  }
  localValue.value = value
  sourceMode.value = props.includeEmpty && !value.trim() ? 'inherit' : 'local'
  localValueReady.value = true
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
    sourceMode.value = savedSourceMode.value = 'inherit'
    localValueReady.value = false
    suspendedLocalDraft = null
    confirmingSource.value = false
    confirmedOverridePath.value = ''
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
  unsubscribeSettingsChanges()
})
// A page refresh must not cancel an auto-save or replace a pending edit.
async function refreshIfClean() {
  if (!dirty.value && !busy.value) await refresh()
}
const unsubscribeSettingsChanges = subscribeRuntimeSettingsChanges(() => {
  pendingRead = true
  void refreshIfClean()
})
watch([busy, dirty], () => {
  if (pendingRead && visible.value && !busy.value && !dirty.value) void refresh()
})
defineExpose({ refresh: refreshIfClean })
</script>

<template>
  <div
    :data-server-setting="path"
    :class="
      compact
        ? 'min-w-0 border-b border-border/50 py-2 first:pt-0 last:border-b-0 last:pb-0'
        : 'min-w-0 rounded-lg border border-border/60 bg-background/30 p-2.5'
    "
  >
    <div class="settings-field-row">
      <div class="min-w-0">
        <div class="text-[13px] font-medium">{{ label }}</div>
        <p v-if="description" class="mt-0.5 text-xs leading-snug text-muted-foreground">{{ description }}</p>
        <p v-if="groupOverridePath" class="mt-1 break-all text-xs text-muted-foreground" data-setting-group-override>
          {{ $st('Group override: {path}', { path: groupOverridePath }) }}
        </p>
      </div>
      <div class="min-w-0 space-y-1.5">
        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <SettingsSourceControl
            :model-value="sourceMode"
            :scope="targetLayer"
            :label="label"
            :inherited-value="inheritedLabel"
            :inherited-source="inheritedSource"
            :disabled="busy || !sources"
            :local-disabled="localDisabled"
            @update:model-value="chooseSource"
          />
          <div
            v-if="kind === 'boolean' && !includeEmpty && typeof localValue === 'boolean'"
            class="flex min-h-9 flex-1 items-center gap-2 sm:justify-end"
          >
            <span class="text-xs text-muted-foreground">{{ valueLabel(localValue) }}</span>
            <SettingsToggle
              :model-value="localValue === true"
              :label="label"
              :disabled="valueDisabled"
              @update:model-value="onBooleanCheckbox"
            />
          </div>
          <OptionPicker
            v-else-if="kind === 'boolean'"
            class="min-w-[7rem] flex-1"
            size="sm"
            :model-value="localValue === true ? 'true' : localValue === false ? 'false' : ''"
            :options="booleanOptions"
            :title="label"
            :placeholder="placeholder || $st('Select value')"
            :search-placeholder="placeholder || label"
            :include-empty="allowsEmptyChoice"
            :empty-label="emptyOptionLabel"
            :display-label="emptyValueDisplayLabel"
            :disabled="valueDisabled"
            @update:model-value="onBooleanSelect"
          />
          <OptionPicker
            v-else-if="kind === 'select'"
            class="min-w-[7rem] flex-1"
            size="sm"
            :model-value="String(localValue)"
            :options="selectOptions"
            :title="label"
            :placeholder="placeholder || $st('Select value')"
            :search-placeholder="placeholder || label"
            :include-empty="allowsEmptyChoice"
            :empty-label="emptyOptionLabel"
            :display-label="emptyValueDisplayLabel"
            :allow-custom="allowCustom"
            :disabled="valueDisabled"
            :monospace="monospace"
            @update:model-value="onSelectValue"
          />
          <input
            v-else-if="kind === 'number'"
            :value="localValue"
            type="number"
            :aria-label="label"
            :placeholder="emptyValueDisplayLabel || placeholder"
            :disabled="valueDisabled"
            class="h-8 min-w-[7rem] flex-1 rounded-md border border-input bg-background px-2 text-[13px] outline-none focus:border-ring disabled:cursor-not-allowed disabled:opacity-60"
            @input="onNumberInput"
            @blur="flushAutoSave"
            @keydown.enter="flushAutoSave"
          />
          <input
            v-else
            :value="localValue"
            type="text"
            :aria-label="label"
            :placeholder="emptyValueDisplayLabel || placeholder"
            :disabled="valueDisabled"
            :class="[
              'h-8 min-w-[7rem] flex-1 rounded-md border border-input bg-background px-2 text-[13px] outline-none focus:border-ring disabled:cursor-not-allowed disabled:opacity-60',
              monospace ? 'font-mono' : '',
            ]"
            @input="onTextInput"
            @blur="flushAutoSave"
            @keydown.enter="flushAutoSave"
          />
        </div>
        <p v-if="sources && layerState.effectiveDiffers" class="text-xs text-muted-foreground" data-setting-effective>
          {{ $st('Effective:') }} {{ valueLabel(layerState.effective) }}
          <span class="ml-1">{{ $st('Overridden by a later layer or runtime setting.') }}</span>
        </p>
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
        </div>
        <dl class="settings-control-grid">
          <div>
            <dt class="font-medium">{{ sourceValueHeading }}</dt>
            <dd class="mt-1 break-all">{{ inheritedLabel }}</dd>
          </div>
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
    <div v-if="error" role="alert" class="mt-2 flex items-center gap-2 break-words text-xs text-destructive">
      {{ error }}
      <button v-if="dirty" type="button" class="underline" :disabled="busy" @click="save">{{ $st('Retry') }}</button>
    </div>
  </div>
</template>
