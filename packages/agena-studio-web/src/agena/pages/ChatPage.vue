<script setup lang="ts">
import { watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import {
  permissionActionView,
  permissionExplainability,
  permissionReplyPreview,
  permissionRiskLabel,
} from '@/agena/lib/permissionFormatting'
import { openGlobalCommandPalette } from '@/agena/lib/commandPaletteRegistry'
import ChatPageContent from './ChatPageContent.vue'
import { createChatPageContentState } from './chatPageContentModel'
import {
  messageBlocks,
  messageUsageFacts,
  readPayloadMessageId,
  readPayloadPartId,
} from './chatRenderModel'
import { useChatCommandState } from './useChatCommandState'
import { useChatDerivedState } from './useChatDerivedState'
import { useChatSessionActions } from './useChatSessionActions'
import { useChatSessionLifecycle } from './useChatSessionLifecycle'
import { useChatPageState } from './useChatPageState'
import { useChatPageUiState } from './useChatPageUiState'
import { formatUsageCount, formatUsageUsd } from './chatUsageModel'
import { useChatSidebarState } from './useChatSidebarState'

const route = useRoute()
const router = useRouter()
const {
  composer,
  continuing,
  errorMessage,
  interactiveRequestInFlight,
  inspectedMessage,
  inspectedMessageParts,
  inspectedPart,
  loading,
  localCommandNotice,
  messages,
  newSessionTitle,
  providerModels,
  providers,
  rewindCheckpoints,
  runtime,
  selectedAdapterId,
  selectedModelId,
  selectedProviderId,
  selectedThinkingMode,
  selectedSpeedMode,
  selectedVerbosity,
  selectedParallelToolCalls,
  selectedSessionId,
  selectedWorkspaceId,
  sending,
  sessionImportJsonl,
  sessionSearch,
  sessionState,
  sessions,
  sessionTree,
  timelineEvents,
  userInputDrafts,
  workspacePath,
  workspaces,
} = useChatPageState()

watch(
  () => [route.query.prompt, route.query.slash] as const,
  ([prompt, slash]) => {
    const promptText = typeof prompt === 'string' ? prompt.trim() : ''
    const slashText = typeof slash === 'string' ? slash.trim() : ''
    if (promptText) {
      composer.value = promptText
    } else if (slashText) {
      composer.value = slashText.startsWith('/') ? `${slashText} ` : `/${slashText} `
    } else {
      return
    }
    const nextQuery = { ...route.query }
    delete nextQuery.prompt
    delete nextQuery.slash
    void router.replace({ query: nextQuery })
  },
  { immediate: true },
)

const {
  formatEventTime,
  formatMessageTime,
  openRuntimeSection,
  openWorkspaceBrowser,
  providerAdapterOptions,
  providerDefaultAdapter,
  providerDefaultModel,
  providerModelLabel,
  providerModelOptions,
  modelThinkingModeOptions,
  modelSpeedModeOptions,
  readUserAnswer,
  scrollToMessage,
  updateUserAnswer,
  copySessionUsageSummary,
} = useChatPageUiState(
  {
    localCommandNotice,
    providerModels,
    providers,
    selectedAdapterId,
    selectedModelId,
    selectedProviderId,
    selectedThinkingMode,
    selectedSpeedMode,
    selectedVerbosity,
    selectedParallelToolCalls,
    selectedWorkspaceId,
    userInputDrafts,
    workspaces,
  },
  { router },
)

const {
  loadRewindCheckpoints,
  loadSessionsForWorkspace,
  loadSessionTree,
  loadSidebar,
  openSessionById,
  refreshConversation,
  selectSession,
  selectWorkspace,
  syncEventStream,
} = useChatSessionLifecycle({
  composer,
  errorMessage,
  loading,
  localCommandNotice,
  messages,
  providerModels,
  providers,
  rewindCheckpoints,
  route,
  runtime,
  selectedAdapterId,
  selectedModelId,
  selectedProviderId,
  selectedThinkingMode,
  selectedSpeedMode,
  selectedVerbosity,
  selectedParallelToolCalls,
  selectedSessionId,
  selectedWorkspaceId,
  sessionSearch,
  sessionState,
  sessions,
  sessionTree,
  timelineEvents,
  workspaces,
})

const {
  ancestorSessions,
  childSessions,
  contextUsageLabel,
  executionFacts,
  parentSession,
  rewindCheckpointFacts,
  selectedSession,
  selectedWorkspace,
  sessionLineageLabel,
  sessionTreeRows,
  sessionUsageHeadline,
  sessionUsageModelLines,
  sessionUsageSummary,
  sessionUsageSummaryFacts,
  siblingSessions,
} = useChatDerivedState({
  formatEventTime,
  messages,
  rewindCheckpoints,
  selectedSessionId,
  selectedWorkspaceId,
  sessionState,
  sessionTree,
  sessions,
  workspaces,
})

let commandPalette: ReturnType<typeof useChatCommandState>['commandPalette']
let slashSuggestions: ReturnType<typeof useChatCommandState>['slashSuggestions']

const {
  approvePermission,
  cancelCurrentSessionRun,
  cancelUserAnswers,
  clearSessionGoalAction,
  completeSessionGoalAction,
  continueCurrentSession,
  createSessionAction,
  deleteCurrentSession,
  exportCurrentSession,
  forkCurrentSession,
  importSessionFromJsonl,
  inspectMessage,
  isInteractiveRequestBusy,
  renameCurrentSession,
  resolveWorkspaceAction,
  rewindToMessage,
  sendPrompt,
  setSessionGoalAction,
  showSessionGoalAction,
  submitUserAnswers,
} = useChatSessionActions({
  confirm: (message) => (typeof window === 'undefined' ? false : window.confirm(message)),
  composer,
  continuing,
  errorMessage,
  interactiveRequestInFlight,
  inspectedMessage,
  inspectedMessageParts,
  inspectedPart,
  loading,
  localCommandNotice,
  messages,
  newSessionTitle,
  refreshConversation,
  runSlashCommand: (inputText) => commandPalette.runSlashCommand(inputText),
  selectedAdapterId,
  selectedModelId,
  selectedProviderId,
  selectedThinkingMode,
  selectedSpeedMode,
  selectedVerbosity,
  selectedParallelToolCalls,
  selectedSessionId,
  selectedWorkspaceId,
  sending,
  sessionImportJsonl,
  sessionState,
  syncEventStream,
  prompt: (message, defaultValue) => (typeof window === 'undefined' ? null : window.prompt(message, defaultValue)),
  userInputDrafts,
  workspacePath,
  loadSidebar,
  loadSessionsForWorkspace,
  selectSession,
  selectWorkspace,
})

;({ commandPalette, slashSuggestions } = useChatCommandState({
  routeRouter: router,
  runtime,
  selectedWorkspaceId,
  selectedSessionId,
  sessions,
  workspaces,
  sessionImportJsonl,
  sessionTreeRows,
  rewindCheckpoints,
  ancestorSessions,
  sessionUsageSummary,
  composer,
  localCommandNotice,
  newSessionTitle,
  workspacePath,
  actions: {
    openWorkspaceBrowser,
    openRuntimeSection,
    openSessionById,
    createSessionAction,
    continueCurrentSession,
    forkCurrentSession,
    exportCurrentSession,
    importSessionFromJsonl,
    selectWorkspace,
    resolveWorkspaceAction,
    showSessionGoalAction,
    setSessionGoalAction,
    completeSessionGoalAction,
    clearSessionGoalAction,
    loadSessionTree,
    loadRewindCheckpoints,
  },
}))

const sidebar = useChatSidebarState({
  createSessionAction,
  loadSessionsForWorkspace,
  loading,
  newSessionTitle,
  resolveWorkspaceAction,
  selectedSession,
  selectedSessionId,
  selectedWorkspace,
  selectedWorkspaceId,
  selectSession,
  selectWorkspace,
  sessionSearch,
  sessions,
  workspacePath,
  workspaces,
})

const pageContent = createChatPageContentState({
  ancestorSessions,
  approvePermission,
  cancelCurrentSessionRun,
  cancelUserAnswers,
  childSessions,
  contextUsageLabel,
  composer,
  continueCurrentSession,
  continuing,
  copySessionUsageSummary,
  deleteCurrentSession,
  executionFacts,
  exportCurrentSession,
  forkCurrentSession,
  formatEventTime,
  formatMessageTime,
  formatUsageCount,
  formatUsageUsd,
  importSessionFromJsonl,
  inspectMessage,
  isInteractiveRequestBusy,
  loadRewindCheckpoints,
  loadSessionTree,
  loading,
  messageBlocks,
  messages,
  messageUsageFacts,
  openGlobalCommandPalette,
  parentSession,
  permissionActionView,
  permissionExplainability,
  permissionReplyPreview,
  permissionRiskLabel,
  providerDefaultModel,
  providerModelLabel,
  providerModelOptions,
  modelThinkingModeOptions,
  modelSpeedModeOptions,
  providers,
  readPayloadMessageId,
  readPayloadPartId,
  readUserAnswer,
  refreshConversation,
  renameCurrentSession,
  rewindCheckpointFacts,
  rewindCheckpoints,
  rewindToMessage,
  scrollToMessage,
  selectedAdapterId,
  selectedModelId,
  selectedProviderId,
  selectedThinkingMode,
  selectedSpeedMode,
  selectedVerbosity,
  selectedSession,
  selectedWorkspace,
  sendPrompt,
  sending,
  sessionImportJsonl,
  sessionLineageLabel,
  sessionState,
  sessionTreeRows,
  sessionUsageHeadline,
  sessionUsageModelLines,
  sessionUsageSummary,
  sessionUsageSummaryFacts,
  sidebar,
  siblingSessions,
  slashSuggestions,
  submitUserAnswers,
  providerAdapterOptions,
  providerDefaultAdapter,
  timelineEvents,
  updateUserAnswer,
})
</script>

<template>
  <section class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Chat</h1>
        <p class="page-description">
          Drive agena sessions directly through the native HTTP API. No legacy compatibility layer remains.
        </p>
      </div>
      <div class="badge">{{ runtime?.provider_ids?.length || 0 }} provider(s)</div>
    </header>

    <div v-if="errorMessage" class="notice">{{ errorMessage }}</div>
    <div v-else-if="localCommandNotice" class="notice">{{ localCommandNotice }}</div>

    <ChatPageContent :state="pageContent" />
  </section>
</template>
