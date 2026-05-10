import { watch, type Ref } from 'vue'
import type { Router } from 'vue-router'

import type { ProviderModel, ProviderSummary, WorkspaceResource } from '../lib/agenaApi'
import { buildRuntimeSectionPath, type RuntimeRouteSection } from './runtimePageStateModel'

export type ChatPageUiStateInput = {
  localCommandNotice: Ref<string>
  providerModels: Record<string, ProviderModel[]>
  providers: Ref<ProviderSummary[]>
  selectedModelId: Ref<string>
  selectedProviderId: Ref<string>
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

  function providerModelOptions(providerId: string): ProviderModel[] {
    return providerId ? input.providerModels[providerId] || [] : []
  }

  function providerModelLabel(model: ProviderModel): string {
    return model.display_name?.trim() || model.id
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
    if (!input.selectedModelId.value) {
      input.selectedModelId.value = providerDefaultModel(providerId)
    }
  })

  return {
    copySessionUsageSummary,
    formatEventTime,
    formatMessageTime,
    openRuntimeSection,
    openWorkspaceBrowser,
    providerDefaultModel,
    providerModelLabel,
    providerModelOptions,
    readUserAnswer,
    scrollToMessage,
    updateUserAnswer,
  }
}
