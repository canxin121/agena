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
import { messageBlocks, messageUsageFacts, readPayloadMessageId, readPayloadPartId } from './chatRenderModel'
import { useChatCommandState } from './useChatCommandState'
import { useChatDerivedState } from './useChatDerivedState'
import { useChatSessionActions } from './useChatSessionActions'
import { useChatSessionLifecycle } from './useChatSessionLifecycle'
import { useChatPageState } from './useChatPageState'
import { useChatPageUiState } from './useChatPageUiState'
import { formatUsageCount, formatUsageUsd } from './chatUsageModel'
import {
  createComposerAttachmentDraft,
  MAX_COMPOSER_ATTACHMENT_TOTAL_BYTES,
  MAX_COMPOSER_ATTACHMENTS,
  validateComposerAttachment,
} from './chatAttachmentModel'
import { useChatSidebarState } from './useChatSidebarState'

const route = useRoute()
const router = useRouter()
const {
  attachments,
  attachmentLoading,
  composer,
  composerQueue,
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
  queueDraining,
  rewindCheckpoints,
  runtime,
  selectedAdapterId,
  selectedModelId,
  selectedProviderId,
  selectedThinkingMode,
  selectedSpeedMode,
  selectedVerbosity,
  selectedParallelToolCalls,
  selectedTemperature,
  selectedMaxOutput,
  selectedSystemPrompt,
  selectedSessionId,
  selectedWorkspaceId,
  sending,
  sessionImportJsonl,
  sessionSearch,
  sessionViewMode,
  sessionState,
  sessions,
  sessionTree,
  timelineEvents,
  userInputDrafts,
  workspacePath,
  workspaces,
} = useChatPageState()

async function addComposerFiles(files: File[], imageOnly = false) {
  if (!files.length) return
  errorMessage.value = ''
  const available = Math.max(0, MAX_COMPOSER_ATTACHMENTS - attachments.value.length)
  if (!available) {
    errorMessage.value = `A maximum of ${MAX_COMPOSER_ATTACHMENTS} attachments can be sent at once.`
    return
  }
  const selectedFiles = files.slice(0, available)
  const totalBytes =
    attachments.value.reduce((total, attachment) => total + attachment.size, 0) +
    selectedFiles.reduce((total, file) => total + file.size, 0)
  if (totalBytes > MAX_COMPOSER_ATTACHMENT_TOTAL_BYTES) {
    errorMessage.value = 'Attachments for one message cannot exceed 64 MB in total.'
    return
  }
  attachmentLoading.value = true
  try {
    const next = []
    for (const file of selectedFiles) {
      const validationError = validateComposerAttachment(file, imageOnly)
      if (validationError) throw new Error(validationError)
      next.push(await createComposerAttachmentDraft(file))
    }
    attachments.value = [...attachments.value, ...next]
    if (files.length > available) {
      errorMessage.value = `Only the first ${available} attachment(s) were added; the limit is ${MAX_COMPOSER_ATTACHMENTS}.`
    }
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error)
  } finally {
    attachmentLoading.value = false
  }
}

function removeComposerAttachment(id: string) {
  attachments.value = attachments.value.filter((attachment) => attachment.id !== id)
}

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
  modelThinkingModeOptions,
  modelSpeedModeOptions,
  modelVerbosityOptions,
  modelParallelToolCallsOptions,
  readUserAnswer,
  scrollToMessage,
  updateUserAnswer,
  copySessionUsageSummary,
  copyText,
  createCommit,
  createPullRequest,
  downloadWorkspaceFile,
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
  loadSessionTimeline,
  loadSidebar,
  openSessionById,
  refreshConversation,
  selectSession,
  selectWorkspace,
  setSessionViewMode,
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
  selectedTemperature,
  selectedMaxOutput,
  selectedSystemPrompt,
  selectedSessionId,
  selectedWorkspaceId,
  sessionSearch,
  sessionViewMode,
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
  askAside,
  cancelCurrentSessionRun,
  cancelUserAnswers,
  clearComposerQueue,
  clearSessionGoalAction,
  compactCurrentSession,
  completeSessionGoalAction,
  continueCurrentSession,
  createSessionAction,
  deleteCurrentSession,
  drainComposerQueue,
  exportCurrentSession,
  forkCurrentSession,
  importSessionFromJsonl,
  inspectMessage,
  isInteractiveRequestBusy,
  popComposerQueue,
  renameCurrentSession,
  resolveWorkspaceAction,
  rewindToMessage,
  sendPrompt,
  setSessionGoalAction,
  showSessionGoalAction,
  submitUserAnswers,
} = useChatSessionActions({
  attachments,
  composerQueue,
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
  selectedTemperature,
  selectedMaxOutput,
  selectedSystemPrompt,
  selectedSessionId,
  selectedWorkspaceId,
  sending,
  queueDraining,
  sessionImportJsonl,
  sessionState,
  sessions,
  syncEventStream,
  prompt: (message, defaultValue) => (typeof window === 'undefined' ? null : window.prompt(message, defaultValue)),
  userInputDrafts,
  workspacePath,
  loadSidebar,
  loadSessionsForWorkspace,
  selectSession,
  selectWorkspace,
})

watch(
  () =>
    [sessionState.value?.run_state, sessionState.value?.blocked, sending.value, composerQueue.value.length] as const,
  ([runState, blocked, isSending, queueLength]) => {
    if (runState === 'idle' && !blocked && !isSending && queueLength) void drainComposerQueue()
  },
)
;({ commandPalette, slashSuggestions } = useChatCommandState({
  routeRouter: router,
  runtime,
  selectedWorkspaceId,
  selectedSessionId,
  sessions,
  messages,
  composerQueue,
  timelineEvents,
  workspaces,
  sessionImportJsonl,
  sessionTreeRows,
  rewindCheckpoints,
  ancestorSessions,
  childSessions,
  parentSession,
  sessionState,
  sessionUsageSummary,
  composer,
  localCommandNotice,
  newSessionTitle,
  workspacePath,
  sessionSearch,
  actions: {
    approvePermission,
    askAside,
    clearComposerQueue,
    copyText,
    createCommit,
    createPullRequest,
    downloadWorkspaceFile,
    forgetMemory,
    focusComposer,
    focusTranscript,
    focusRunOptions,
    openCommandPalette: openGlobalCommandPalette,
    openAttachmentPicker,
    openMemorySettings,
    openPermissionSettings,
    popComposerQueue,
    openWorkspaceBrowser,
    openSnapshotInspector,
    openRuntimeSection,
    openSessionById,
    createSessionAction,
    continueCurrentSession,
    compactCurrentSession,
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
    loadSessionTimeline,
    loadRewindCheckpoints,
    refreshConversation,
    renameCurrentSession,
    selectSession,
    setSessionViewMode,
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
  sessionViewMode,
  sessions,
  setSessionViewMode,
  workspacePath,
  workspaces,
})

const pageContent = createChatPageContentState({
  addComposerFiles,
  attachments,
  attachmentLoading,
  ancestorSessions,
  approvePermission,
  cancelCurrentSessionRun,
  cancelUserAnswers,
  clearComposerQueue,
  childSessions,
  contextUsageLabel,
  composer,
  composerQueue,
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
  inspectedMessage,
  inspectedMessageParts,
  inspectedPart,
  isInteractiveRequestBusy,
  loadRewindCheckpoints,
  loadSessionTree,
  loading,
  messageBlocks,
  messages,
  messageUsageFacts,
  openGlobalCommandPalette,
  parentSession,
  popComposerQueue,
  permissionActionView,
  permissionExplainability,
  permissionReplyPreview,
  permissionRiskLabel,
  providerDefaultModel,
  providerModelLabel,
  providerModelOptions,
  modelThinkingModeOptions,
  modelSpeedModeOptions,
  modelVerbosityOptions,
  modelParallelToolCallsOptions,
  providers,
  readPayloadMessageId,
  readPayloadPartId,
  readUserAnswer,
  removeComposerAttachment,
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
  selectedParallelToolCalls,
  selectedTemperature,
  selectedMaxOutput,
  selectedSystemPrompt,
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
