<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router'

import { permissionActionView, permissionExplainability, permissionReplyPreview, permissionRiskLabel } from '@/agena/lib/permissionFormatting'
import { openGlobalCommandPalette } from '@/agena/lib/commandPaletteRegistry'
import ChatPageContent from './ChatPageContent.vue'
import { createChatPageContentState } from './chatPageContentModel'
import { messageBlocks, messageTags, messageUsageFacts, readPayloadMessageId } from './chatRenderModel'
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
  loading,
  localCommandNotice,
  messages,
  newSessionTitle,
  providerModels,
  providers,
  rewindCheckpoints,
  runtime,
  selectedModelId,
  selectedProviderId,
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

const {
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
  copySessionUsageSummary,
} = useChatPageUiState(
  {
    localCommandNotice,
    providerModels,
    providers,
    selectedModelId,
    selectedProviderId,
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
  selectedModelId,
  selectedProviderId,
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
  cancelUserAnswers,
  continueCurrentSession,
  createSessionAction,
  exportCurrentSession,
  forkCurrentSession,
  importSessionFromJsonl,
  resolveWorkspaceAction,
  rewindToMessage,
  sendPrompt,
  submitUserAnswers,
} = useChatSessionActions({
  composer,
  continuing,
  errorMessage,
  loading,
  localCommandNotice,
  messages,
  newSessionTitle,
  refreshConversation,
  runSlashCommand: (inputText) => commandPalette.runSlashCommand(inputText),
  selectedModelId,
  selectedProviderId,
  selectedSessionId,
  selectedWorkspaceId,
  sending,
  sessionImportJsonl,
  sessionState,
  syncEventStream,
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
  cancelUserAnswers,
  childSessions,
  composer,
  continueCurrentSession,
  continuing,
  copySessionUsageSummary,
  executionFacts,
  exportCurrentSession,
  forkCurrentSession,
  formatEventTime,
  formatMessageTime,
  formatUsageCount,
  formatUsageUsd,
  importSessionFromJsonl,
  loadRewindCheckpoints,
  loadSessionTree,
  loading,
  messageBlocks,
  messageTags,
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
  providers,
  readPayloadMessageId,
  readUserAnswer,
  refreshConversation,
  rewindCheckpointFacts,
  rewindCheckpoints,
  rewindToMessage,
  scrollToMessage,
  selectedModelId,
  selectedProviderId,
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
