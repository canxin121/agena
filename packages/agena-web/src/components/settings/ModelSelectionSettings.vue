<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RiResetLeftLine } from '@remixicon/vue'
import SettingsSourceControl from './SettingsSourceControl.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { sameServerModelIdentity } from '@/lib/serverModelSettings'
import { runtimeSettingBaseValue } from '@/lib/runtimeSettingLayer'
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
import { getSessionExecution, setSessionSelection, type RunOptionsPayload } from '@/stores/chat/api'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{
  scope: string
  purpose: 'default' | 'approval' | 'auxiliary'
  sessionId?: string
}>()
const scope = computed(() =>
  props.scope === 'global' || props.scope === 'workspace' || props.scope === 'session' ? props.scope : 'effective',
)
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
const saveLabel = computed(() =>
  auxiliary.value ? st('Save auxiliary model') : approval.value ? st('Save approval model') : st('Save default model'),
)
const sessionTarget = computed(() => String(props.sessionId || '').trim())
const targetIdentity = computed(() => JSON.stringify([scope.value, path.value, sessionTarget.value]))
const editable = computed(
  () => scope.value !== 'effective' && (scope.value !== 'session' || Boolean(sessionTarget.value)),
)
const catalog = useModelSelectionCatalog()
const toasts = useToastsStore()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const saveError = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const sessionSelection = ref<JsonValue | undefined>(undefined)
const sessionInheritedSelection = ref<JsonValue | undefined>(undefined)
const sessionEffectiveSelection = ref<JsonValue | undefined>(undefined)
const sourceMode = ref<'inherit' | 'local'>('inherit')
const modelKey = ref('')
const thinkingMode = ref('')
const speedMode = ref('')
const verbosity = ref('')
const parallelToolCalls = ref<boolean | undefined>(undefined)
const savedState = ref('')
const resetPending = ref(false)
const preferenceKeys = ['thinking_mode', 'speed_mode', 'verbosity'] as const
type PreferenceKey = (typeof preferenceKeys)[number]
const routeOwned = ref(false)
const routeLocalIntent = ref(false)
const preferenceOwned = ref(new Set<PreferenceKey>())
const preferenceLocalIntent = ref(new Set<PreferenceKey>())
const suspendedPreferenceDrafts = new Map<PreferenceKey, string>()
let suspendedModelDraft: {
  model: string
  thinking: string
  speed: string
  verbosity: string
  parallel: boolean | undefined
  routeOwned: boolean
  routeIntent: boolean
  owned: PreferenceKey[]
  intents: PreferenceKey[]
} | null = null
let controller: AbortController | null = null
let disposed = false

function record(value: JsonValue | undefined): Record<string, JsonValue> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}
function selectionKey(value: Record<string, JsonValue>) {
  return encodeModelSelectionKey({
    provider: String(value.provider || ''),
    adapter: String(value.adapter || ''),
    model: String(value.model || ''),
  })
}
function hasModelChoices(value: Record<string, JsonValue>) {
  return ['provider', 'adapter', 'model', 'thinking_mode', 'speed_mode', 'verbosity', 'parallel_tool_calls'].some(
    (key) => value[key] !== undefined && value[key] !== null,
  )
}
/** Older servers expose an effective selection but cannot prove its source. */
function effectiveSessionRecord(state: Awaited<ReturnType<typeof getSessionExecution>> | null): JsonValue | undefined {
  const execution = state?.execution
  if (!execution?.model_provider_id || !execution.model_id) return undefined
  return {
    provider: execution.model_provider_id,
    model: execution.model_id,
    ...(execution.model_adapter_id ? { adapter: execution.model_adapter_id } : {}),
    ...(execution.model_thinking_mode ? { thinking_mode: execution.model_thinking_mode } : {}),
    ...(execution.model_speed_mode ? { speed_mode: execution.model_speed_mode } : {}),
    ...(execution.model_verbosity ? { verbosity: execution.model_verbosity } : {}),
  }
}
function fingerprint() {
  return JSON.stringify([
    sourceMode.value,
    modelKey.value,
    thinkingMode.value,
    speedMode.value,
    verbosity.value,
    parallelToolCalls.value,
    routeOwned.value,
    routeLocalIntent.value,
    [...preferenceOwned.value].sort(),
    [...preferenceLocalIntent.value].sort(),
  ])
}
const dirty = computed(() => editable.value && !!savedState.value && fingerprint() !== savedState.value)
const disabled = computed(() => !editable.value || loading.value || saving.value || !sources.value)
const modelSource = computed<'inherit' | 'local'>(() =>
  scope.value === 'session' ? (routeOwned.value || routeLocalIntent.value ? 'local' : 'inherit') : sourceMode.value,
)
const modelValueDisabled = computed(
  () => disabled.value || (scope.value !== 'global' && modelSource.value === 'inherit'),
)
const inheritedSource = computed(() => (scope.value === 'workspace' ? st('Global layer') : st('Parent configuration')))
const localDraftIncomplete = computed(
  () =>
    (routeLocalIntent.value && !modelKey.value) ||
    [...preferenceLocalIntent.value].some((key) => !preferenceRef(key).value),
)
const layerValue = computed<JsonValue | undefined>(() =>
  scope.value === 'session' ? sessionSelection.value : sources.value?.[scope.value]?.value,
)
const persisted = computed(
  () =>
    editable.value &&
    (scope.value === 'session'
      ? hasModelChoices(record(sessionSelection.value))
      : hasPersistedSetting(sources.value?.[scope.value])),
)
const sourceKnown = computed(() => scope.value !== 'session' || sessionSelection.value !== undefined)
const inheritedValue = computed(() =>
  scope.value === 'session'
    ? sessionInheritedSelection.value
    : scope.value === 'effective'
      ? undefined
      : runtimeSettingBaseValue(sources.value?.[scope.value], scope.value),
)
const inheritedSelection = computed(() => record(inheritedValue.value))
const inheritedModelKey = computed(() => selectionKey(inheritedSelection.value))
const resolvedSelection = computed(() => {
  if (scope.value === 'session' || scope.value === 'effective') return undefined
  const value = sources.value?.[scope.value]?.layer_value
  return value === undefined ? undefined : record(value)
})
const displayedModelKey = computed(() => {
  if (!editable.value) return selectionKey(record(layerValue.value))
  if (scope.value !== 'global' && modelSource.value === 'inherit') return inheritedModelKey.value
  if (routeLocalIntent.value && !modelKey.value) return ''
  if (!dirty.value && resolvedSelection.value !== undefined) return selectionKey(resolvedSelection.value)
  return modelKey.value || (sourceMode.value === 'inherit' || scope.value === 'session' ? inheritedModelKey.value : '')
})
const identity = computed(() => parseModelSlug(modelKey.value))
const displayedIdentity = computed(() => parseModelSlug(displayedModelKey.value))
const displayedRoute = computed(() => {
  const { provider, adapter, model } = displayedIdentity.value
  return [provider, adapter, model].filter(Boolean).join(' / ')
})
const selectedModel = computed(() =>
  catalog.modelMetaFor(
    displayedIdentity.value.provider,
    displayedIdentity.value.model,
    displayedIdentity.value.adapter,
  ),
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
  for (const key of new Set([modelKey.value, inheritedModelKey.value, displayedModelKey.value])) {
    if (key && !options.some((option) => option.value === key)) {
      options.unshift({ value: key, label: parseModelSlug(key).model, description: st('Configured value') })
    }
  }
  return options
})
function withSelected(options: ModelModeOption[], value: string) {
  return value && !options.some((option) => option.value === value)
    ? [...options, { value, label: value, description: st('Configured value'), isDefault: false }]
    : options
}
const thinkingOptions = computed(() =>
  withSelected(thinkingModeOptionsForModel(selectedModel.value), preferenceValue('thinking_mode')),
)
const speedOptions = computed(() =>
  withSelected(speedModeOptionsForModel(selectedModel.value), preferenceValue('speed_mode')),
)
const verbosityOptions = computed(() =>
  withSelected(verbosityOptionsForModel(selectedModel.value), preferenceValue('verbosity')),
)
const preferences = computed(
  () =>
    [
      { key: 'thinking_mode', label: st('Thinking'), options: thinkingOptions.value },
      { key: 'speed_mode', label: st('Speed'), options: speedOptions.value },
      { key: 'verbosity', label: st('Verbosity'), options: verbosityOptions.value },
    ] as const,
)
const modelName = computed(
  () => modelOptions.value.find((option) => option.value === displayedModelKey.value)?.label || '',
)
const inheritedModelName = computed(() =>
  inheritedValue.value === undefined
    ? st('Unavailable')
    : modelOptions.value.find((option) => option.value === inheritedModelKey.value)?.label ||
      (approval.value || auxiliary.value ? st('Use the session model') : st('No default model configured')),
)
const modelPlaceholder = computed(() => {
  if (!sources.value) return loading.value ? st('Loading…') : st('Unavailable')
  if (scope.value === 'session' && !sessionTarget.value) return '—'
  if (editable.value) return inheritedModelName.value
  return approval.value || auxiliary.value ? st('Use the session model') : st('No default model configured')
})

function readOnlyPreferenceLabel(key: PreferenceKey, options: ModelModeOption[]) {
  if (!sources.value || (scope.value === 'session' && !sessionTarget.value)) return '—'
  const value = preferenceValue(key)
  return value ? options.find((option) => option.value === value)?.label || value : preferenceEmptyLabel(key)
}

function syncEditor() {
  const value = record(layerValue.value)
  modelKey.value = selectionKey(value)
  thinkingMode.value = String(value.thinking_mode || '')
  speedMode.value = String(value.speed_mode || '')
  verbosity.value = String(value.verbosity || '')
  parallelToolCalls.value = typeof value.parallel_tool_calls === 'boolean' ? value.parallel_tool_calls : undefined
  sourceMode.value = persisted.value ? 'local' : 'inherit'
  routeOwned.value = ['provider', 'adapter', 'model'].some((key) => value[key] !== undefined && value[key] !== null)
  routeLocalIntent.value = false
  preferenceOwned.value = new Set(preferenceKeys.filter((key) => value[key] !== undefined && value[key] !== null))
  preferenceLocalIntent.value = new Set()
  suspendedModelDraft = null
  suspendedPreferenceDrafts.clear()
  resetPending.value = false
  savedState.value = fingerprint()
  saveError.value = ''
}
function chooseModel(value: string) {
  if (modelValueDisabled.value || (!value && !sourceKnown.value)) return
  if (!value) return chooseSource('inherit')
  sourceMode.value = 'local'
  modelKey.value = value
  routeOwned.value = true
  routeLocalIntent.value = false
  resetPending.value = false
  const model = selectedModel.value
  if (scope.value === 'session') {
    for (const key of preferenceKeys) if (!preferenceOwned.value.has(key)) preferenceRef(key).value = ''
  } else {
    thinkingMode.value = defaultModeValue(thinkingModeOptionsForModel(model))
    speedMode.value = defaultModeValue(speedModeOptionsForModel(model))
    verbosity.value = defaultModeValue(verbosityOptionsForModel(model))
    parallelToolCalls.value = undefined
  }
  saveError.value = ''
}
function chooseSource(value: 'inherit' | 'local') {
  if (scope.value === 'global' && value === 'local') return
  if (disabled.value || (value === 'local' ? modelSource.value === 'local' : sourceMode.value === 'inherit')) return
  if (value === 'inherit') {
    suspendedModelDraft = {
      model: modelKey.value,
      thinking: thinkingMode.value,
      speed: speedMode.value,
      verbosity: verbosity.value,
      parallel: parallelToolCalls.value,
      routeOwned: routeOwned.value,
      routeIntent: routeLocalIntent.value,
      owned: [...preferenceOwned.value],
      intents: [...preferenceLocalIntent.value],
    }
    modelKey.value = thinkingMode.value = speedMode.value = verbosity.value = ''
    parallelToolCalls.value = undefined
    routeOwned.value = routeLocalIntent.value = false
    preferenceOwned.value = new Set()
    preferenceLocalIntent.value = new Set()
    sourceMode.value = 'inherit'
    resetPending.value = persisted.value
    saveError.value = ''
    return
  }
  if (suspendedModelDraft) {
    const draft = suspendedModelDraft
    modelKey.value = draft.model
    thinkingMode.value = draft.thinking
    speedMode.value = draft.speed
    verbosity.value = draft.verbosity
    parallelToolCalls.value = draft.parallel
    routeOwned.value = draft.routeOwned
    routeLocalIntent.value = draft.routeIntent
    preferenceOwned.value = new Set(draft.owned)
    preferenceLocalIntent.value = new Set(draft.intents)
    sourceMode.value = 'local'
    resetPending.value = false
    suspendedModelDraft = null
    return
  }
  materializeBaseSelection()
}
function materializeBaseSelection() {
  const selection = {
    ...(scope.value === 'global' && !dirty.value && resolvedSelection.value !== undefined
      ? resolvedSelection.value
      : inheritedSelection.value),
  }
  if (scope.value === 'session') {
    for (const key of preferenceKeys) if (preferenceOwned.value.has(key)) selection[key] = preferenceRef(key).value
    if (parallelToolCalls.value !== undefined) selection.parallel_tool_calls = parallelToolCalls.value
  }
  const key = selectionKey(selection)
  modelKey.value = key
  thinkingMode.value = String(selection.thinking_mode || '')
  speedMode.value = String(selection.speed_mode || '')
  verbosity.value = String(selection.verbosity || '')
  parallelToolCalls.value =
    typeof selection.parallel_tool_calls === 'boolean' ? selection.parallel_tool_calls : undefined
  sourceMode.value = 'local'
  routeOwned.value = Boolean(key)
  routeLocalIntent.value = !key
  preferenceOwned.value = new Set(
    preferenceKeys.filter((key) => selection[key] !== undefined && selection[key] !== null),
  )
  preferenceLocalIntent.value = new Set()
  resetPending.value = false
  saveError.value = ''
}
function preferenceRef(key: PreferenceKey) {
  return key === 'thinking_mode' ? thinkingMode : key === 'speed_mode' ? speedMode : verbosity
}
function preferenceValue(key: PreferenceKey) {
  if (!editable.value) return preferenceRef(key).value
  if (scope.value === 'session')
    return preferenceSource(key) === 'inherit' ? String(inheritedSelection.value[key] || '') : preferenceRef(key).value
  if (scope.value === 'workspace' && sourceMode.value === 'inherit') return String(inheritedSelection.value[key] || '')
  if (!dirty.value && resolvedSelection.value !== undefined) return String(resolvedSelection.value[key] || '')
  return sourceMode.value === 'inherit' ? String(inheritedSelection.value[key] || '') : preferenceRef(key).value
}
function preferenceEmptyLabel(key: PreferenceKey) {
  if (scope.value === 'session' && preferenceSource(key) === 'inherit') return inheritedPreferenceLabel(key)
  return st('Model default')
}
function preferenceSource(key: PreferenceKey): 'inherit' | 'local' {
  if (scope.value !== 'session') return sourceMode.value
  return routeOwned.value || preferenceOwned.value.has(key) || preferenceLocalIntent.value.has(key)
    ? 'local'
    : 'inherit'
}
function inheritedPreferenceLabel(key: PreferenceKey) {
  return inheritedValue.value === undefined
    ? st('Unavailable')
    : String(inheritedSelection.value[key] || st('Model default'))
}
function preferenceValueDisabled(key: PreferenceKey) {
  return (
    disabled.value ||
    (scope.value !== 'global' && preferenceSource(key) === 'inherit') ||
    (scope.value !== 'session' && !displayedModelKey.value)
  )
}
function choosePreferenceSource(key: PreferenceKey, value: 'inherit' | 'local') {
  if (scope.value !== 'session' || disabled.value || routeOwned.value || value === preferenceSource(key)) return
  if (value === 'inherit') {
    suspendedPreferenceDrafts.set(key, preferenceRef(key).value)
    preferenceRef(key).value = ''
    preferenceOwned.value.delete(key)
    preferenceLocalIntent.value.delete(key)
  } else {
    const inherited = inheritedSelection.value[key]
    preferenceRef(key).value = suspendedPreferenceDrafts.has(key)
      ? suspendedPreferenceDrafts.get(key)!
      : typeof inherited === 'string'
        ? inherited
        : ''
    suspendedPreferenceDrafts.delete(key)
    preferenceLocalIntent.value.add(key)
    if (preferenceRef(key).value) preferenceOwned.value.add(key)
  }
  sourceMode.value =
    routeOwned.value ||
    routeLocalIntent.value ||
    preferenceOwned.value.size ||
    preferenceLocalIntent.value.size ||
    parallelToolCalls.value !== undefined
      ? 'local'
      : 'inherit'
  resetPending.value = sourceMode.value === 'inherit' && persisted.value
  saveError.value = ''
}
function choosePreference(key: PreferenceKey, value: string) {
  if (preferenceValueDisabled(key)) return
  if (scope.value === 'session' && !routeOwned.value && !value) return choosePreferenceSource(key, 'inherit')
  // Config model selections replace the entire group. Session mode-only
  // overrides retain their inherited route and other unset response modes.
  if (sourceMode.value === 'inherit' && scope.value !== 'session') materializeBaseSelection()
  sourceMode.value = 'local'
  preferenceRef(key).value = value
  if (value) preferenceOwned.value.add(key)
  else preferenceOwned.value.delete(key)
  preferenceLocalIntent.value.delete(key)
  resetPending.value = false
  saveError.value = ''
}
function acceptSessionState(state: Awaited<ReturnType<typeof getSessionExecution>> | null) {
  sessionSelection.value = state?.execution?.selected_selection
  sessionInheritedSelection.value = state?.execution?.inherited_selection
  sessionEffectiveSelection.value = effectiveSessionRecord(state)
}
async function refresh() {
  if (saving.value) return
  controller?.abort()
  const request = new AbortController()
  const target = targetIdentity.value
  controller = request
  loading.value = true
  error.value = ''
  try {
    const [bundle, state] = await Promise.all([
      readRuntimeSettingSources(path.value, request.signal),
      scope.value === 'session' && sessionTarget.value
        ? getSessionExecution(sessionTarget.value)
        : Promise.resolve(null),
      catalog.loadProvidersAndModels(),
    ])
    if (disposed || request.signal.aborted || target !== targetIdentity.value) return
    sources.value = bundle
    acceptSessionState(state)
    if (!dirty.value) syncEditor()
  } catch (reason) {
    if (!disposed && !request.signal.aborted && target === targetIdentity.value) {
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (controller === request) loading.value = false
  }
}
async function save() {
  const layer = scope.value
  if (disabled.value || layer === 'effective' || !dirty.value || localDraftIncomplete.value) return
  if (layer === 'session') return saveSessionSelection()
  const selected = identity.value
  const mode = sourceMode.value
  if (mode === 'local' && (!selected.provider || !selected.model)) {
    saveError.value = st('Select a configured model.')
    return
  }
  const target = targetIdentity.value
  const settingPath = path.value
  saving.value = true
  saveError.value = ''
  controller?.abort()
  try {
    if (mode === 'inherit') {
      await deleteRuntimeSetting(settingPath, { reload: true }, layer)
    } else {
      const value = { ...record(sources.value?.[layer]?.value) }
      for (const key of [
        'provider',
        'adapter',
        'model',
        'thinking_mode',
        'speed_mode',
        'verbosity',
        'parallel_tool_calls',
      ])
        delete value[key]
      Object.assign(
        value,
        { provider: selected.provider, model: selected.model },
        selected.adapter ? { adapter: selected.adapter } : {},
        thinkingMode.value ? { thinking_mode: thinkingMode.value } : {},
        speedMode.value ? { speed_mode: speedMode.value } : {},
        verbosity.value ? { verbosity: verbosity.value } : {},
        parallelToolCalls.value !== undefined ? { parallel_tool_calls: parallelToolCalls.value } : {},
      )
      await setRuntimeSetting(settingPath, value, { reload: true }, layer)
    }
    const bundle = await readRuntimeSettingSources(settingPath)
    if (disposed || target !== targetIdentity.value) return
    if (
      mode === 'local' ? !sameServerModelIdentity(bundle[layer]?.value, selected) : hasPersistedSetting(bundle[layer])
    ) {
      throw new Error(st('The server did not apply the model selection. Your draft has been kept.'))
    }
    sources.value = bundle
    syncEditor()
    toasts.push('success', st('Model settings saved'))
  } catch (reason) {
    if (!disposed && target === targetIdentity.value)
      saveError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
    if (!disposed && target !== targetIdentity.value) void refresh()
  }
}
async function saveSessionSelection() {
  if (disabled.value || !dirty.value || localDraftIncomplete.value) return
  const sid = sessionTarget.value
  const target = targetIdentity.value
  const selected = identity.value
  if (!sid || localDraftIncomplete.value || (!sourceKnown.value && !modelKey.value)) return
  if (modelKey.value && (!selected.provider || !selected.model)) {
    saveError.value = st('Select a configured model.')
    return
  }
  saving.value = true
  saveError.value = ''
  controller?.abort()
  try {
    const options: RunOptionsPayload = {}
    if (sourceMode.value === 'local') {
      if (modelKey.value)
        options.model = {
          provider_id: selected.provider,
          ...(selected.adapter ? { adapter_id: selected.adapter } : {}),
          model_id: selected.model,
        }
      const raw = record(sessionSelection.value)
      if (parallelToolCalls.value !== undefined) options.parallel_tool_calls = parallelToolCalls.value
      for (const key of ['thinking_mode', 'speed_mode', 'verbosity'] as const) {
        const value = preferenceRef(key).value
        if (value || raw[key] !== undefined) options[key] = value
      }
    }
    const state = await setSessionSelection(sid, options)
    if (disposed || target !== targetIdentity.value) return
    acceptSessionState(state)
    syncEditor()
    toasts.push('success', st('Model settings saved'))
  } catch (reason) {
    if (!disposed && target === targetIdentity.value)
      saveError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
    if (!disposed && target !== targetIdentity.value) void refresh()
  }
}

watch(targetIdentity, () => {
  controller?.abort()
  sources.value = null
  acceptSessionState(null)
  syncEditor()
  void refresh()
})
onMounted(() => void refresh())
onBeforeUnmount(() => {
  disposed = true
  controller?.abort()
})
defineExpose({ refresh, dirty, loading, saving, catalogError: catalog.catalogError })
</script>

<template>
  <tbody
    class="model-setting-role"
    role="rowgroup"
    :class="{ 'model-setting-role-with-sources': scope === 'session' && !routeOwned && editable }"
    :data-model-selection="purpose"
    :aria-busy="loading || saving"
  >
    <tr class="model-setting-row" role="row">
      <th class="model-setting-name" scope="row" role="rowheader">{{ title }}</th>
      <td class="model-setting-route" role="cell">
        <div v-if="editable" class="model-setting-route-controls">
          <OptionPicker
            class="model-setting-picker model-setting-route-picker"
            :model-value="displayedModelKey"
            :options="modelOptions"
            :title="title"
            :placeholder="modelPlaceholder"
            :include-empty="false"
            :search-placeholder="$st('Search configured models...')"
            :disabled="modelValueDisabled"
            size="sm"
            @update:model-value="chooseModel"
          />
          <SettingsSourceControl
            :model-value="modelSource"
            :scope="scope"
            :label="title"
            :inherited-value="inheritedModelName"
            :inherited-source="inheritedSource"
            :show-inherited-value="false"
            :disabled="disabled"
            @update:model-value="chooseSource"
          />
          <IconButton
            v-if="scope === 'session' && sourceMode === 'local' && modelSource === 'inherit'"
            size="sm"
            data-model-selection-reset
            :disabled="disabled"
            :tooltip="$st('Model route') + ' · ' + $st('Response preferences') + ': ' + $st('Use inherited value')"
            :aria-label="$st('Model route') + ' · ' + $st('Response preferences') + ': ' + $st('Use inherited value')"
            @click="chooseSource('inherit')"
            ><RiResetLeftLine class="h-3.5 w-3.5" aria-hidden="true"
          /></IconButton>
        </div>
        <span v-else class="model-setting-value">{{ modelName || modelPlaceholder }}</span>
        <code v-if="displayedRoute" class="model-setting-model-id" :title="displayedRoute" dir="ltr">{{
          displayedRoute
        }}</code>
      </td>
      <td v-for="{ key, label, options } in preferences" :key="key" class="model-setting-preference" role="cell">
        <span class="model-setting-compact-label" aria-hidden="true">{{ label }}</span>
        <div class="model-setting-preference-controls">
          <OptionPicker
            v-if="editable"
            class="model-setting-picker"
            :model-value="preferenceValue(key)"
            :options="options"
            :title="label"
            :empty-label="preferenceEmptyLabel(key)"
            :include-empty="scope !== 'session' || routeOwned"
            :allow-custom="scope === 'session' && (!displayedModelKey || !options.length)"
            :display-label="
              !sources
                ? '—'
                : preferenceSource(key) === 'inherit' && !preferenceValue(key)
                  ? inheritedPreferenceLabel(key)
                  : undefined
            "
            :disabled="preferenceValueDisabled(key) || (key === 'verbosity' && scope !== 'session' && !options.length)"
            size="sm"
            @update:model-value="choosePreference(key, $event)"
          />
          <span v-else class="model-setting-value">{{ readOnlyPreferenceLabel(key, options) }}</span>
          <SettingsSourceControl
            v-if="scope === 'session' && !routeOwned && editable"
            :model-value="preferenceSource(key)"
            scope="session"
            :label="label"
            :inherited-source="inheritedSource"
            :inherited-value="inheritedPreferenceLabel(key)"
            :show-inherited-summary="false"
            :disabled="disabled"
            @update:model-value="choosePreferenceSource(key, $event)"
          />
        </div>
      </td>
    </tr>
    <tr v-if="error || saveError || dirty || saving" class="model-setting-feedback" role="row">
      <td colspan="5" role="cell">
        <div class="settings-toolbar model-setting-status">
          <p v-if="error || saveError" class="min-w-0 flex-1 break-words text-xs text-destructive" role="alert">
            {{ saveError || error }}
          </p>
          <p v-else-if="resetPending" class="min-w-0 flex-1 text-xs text-muted-foreground" role="status">
            {{
              scope === 'global'
                ? $st('Save to restore default settings.')
                : $st('Save to remove this scope’s override.')
            }}
          </p>
          <div v-if="dirty || saving" class="settings-toolbar ms-auto">
            <Button variant="ghost" size="sm" :disabled="loading || saving || !sources" @click="syncEditor">
              {{ $st('Discard changes') }}
            </Button>
            <Button
              size="sm"
              :aria-label="saveLabel"
              :disabled="disabled || !dirty || localDraftIncomplete"
              data-model-save
              @click="save"
              >{{ saving ? $st('Saving…') : $st('Save') }}</Button
            >
          </div>
        </div>
      </td>
    </tr>
  </tbody>
</template>
