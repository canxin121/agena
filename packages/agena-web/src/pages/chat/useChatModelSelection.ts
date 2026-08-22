import { computed, ref, watch, type Ref } from 'vue'

import {
  defaultModeValue,
  modeOptionDisplayLabel,
  speedModeOptionsForModel,
  thinkingModeOptionsForModel,
  useModelSelectionCatalog,
  type ModelModeOption,
  type ProviderModel,
} from './modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from './modelSelectionDefaults'
import {
  deriveSessionSelectionFromMessages,
  normalizeSessionManualModelStorageEntry,
  readSessionManualModelPair,
  readSessionRunConfigSelection,
  writeSessionManualModelPair,
  type SessionSelection,
} from './modelSelectionSession'
import { useModelSelectionPickerUi } from './modelSelectionPickerUi'
import { createStringMapPersister, loadStringMapFromStorage, normalizeStringMapEntry } from './modelSelectionStorage'
import { localStorageKeys } from '../../lib/persistence/storageKeys'
import type { SessionRunConfig } from '@/types/chat'

type ChatMessageLike = {
  info?: {
    providerID?: unknown
    adapterID?: unknown
    modelID?: unknown
  }
}

type ChatLike = {
  selectedSessionId: string | null
  selectedSessionRunConfig: SessionRunConfig | null
  messages: ChatMessageLike[]
}

export type ModelSlugOption = {
  value: string
  label: string
  providerId: string
  adapterId: string
  modelId: string
  description: string
}

type PickerKind = 'model' | 'thinking' | 'speed'

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function statusThinkingMode(value: string): string {
  return text(value)
}

function statusSpeedMode(value: string): string {
  const normalized = text(value)
  if (normalized.toLowerCase() === 'no-speed') return 'off'
  return normalized.startsWith('speed-') ? normalized.slice('speed-'.length) : normalized
}

function sameModel(
  left: { provider?: string; adapter?: string; model?: string },
  right: { provider?: string; adapter?: string; model?: string },
): boolean {
  return (
    text(left.provider) === text(right.provider) &&
    text(left.adapter) === text(right.adapter) &&
    text(left.model) === text(right.model)
  )
}

function withSelectedMode(options: ModelModeOption[], selected: string): ModelModeOption[] {
  const value = text(selected)
  if (!value || options.some((option) => option.value === value)) return options
  return [...options, { value, label: value, description: '', isDefault: false }]
}

export function useChatModelSelection(opts: {
  chat: ChatLike
  composerControlsRef: Ref<HTMLDivElement | null>
  composerPickerOpen: Ref<null | PickerKind>
  composerPickerStyle: Ref<Record<string, string>>
  modelTriggerRef: Ref<HTMLElement | null>
  thinkingTriggerRef: Ref<HTMLElement | null>
  speedTriggerRef: Ref<HTMLElement | null>
  modelPickerQuery: Ref<string>
  onOpenComposerPicker: () => void
  commandOpen: Ref<boolean>
  commandQuery: Ref<string>
  commandIndex: Ref<number>
}) {
  const {
    chat,
    composerControlsRef,
    composerPickerOpen,
    composerPickerStyle,
    modelTriggerRef,
    thinkingTriggerRef,
    speedTriggerRef,
    modelPickerQuery,
    onOpenComposerPicker,
    commandOpen,
    commandQuery,
    commandIndex,
  } = opts

  const catalog = useModelSelectionCatalog()
  const { providers, catalogLoading, catalogError, modelMetaFor } = catalog

  const selectedProviderId = ref('')
  const selectedAdapterId = ref('')
  const selectedModelId = ref('')
  const selectedThinkingMode = ref('')
  const selectedSpeedMode = ref('')
  const modelSource = ref<'empty' | 'session' | 'manual'>('empty')
  const thinkingModeSource = ref<'empty' | 'session' | 'model' | 'manual'>('empty')
  const speedModeSource = ref<'empty' | 'session' | 'model' | 'manual'>('empty')

  const sessionManualModelBySession = ref<Record<string, string>>(
    loadStringMapFromStorage(
      localStorageKeys.chat.sessionManualModelBySession,
      normalizeSessionManualModelStorageEntry,
    ),
  )
  const sessionManualModelPersister = createStringMapPersister({
    storageKey: localStorageKeys.chat.sessionManualModelBySession,
    getValue: () => sessionManualModelBySession.value,
  })

  const thinkingModeByModelKey = ref<Record<string, string>>(
    loadStringMapFromStorage(localStorageKeys.chat.modelThinkingModeByKey, normalizeStringMapEntry),
  )
  const thinkingModePersister = createStringMapPersister({
    storageKey: localStorageKeys.chat.modelThinkingModeByKey,
    getValue: () => thinkingModeByModelKey.value,
  })

  const speedModeByModelKey = ref<Record<string, string>>(
    loadStringMapFromStorage(localStorageKeys.chat.modelSpeedModeByKey, normalizeStringMapEntry),
  )
  const speedModePersister = createStringMapPersister({
    storageKey: localStorageKeys.chat.modelSpeedModeByKey,
    getValue: () => speedModeByModelKey.value,
  })

  const selectedModelSlug = computed(() =>
    encodeModelSelectionKey({
      provider: selectedProviderId.value,
      adapter: selectedAdapterId.value,
      model: selectedModelId.value,
    }),
  )
  const selectedModelKey = selectedModelSlug

  const selectedModelMeta = computed<ProviderModel | null>(() =>
    modelMetaFor(selectedProviderId.value, selectedModelId.value, selectedAdapterId.value),
  )

  const modelSlugOptions = computed<ModelSlugOption[]>(() => {
    const list: ModelSlugOption[] = []
    for (const provider of providers.value) {
      for (const model of provider.models) {
        const adapterId = text(model.adapter_id)
        const value = encodeModelSelectionKey({ provider: provider.id, adapter: adapterId, model: model.id })
        if (!value) continue
        list.push({
          value,
          label: text(model.display_name) || model.id,
          providerId: provider.id,
          adapterId,
          modelId: model.id,
          description: adapterId ? `${provider.id} / ${adapterId}` : provider.id,
        })
      }
    }
    const selected = selectedModelSlug.value
    if (selected && !list.some((option) => option.value === selected)) {
      list.push({
        value: selected,
        label: selectedModelId.value,
        providerId: selectedProviderId.value,
        adapterId: selectedAdapterId.value,
        modelId: selectedModelId.value,
        description: selectedAdapterId.value
          ? `${selectedProviderId.value} / ${selectedAdapterId.value}`
          : selectedProviderId.value,
      })
    }
    return list.sort((left, right) =>
      `${left.providerId}/${left.adapterId}/${left.label}`.localeCompare(
        `${right.providerId}/${right.adapterId}/${right.label}`,
      ),
    )
  })

  const filteredModelSlugOptions = computed(() => {
    const query = modelPickerQuery.value.trim().toLowerCase()
    if (!query) return modelSlugOptions.value
    return modelSlugOptions.value.filter((option) =>
      `${option.label} ${option.providerId} ${option.adapterId} ${option.modelId}`.toLowerCase().includes(query),
    )
  })

  const thinkingModeOptions = computed(() =>
    withSelectedMode(thinkingModeOptionsForModel(selectedModelMeta.value), selectedThinkingMode.value),
  )
  const speedModeOptions = computed(() =>
    withSelectedMode(speedModeOptionsForModel(selectedModelMeta.value), selectedSpeedMode.value),
  )
  const hasThinkingModesForSelection = computed(() => thinkingModeOptionsForModel(selectedModelMeta.value).length > 0)
  const hasSpeedModesForSelection = computed(() => speedModeOptionsForModel(selectedModelMeta.value).length > 0)

  const modelChipLabel = computed(() => {
    const provider = selectedProviderId.value
    const adapter = selectedAdapterId.value
    const model = selectedModelId.value
    if (!provider || !model) return 'Model'
    return adapter ? `${provider}/${adapter}/${model}` : `${provider}/${model}`
  })
  // The TUI's composer status uses the provider catalog display name and
  // hides routing details. Keep the full slug above for the Web picker and
  // model hint, but expose the same compact status projection separately.
  const modelStatusLabel = computed(() => {
    const displayName = text(selectedModelMeta.value?.display_name)
    if (displayName) return displayName
    return selectedModelId.value || 'Model'
  })
  const modelChipLabelMobile = computed(() => selectedModelId.value || 'Model')
  const thinkingModeChipLabel = computed(() => {
    const selected = selectedThinkingMode.value
    return statusThinkingMode(selected) || 'Thinking'
  })
  const speedModeChipLabel = computed(() => {
    const selected = selectedSpeedMode.value
    return (
      modeOptionDisplayLabel(speedModeOptionsForModel(selectedModelMeta.value), selected) || statusSpeedMode(selected)
    )
  })

  const modelHint = computed(() => {
    if (!selectedProviderId.value || !selectedModelId.value) return 'Select a model before sending'
    if (modelSource.value === 'manual') return ''
    return `Using session model: ${modelChipLabel.value}`
  })
  const thinkingModeHint = computed(() =>
    thinkingModeSource.value === 'manual' || !selectedThinkingMode.value
      ? ''
      : `Using thinking mode: ${selectedThinkingMode.value}`,
  )
  const speedModeHint = computed(() =>
    speedModeSource.value === 'manual' || !selectedSpeedMode.value
      ? ''
      : `Using speed mode: ${
          modeOptionDisplayLabel(speedModeOptionsForModel(selectedModelMeta.value), selectedSpeedMode.value) ||
          statusSpeedMode(selectedSpeedMode.value)
        }`,
  )

  const picker = useModelSelectionPickerUi({
    composerControlsRef,
    composerPickerOpen,
    composerPickerStyle,
    modelTriggerRef,
    thinkingTriggerRef,
    speedTriggerRef,
    modelPickerQuery,
    onOpenComposerPicker,
    commandOpen,
    commandQuery,
    commandIndex,
  })

  function activeSessionId(): string {
    return text(chat.selectedSessionId)
  }

  function setModelSelection(
    selection: { provider: string; adapter?: string; model: string },
    source: typeof modelSource.value,
  ) {
    selectedProviderId.value = text(selection.provider)
    selectedAdapterId.value = text(selection.adapter)
    selectedModelId.value = text(selection.model)
    modelSource.value = selectedProviderId.value && selectedModelId.value ? source : 'empty'
  }

  function sessionRunSelection(): SessionSelection {
    return readSessionRunConfigSelection(chat.selectedSessionRunConfig)
  }

  function resolveModelSelection(includeSessionLayers: boolean) {
    const sessionId = includeSessionLayers ? activeSessionId() : ''
    const candidates = includeSessionLayers
      ? [
          readSessionManualModelPair(sessionManualModelBySession.value, sessionId),
          sessionRunSelection(),
          deriveSessionSelectionFromMessages(chat.messages),
        ]
      : []
    for (const candidate of candidates) {
      if (candidate.provider && candidate.model) {
        setModelSelection(candidate, 'session')
        return
      }
    }

    setModelSelection({ provider: '', adapter: '', model: '' }, 'empty')
  }

  function resolveModes(includeSessionLayers: boolean) {
    const key = selectedModelKey.value
    if (!key) {
      selectedThinkingMode.value = ''
      selectedSpeedMode.value = ''
      thinkingModeSource.value = 'empty'
      speedModeSource.value = 'empty'
      return
    }

    const run = sessionRunSelection()
    const current = {
      provider: selectedProviderId.value,
      adapter: selectedAdapterId.value,
      model: selectedModelId.value,
    }
    const runMatches = includeSessionLayers && sameModel(run, current)
    const savedThinking = text(thinkingModeByModelKey.value[key])
    const runThinking = runMatches ? run.thinkingMode : ''
    const modelThinking = defaultModeValue(thinkingModeOptionsForModel(selectedModelMeta.value))
    selectedThinkingMode.value = savedThinking || runThinking || modelThinking
    thinkingModeSource.value = savedThinking
      ? 'manual'
      : runThinking
        ? 'session'
        : selectedThinkingMode.value
          ? 'model'
          : 'empty'

    const savedSpeed = text(speedModeByModelKey.value[key])
    const runSpeed = runMatches ? run.speedMode : ''
    const modelSpeed = defaultModeValue(speedModeOptionsForModel(selectedModelMeta.value))
    selectedSpeedMode.value = savedSpeed || runSpeed || modelSpeed
    speedModeSource.value = savedSpeed ? 'manual' : runSpeed ? 'session' : selectedSpeedMode.value ? 'model' : 'empty'
  }

  function applyResolvedSelection(includeSessionLayers: boolean) {
    resolveModelSelection(includeSessionLayers)
    resolveModes(includeSessionLayers)
  }

  function applySessionSelection() {
    applyResolvedSelection(Boolean(chat.selectedSessionId))
  }

  function resetSelectionForSessionSwitch() {
    selectedProviderId.value = ''
    selectedAdapterId.value = ''
    selectedModelId.value = ''
    selectedThinkingMode.value = ''
    selectedSpeedMode.value = ''
    modelSource.value = 'empty'
    thinkingModeSource.value = 'empty'
    speedModeSource.value = 'empty'
  }

  function chooseModelSlug(value: string) {
    const selection = parseModelSlug(value)
    if (!selection.provider || !selection.model) return
    setModelSelection(selection, 'manual')
    const sessionId = activeSessionId()
    if (sessionId) {
      sessionManualModelBySession.value = writeSessionManualModelPair(
        sessionManualModelBySession.value,
        sessionId,
        selection.provider,
        selection.adapter,
        selection.model,
      )
      sessionManualModelPersister.persistSoon()
    }
    resolveModes(false)
    picker.closeComposerPicker()
  }

  function chooseThinkingMode(value: string) {
    const key = selectedModelKey.value
    const mode = text(value)
    if (!key || !mode) return
    thinkingModeByModelKey.value = { ...thinkingModeByModelKey.value, [key]: mode }
    thinkingModePersister.persistSoon()
    selectedThinkingMode.value = mode
    thinkingModeSource.value = 'manual'
    picker.closeComposerPicker()
  }

  function chooseThinkingModeDefault() {
    const key = selectedModelKey.value
    if (key && Object.prototype.hasOwnProperty.call(thinkingModeByModelKey.value, key)) {
      const next = { ...thinkingModeByModelKey.value }
      delete next[key]
      thinkingModeByModelKey.value = next
      thinkingModePersister.persistSoon()
    }
    selectedThinkingMode.value = ''
    thinkingModeSource.value = 'empty'
    resolveModes(false)
    picker.closeComposerPicker()
  }

  function chooseSpeedMode(value: string) {
    const key = selectedModelKey.value
    const mode = text(value)
    if (!key || !mode) return
    speedModeByModelKey.value = { ...speedModeByModelKey.value, [key]: mode }
    speedModePersister.persistSoon()
    selectedSpeedMode.value = mode
    speedModeSource.value = 'manual'
    picker.closeComposerPicker()
  }

  function chooseSpeedModeDefault() {
    const key = selectedModelKey.value
    if (key && Object.prototype.hasOwnProperty.call(speedModeByModelKey.value, key)) {
      const next = { ...speedModeByModelKey.value }
      delete next[key]
      speedModeByModelKey.value = next
      speedModePersister.persistSoon()
    }
    selectedSpeedMode.value = ''
    speedModeSource.value = 'empty'
    resolveModes(false)
    picker.closeComposerPicker()
  }

  async function loadProvidersAndModels() {
    await catalog.loadProvidersAndModels()
    applySessionSelection()
  }

  watch(
    () => chat.selectedSessionRunConfig?.at ?? null,
    () => applySessionSelection(),
    { immediate: true },
  )
  watch(
    () => chat.messages.length,
    () => applySessionSelection(),
  )

  return {
    providers,
    catalogLoading,
    catalogError,
    selectedProviderId,
    selectedAdapterId,
    selectedModelId,
    selectedThinkingMode,
    selectedSpeedMode,
    modelSource,
    thinkingModeSource,
    speedModeSource,
    selectedModelSlug,
    modelSlugOptions,
    filteredModelSlugOptions,
    thinkingModeOptions,
    speedModeOptions,
    hasThinkingModesForSelection,
    hasSpeedModesForSelection,
    modelChipLabel,
    modelStatusLabel,
    modelChipLabelMobile,
    thinkingModeChipLabel,
    speedModeChipLabel,
    modelHint,
    thinkingModeHint,
    speedModeHint,
    modelMetaFor,
    composerPickerOpen,
    composerPickerStyle,
    modelPickerQuery,
    toggleComposerPicker: picker.toggleComposerPicker,
    chooseModelSlug,
    chooseThinkingMode,
    chooseThinkingModeDefault,
    chooseSpeedMode,
    chooseSpeedModeDefault,
    resetSelectionForSessionSwitch,
    applySessionSelection,
    loadProvidersAndModels,
  }
}
