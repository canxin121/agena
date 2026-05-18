import { watch, type Ref } from 'vue'
import type { Router } from 'vue-router'

import type { ProviderModel, ProviderSummary, WorkspaceResource } from '../lib/agenaApi'
import { buildRuntimeSectionPath, type RuntimeRouteSection } from './runtimePageStateModel'

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
  selectedWorkspaceId: Ref<number | null>
  userInputDrafts: Record<string, Record<string, string>>
  workspaces: Ref<WorkspaceResource[]>
}

export type ChatPageUiStateDeps = {
  router: Pick<Router, 'push'>
}

export function useChatPageUiState(input: ChatPageUiStateInput, deps: ChatPageUiStateDeps) {
  function providerDefaultModel(providerId: string): string {
    return input.providers.value.find((provider) => provider.provider_id === providerId)?.default_model || ''
  }

  function providerDefaultAdapter(providerId: string): string {
    const provider = input.providers.value.find((provider) => provider.provider_id === providerId)
    return provider?.default_adapter || provider?.adapters?.find((adapter) => adapter.enabled)?.adapter_id || ''
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

  function openRuntimeSection(section: RuntimeRouteSection, tab: string) {
    void deps.router.push(buildRuntimeSectionPath(section, tab))
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
    const modes = selectedProviderModel()?.thinking_modes || {}
    return Object.entries(modes).map(([id, mode]) => ({
      id,
      label: mode.display_name?.trim() || id,
      description: mode.description?.trim() || id,
    }))
  }

  function modelSpeedModeOptions(): Array<{ id: string; label: string; description: string }> {
    const modes = selectedProviderModel()?.speed_modes || {}
    return Object.entries(modes).map(([id, mode]) => ({
      id,
      label: mode.display_name?.trim() || id,
      description: mode.description?.trim() || id,
    }))
  }

  function modelVerbosityOptions(): Array<{ id: string; label: string; description: string }> {
    const model = selectedProviderModel()
    const metadata = model?.metadata
    if (!metadata?.supports_verbosity && !metadata?.default_verbosity) return []
    const optionIds = new Set(['low', 'medium', 'high'])
    if (metadata.default_verbosity?.trim()) optionIds.add(metadata.default_verbosity.trim().toLowerCase())
    return [...optionIds].map((id) => ({
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
    formatEventTime,
    formatMessageTime,
    openRuntimeSection,
    openWorkspaceBrowser,
    providerAdapterOptions,
    providerDefaultAdapter,
    providerDefaultModel,
    providerModelLabel,
    providerModelOptions,
    modelParallelToolCallsOptions,
    modelThinkingModeOptions,
    modelSpeedModeOptions,
    modelVerbosityOptions,
    readUserAnswer,
    scrollToMessage,
    updateUserAnswer,
  }
}
