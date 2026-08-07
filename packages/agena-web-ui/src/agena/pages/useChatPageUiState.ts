import { userErrorMessage } from '@/lib/api'
import { watch, type Ref } from 'vue'
import type { Router } from 'vue-router'

import {
  createGitCommit,
  createGitPullRequest,
  deleteMemory,
  downloadWorkspaceFile as downloadWorkspaceFileFromApi,
  providerModelThinkingModeSelector,
  type ProviderModel,
  type ProviderSummary,
  type WorkspaceResource,
} from '../lib/agenaApi'
import { buildRuntimeSectionPath, type RuntimeRouteSection } from './runtimePageStateModel'

function supportedVerbosityLevelsForModel(
  modelId: string | null | undefined,
  metadata: { supports_verbosity?: boolean | null; default_verbosity?: string | null } | null | undefined,
): string[] {
  const defaultVerbosity = metadata?.default_verbosity?.trim().toLowerCase() || ''
  if (!metadata?.supports_verbosity && !defaultVerbosity) return []
  const loweredId = (modelId || '').trim().toLowerCase()
  const levels = loweredId.includes('gpt-5') && loweredId.includes('-chat') ? ['medium'] : ['low', 'medium', 'high']
  if (defaultVerbosity && !levels.includes(defaultVerbosity)) levels.push(defaultVerbosity)
  return levels
}

export type ChatPageUiStateInput = {
  localCommandNotice: Ref<string>
  providerModels: Record<string, ProviderModel[]>
  providers: Ref<ProviderSummary[]>
  selectedAdapterId: Ref<string>
  selectedModelId: Ref<string>
  selectedProviderId: Ref<string>
  selectedThinkingMode: Ref<string>
  selectedSpeedMode: Ref<string>
  selectedVerbosity: Ref<string>
  selectedParallelToolCalls: Ref<string>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  userInputDrafts: Record<string, Record<string, string>>
  workspaces: Ref<WorkspaceResource[]>
}

export type ChatPageUiStateDeps = {
  router: Pick<Router, 'push'>
}

export function useChatPageUiState(input: ChatPageUiStateInput, deps: ChatPageUiStateDeps) {
  function providerDefaultModel(providerId: string): string {
    return input.providers.value.find((provider) => provider.provider_id === providerId)?.defaults.model || ''
  }

  function providerDefaultAdapter(providerId: string): string {
    const provider = input.providers.value.find((provider) => provider.provider_id === providerId)
    return provider?.defaults.adapter || provider?.adapters?.find((adapter) => adapter.enabled)?.adapter_id || ''
  }

  function openWorkspaceBrowser(relativePath = '') {
    const workspaceId = input.selectedWorkspaceId.value || input.workspaces.value[0]?.id
    const query: Record<string, string> = {}
    if (workspaceId) {
      query.workspace = String(workspaceId)
    }
    const normalizedPath = relativePath.trim().replace(/^\/+/, '')
    if (normalizedPath) {
      query.path = normalizedPath
    }
    void deps.router.push({ path: '/workspace', query })
  }

  function openSnapshotInspector() {
    const workspaceId = input.selectedWorkspaceId.value || input.workspaces.value[0]?.id
    const query = workspaceId ? { workspace: String(workspaceId) } : undefined
    void deps.router.push({ path: '/workspace', query, hash: '#workspace-snapshots' })
  }

  function openRuntimeSection(section: RuntimeRouteSection, tab: string) {
    void deps.router.push(buildRuntimeSectionPath(section, tab))
  }

  function openAttachmentPicker(imageOnly = false) {
    if (typeof document === 'undefined') return
    const id = imageOnly ? 'composer-image-input' : 'composer-file-input'
    document.getElementById(id)?.click()
  }

  function focusComposer() {
    if (typeof document === 'undefined') return
    const composer = document.getElementById('composer') as HTMLTextAreaElement | null
    composer?.focus()
    composer?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  }

  function focusTranscript() {
    if (typeof document === 'undefined') return
    const transcript = document.getElementById('chat-messages-panel') as HTMLElement | null
    transcript?.focus()
    transcript?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  function focusRunOptions() {
    if (typeof document === 'undefined') return
    const panel = document.getElementById('chat-run-options') as HTMLElement | null
    panel?.focus()
    panel?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  }

  function openMemorySettings(name?: string) {
    const query = name?.trim() ? { memory: name.trim().replace(/\.md$/i, '') } : undefined
    void deps.router.push({ path: buildRuntimeSectionPath('settings', 'memory'), query })
  }

  function openPermissionSettings(mode?: string) {
    const query: Record<string, string> = {}
    if (mode?.trim()) query.mode = mode.trim().toLowerCase()
    if (input.selectedSessionId.value != null) {
      query.session = String(input.selectedSessionId.value)
    }
    void deps.router.push({ path: buildRuntimeSectionPath('settings', 'permissions'), query })
  }

  async function forgetMemory(name: string) {
    try {
      const removed = await deleteMemory(name)
      input.localCommandNotice.value = `Forgot memory ${removed.name}.`
    } catch (error) {
      input.localCommandNotice.value = userErrorMessage(error)
    }
  }

  async function downloadWorkspaceFile(path: string) {
    const workspaceId = input.selectedWorkspaceId.value
    if (!workspaceId) {
      input.localCommandNotice.value = 'Select a workspace before downloading a file.'
      return
    }
    try {
      await downloadWorkspaceFileFromApi({ workspaceId, path })
      input.localCommandNotice.value = `Downloaded ${path.trim()}.`
    } catch (error) {
      input.localCommandNotice.value = userErrorMessage(error)
    }
  }

  function providerAdapterOptions(providerId: string): string[] {
    const provider = input.providers.value.find((provider) => provider.provider_id === providerId)
    const adapterIds = new Set<string>()
    for (const adapter of provider?.adapters || []) {
      if (adapter.enabled) adapterIds.add(adapter.adapter_id)
    }
    for (const model of providerId ? input.providerModels[providerId] || [] : []) {
      if (model.adapter_id) adapterIds.add(model.adapter_id)
    }
    return [...adapterIds].sort((left, right) => left.localeCompare(right))
  }

  function providerModelOptions(providerId: string, adapterId = ''): ProviderModel[] {
    const models = providerId ? input.providerModels[providerId] || [] : []
    const selectedAdapter = adapterId.trim()
    return selectedAdapter
      ? models.filter((model) => !model.adapter_id || model.adapter_id === selectedAdapter)
      : models
  }

  function providerModelLabel(model: ProviderModel): string {
    return model.display_name?.trim() || model.id
  }

  function selectedProviderModel(): ProviderModel | undefined {
    return providerModelOptions(input.selectedProviderId.value, input.selectedAdapterId.value).find(
      (model) => model.id === input.selectedModelId.value,
    )
  }

  function modelThinkingModeOptions(): Array<{ id: string; label: string; description: string }> {
    const modes = selectedProviderModel()?.thinking_modes || []
    return modes.flatMap((mode) => {
      const id = providerModelThinkingModeSelector(mode)
      return id ? [{ id, label: mode.display_name?.trim() || id, description: mode.description?.trim() || id }] : []
    })
  }

  function modelSpeedModeOptions(): Array<{ id: string; label: string; description: string }> {
    const modes = selectedProviderModel()?.speed_modes || {}
    return Object.entries(modes).map(([id, mode]) => ({
      id,
      label: mode.display_name?.trim() || id,
      description: mode.description?.trim() || id,
    }))
  }

  /**
   * Resolve the default think/speed mode selectors for the currently
   * selected model, matching the defaults the runtime applies when a session
   * starts. Prefers the mode the model marks as default; when the model
   * exposes modes but none is marked default (common for catalog models),
   * falls back to the first listed mode so the active-session status stays
   * populated. Returns empty strings when the model cannot be resolved or
   * exposes no modes.
   */
  function modelDefaultModes(): { thinking: string; speed: string } {
    const model = selectedProviderModel()
    const thinkingModes = model?.thinking_modes || []
    const thinkingMode = thinkingModes.find((mode) => mode.default) ?? thinkingModes[0]
    const thinking = thinkingMode ? providerModelThinkingModeSelector(thinkingMode) : ''
    const speedEntries = Object.entries(model?.speed_modes || {})
    const speedEntry = speedEntries.find(([, mode]) => mode.default) ?? speedEntries[0]
    return { thinking, speed: speedEntry?.[0] || '' }
  }

  function modelVerbosityOptions(): Array<{ id: string; label: string; description: string }> {
    const model = selectedProviderModel()
    const metadata = model?.metadata
    return supportedVerbosityLevelsForModel(model?.id, metadata).map((id) => ({
      id,
      label: id,
      description: id,
    }))
  }

  function modelParallelToolCallsOptions(): Array<{ id: string; label: string; description: string }> {
    const model = selectedProviderModel()
    if (!model?.metadata?.supports_parallel_tool_calls && !input.selectedParallelToolCalls.value) return []
    return [
      { id: 'true', label: 'Enabled', description: 'Allow concurrent tool calls' },
      { id: 'false', label: 'Disabled', description: 'Force serial tool calls' },
    ]
  }

  function formatMessageTime(value: string): string {
    const date = new Date(value)
    if (Number.isNaN(date.getTime())) return value
    return date.toLocaleString()
  }

  function formatEventTime(timestampMs: number): string {
    const date = new Date(timestampMs)
    if (Number.isNaN(date.getTime())) return String(timestampMs)
    return date.toLocaleString()
  }

  function scrollToMessage(messageId: number) {
    if (typeof document === 'undefined') return
    const target = document.querySelector<HTMLElement>(`[data-message-id="${messageId}"]`)
    target?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  }

  function readUserAnswer(requestId: string, questionId: string): string {
    return input.userInputDrafts[requestId]?.[questionId] || ''
  }

  function updateUserAnswer(requestId: string, questionId: string, value: string) {
    ;(input.userInputDrafts[requestId] ||= {})[questionId] = value
  }

  function copySessionUsageSummary(summaryFacts: string[]) {
    input.localCommandNotice.value = summaryFacts.length
      ? `Session usage: ${summaryFacts.join(' · ')}`
      : 'No assistant usage has been recorded for the active session yet.'
  }

  async function copyText(text: string, successMessage: string) {
    const normalized = text.trim()
    if (!normalized) {
      input.localCommandNotice.value = 'There is no text to copy.'
      return
    }
    if (typeof navigator === 'undefined' || !navigator.clipboard?.writeText) {
      input.localCommandNotice.value = 'Clipboard access is unavailable in this browser context.'
      return
    }
    try {
      await navigator.clipboard.writeText(normalized)
      input.localCommandNotice.value = successMessage
    } catch (error) {
      input.localCommandNotice.value = `Clipboard write failed: ${userErrorMessage(error)}`
    }
  }

  async function createCommit(message: string) {
    try {
      const result = await createGitCommit(message)
      input.localCommandNotice.value = `Created commit ${result.commit.slice(0, 12)}: ${result.summary}`
    } catch (error) {
      input.localCommandNotice.value = userErrorMessage(error)
    }
  }

  async function createPullRequest(options: { title: string; body?: string; base?: string; head?: string }) {
    try {
      const result = await createGitPullRequest(options)
      input.localCommandNotice.value = `Created pull request: ${result.url}`
    } catch (error) {
      input.localCommandNotice.value = userErrorMessage(error)
    }
  }

  watch(input.selectedProviderId, (providerId) => {
    if (!providerId) return
    input.selectedThinkingMode.value = ''
    input.selectedSpeedMode.value = ''
    input.selectedVerbosity.value = ''
    input.selectedParallelToolCalls.value = ''
    if (!input.selectedAdapterId.value) {
      input.selectedAdapterId.value = providerDefaultAdapter(providerId)
    }
    if (!input.selectedModelId.value) {
      input.selectedModelId.value = providerDefaultModel(providerId)
    }
  })

  watch(input.selectedAdapterId, () => {
    input.selectedThinkingMode.value = ''
    input.selectedSpeedMode.value = ''
    input.selectedVerbosity.value = ''
    input.selectedParallelToolCalls.value = ''
  })

  watch(input.selectedModelId, () => {
    input.selectedThinkingMode.value = ''
    input.selectedSpeedMode.value = ''
    input.selectedVerbosity.value = ''
    input.selectedParallelToolCalls.value = ''
  })

  return {
    copySessionUsageSummary,
    copyText,
    createCommit,
    createPullRequest,
    downloadWorkspaceFile,
    formatEventTime,
    formatMessageTime,
    openRuntimeSection,
    openAttachmentPicker,
    openMemorySettings,
    openPermissionSettings,
    forgetMemory,
    focusComposer,
    focusTranscript,
    focusRunOptions,
    openWorkspaceBrowser,
    openSnapshotInspector,
    providerAdapterOptions,
    providerDefaultAdapter,
    providerDefaultModel,
    providerModelLabel,
    providerModelOptions,
    modelDefaultModes,
    modelParallelToolCallsOptions,
    modelThinkingModeOptions,
    modelSpeedModeOptions,
    modelVerbosityOptions,
    readUserAnswer,
    scrollToMessage,
    updateUserAnswer,
  }
}
