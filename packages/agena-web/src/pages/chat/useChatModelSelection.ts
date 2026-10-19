import { computed, ref, watch, type Ref } from 'vue'
import { i18n } from '@/i18n'

import {
  defaultModeValue,
  modeOptionDisplayLabel,
  modelSelectionIsConfigured,
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
  readSessionManualModelEntry,
  readSessionRunConfigSelection,
  removeSessionManualModelPair,
  sessionCachePredatesSession,
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
  /** Server creation time of the open session, when the client already knows it. */
  selectedSessionCreatedMs?: number | null
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
  composerPickerOpen: Ref<null | PickerKind>
  modelPickerQuery: Ref<string>
  onOpenComposerPicker: () => void
  commandOpen: Ref<boolean>
  commandQuery: Ref<string>
  commandIndex: Ref<number>
  commandFocusSearch: Ref<boolean>
}) {
  const { chat, composerPickerOpen, modelPickerQuery, onOpenComposerPicker, commandOpen, commandQuery, commandIndex } =
    opts

  const catalog = useModelSelectionCatalog()
  const { providers, runtimeDefaultSelection, catalogLoading, catalogError, modelMetaFor } = catalog

  const selectedProviderId = ref('')
  const selectedAdapterId = ref('')
  const selectedModelId = ref('')
  const selectedThinkingMode = ref('')
  const selectedSpeedMode = ref('')
  const modelSource = ref<'empty' | 'session' | 'default' | 'manual'>('empty')
  const thinkingModeSource = ref<'empty' | 'session' | 'default' | 'model' | 'manual'>('empty')
  const speedModeSource = ref<'empty' | 'session' | 'default' | 'model' | 'manual'>('empty')

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
    if (!provider || !model) return String(i18n.global.t('chat.composer.picker.modelTitle'))
    return adapter ? `${provider}/${adapter}/${model}` : `${provider}/${model}`
  })
  // The TUI's composer status uses the provider catalog display name and
  // hides routing details. Keep the full slug above for the Web picker and
  // model hint, but expose the same compact status projection separately.
  const modelStatusLabel = computed(() => {
    const displayName = text(selectedModelMeta.value?.display_name)
    if (displayName) return displayName
    return selectedModelId.value || String(i18n.global.t('chat.composer.picker.modelTitle'))
  })
  const modelChipLabelMobile = computed(
    () => selectedModelId.value || String(i18n.global.t('chat.composer.picker.modelTitle')),
  )
  const thinkingModeChipLabel = computed(() => {
    const selected = selectedThinkingMode.value
    return statusThinkingMode(selected) || String(i18n.global.t('chat.composer.picker.thinkingTitle'))
  })
  const speedModeChipLabel = computed(() => {
    const selected = selectedSpeedMode.value
    return (
      modeOptionDisplayLabel(speedModeOptionsForModel(selectedModelMeta.value), selected) || statusSpeedMode(selected)
    )
  })

  const modelHint = computed(() => {
    if (!selectedProviderId.value || !selectedModelId.value)
      return String(i18n.global.t('chat.toasts.selectModelBeforeSending'))
    if (modelSource.value === 'manual') return ''
    if (modelSource.value === 'default') {
      return String(i18n.global.t('chat.toasts.modelHints.runtimeDefault', { model: modelChipLabel.value }))
    }
    return String(i18n.global.t('chat.toasts.modelHints.sessionModel', { model: modelChipLabel.value }))
  })
  const thinkingModeHint = computed(() =>
    thinkingModeSource.value === 'manual' || !selectedThinkingMode.value
      ? ''
      : String(i18n.global.t('chat.toasts.modelHints.thinkingMode', { mode: selectedThinkingMode.value })),
  )
  const speedModeHint = computed(() => {
    if (speedModeSource.value === 'manual' || !selectedSpeedMode.value) return ''
    const mode =
      modeOptionDisplayLabel(speedModeOptionsForModel(selectedModelMeta.value), selectedSpeedMode.value) ||
      statusSpeedMode(selectedSpeedMode.value)
    return String(i18n.global.t('chat.toasts.modelHints.speedMode', { mode }))
  })

  const picker = useModelSelectionPickerUi({
    composerPickerOpen,
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

  function selectedSessionCreatedMs(): number | null {
    const created = Number(chat.selectedSessionCreatedMs)
    return Number.isFinite(created) && created > 0 ? created : null
  }

  /**
   * The loaded catalog is the only source of models this client can actually
   * run: it lists enabled adapters and the models configured for them. Until
   * it arrives the cached value is kept so the composer does not flicker, and
   * the catalog watcher re-resolves as soon as the inventory is available.
   */
  function selectionIsOffered(candidate: { provider?: string; adapter?: string; model?: string }): boolean {
    if (!providers.value.length) return true
    const provider = providers.value.find((item) => item.id === text(candidate.provider))
    // A provider whose inventory this client cannot see (model discovery, or a
    // catalog read that failed for it) is no evidence either way: keep the
    // cached selection instead of replacing a working model with the default.
    if (!provider || !provider.models.length) return true
    return modelSelectionIsConfigured(providers.value, candidate)
  }

  function adoptSessionSelection(candidate: { provider?: string; adapter?: string; model?: string }): boolean {
    if (!text(candidate.provider) || !text(candidate.model)) return false
    if (!selectionIsOffered(candidate)) return false
    setModelSelection(
      { provider: text(candidate.provider), adapter: text(candidate.adapter), model: text(candidate.model) },
      'session',
    )
    return true
  }

  function dropStaleManualSelection(sessionId: string) {
    if (!sessionId) return
    if (!Object.prototype.hasOwnProperty.call(sessionManualModelBySession.value, sessionId)) return
    sessionManualModelBySession.value = removeSessionManualModelPair(sessionManualModelBySession.value, sessionId)
    sessionManualModelPersister.persistSoon()
  }

  /**
   * A session's own model wins over the runtime default, but only while the
   * cached evidence can belong to this session and the catalog still offers
   * the model. Session ids are reused after a server database is recreated,
   * and editing the provider configuration can retire a model or an adapter.
   */
  function resolveModelSelection(includeSessionLayers: boolean) {
    const sessionId = includeSessionLayers ? activeSessionId() : ''
    if (includeSessionLayers) {
      const createdAtMs = selectedSessionCreatedMs()
      const manual = readSessionManualModelEntry(sessionManualModelBySession.value, sessionId)
      if (manual) {
        const current = !sessionCachePredatesSession(manual.at, createdAtMs)
        if (current && adoptSessionSelection(parseModelSlug(manual.value))) return
        dropStaleManualSelection(sessionId)
      }
      const runAt = Number(chat.selectedSessionRunConfig?.at)
      if (!sessionCachePredatesSession(runAt, createdAtMs) && adoptSessionSelection(sessionRunSelection())) return
      if (adoptSessionSelection(deriveSessionSelectionFromMessages(chat.messages))) return
    }

    const defaults = runtimeDefaultSelection.value
    if (defaults.provider && defaults.model) {
      setModelSelection(defaults, 'default')
      return
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
    const defaults = runtimeDefaultSelection.value
    const defaultsMatch = sameModel(defaults, current)
    const savedThinking = text(thinkingModeByModelKey.value[key])
    const runThinking = runMatches ? run.thinkingMode : ''
    const configuredThinking = defaultsMatch ? defaults.thinkingMode : ''
    const modelThinking = defaultModeValue(thinkingModeOptionsForModel(selectedModelMeta.value))
    selectedThinkingMode.value = savedThinking || runThinking || configuredThinking || modelThinking
    thinkingModeSource.value = savedThinking
      ? 'manual'
      : runThinking
        ? 'session'
        : configuredThinking
          ? 'default'
          : selectedThinkingMode.value
            ? 'model'
            : 'empty'

    const savedSpeed = text(speedModeByModelKey.value[key])
    const runSpeed = runMatches ? run.speedMode : ''
    const configuredSpeed = defaultsMatch ? defaults.speedMode : ''
    const modelSpeed = defaultModeValue(speedModeOptionsForModel(selectedModelMeta.value))
    selectedSpeedMode.value = savedSpeed || runSpeed || configuredSpeed || modelSpeed
    speedModeSource.value = savedSpeed
      ? 'manual'
      : runSpeed
        ? 'session'
        : configuredSpeed
          ? 'default'
          : selectedSpeedMode.value
            ? 'model'
            : 'empty'
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
    if (!selectionIsOffered(selection)) return
    setModelSelection(selection, 'manual')
    const sessionId = activeSessionId()
    if (sessionId) {
      sessionManualModelBySession.value = writeSessionManualModelPair(
        sessionManualModelBySession.value,
        sessionId,
        selection.provider,
        selection.adapter,
        selection.model,
        Date.now(),
      )
      sessionManualModelPersister.persistSoon()
    }
    resolveModes(false)
    picker.closeComposerPicker()
  }

  function chooseModelDefault() {
    const sessionId = activeSessionId()
    if (sessionId) {
      sessionManualModelBySession.value = removeSessionManualModelPair(sessionManualModelBySession.value, sessionId)
      sessionManualModelPersister.persistSoon()
    }
    applyResolvedSelection(false)
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
    () => chat.selectedSessionId,
    () => applySessionSelection(),
  )
  watch(
    () => chat.selectedSessionRunConfig?.at ?? null,
    () => applySessionSelection(),
    { immediate: true },
  )
  watch(
    () => chat.messages.length,
    () => applySessionSelection(),
  )
  watch([runtimeDefaultSelection, providers], () => applySessionSelection())

  return {
    providers,
    catalogLoading,
    catalogError,
    runtimeDefaultSelection,
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
    modelPickerQuery,
    toggleComposerPicker: picker.toggleComposerPicker,
    chooseModelSlug,
    chooseModelDefault,
    chooseThinkingMode,
    chooseThinkingModeDefault,
    chooseSpeedMode,
    chooseSpeedModeDefault,
    resetSelectionForSessionSwitch,
    applySessionSelection,
    loadProvidersAndModels,
  }
}
