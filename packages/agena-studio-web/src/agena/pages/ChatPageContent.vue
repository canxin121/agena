<script setup lang="ts">
import ChatActiveSessionPanel from './ChatActiveSessionPanel.vue'
import ChatComposerPanel from './ChatComposerPanel.vue'
import ChatMessagesPanel from './ChatMessagesPanel.vue'
import ChatPendingPermissionsPanel from './ChatPendingPermissionsPanel.vue'
import ChatPendingUserInputPanel from './ChatPendingUserInputPanel.vue'
import ChatRewindCheckpointsPanel from './ChatRewindCheckpointsPanel.vue'
import ChatRunOptionsPanel from './ChatRunOptionsPanel.vue'
import ChatSessionTransferPanel from './ChatSessionTransferPanel.vue'
import ChatSessionTreePanel from './ChatSessionTreePanel.vue'
import ChatSidebarPanel from './ChatSidebarPanel.vue'
import ChatTimelinePanel from './ChatTimelinePanel.vue'
import ChatUsagePanel from './ChatUsagePanel.vue'
import type { ChatPageContentState } from './chatPageContentModel'

const props = defineProps<{
  state: ChatPageContentState
}>()
</script>

<template>
  <div class="split-layout">
    <ChatSidebarPanel
      :loading="props.state.sidebar.loading.value"
      :workspace-path="props.state.sidebar.workspacePath.value"
      :workspaces="props.state.sidebar.workspaces.value"
      :selected-workspace-id="props.state.sidebar.selectedWorkspaceId.value"
      :session-search="props.state.sidebar.sessionSearch.value"
      :new-session-title="props.state.sidebar.newSessionTitle.value"
      :sessions="props.state.sidebar.sessions.value"
      :selected-session-id="props.state.sidebar.selectedSessionId.value"
      :resolve-workspace-action="props.state.sidebar.resolveWorkspaceAction"
      :select-workspace="props.state.sidebar.selectWorkspace"
      :load-sessions-for-workspace="props.state.sidebar.loadSessionsForWorkspace"
      :create-session-action="props.state.sidebar.createSessionAction"
      :select-session="props.state.sidebar.selectSession"
      :format-message-time="props.state.formatMessageTime"
      @update:workspace-path="props.state.sidebar.workspacePath.value = $event"
      @update:session-search="props.state.sidebar.sessionSearch.value = $event"
      @update:new-session-title="props.state.sidebar.newSessionTitle.value = $event"
    />

    <section class="stack">
      <ChatActiveSessionPanel
        :selected-session="props.state.selectedSession.value"
        :selected-workspace="props.state.selectedWorkspace.value"
        :selected-session-id="props.state.sidebar.selectedSessionId.value"
        :loading="props.state.loading.value"
        :continuing="props.state.continuing.value"
        :session-state="props.state.sessionState.value"
        :session-lineage-label="props.state.sessionLineageLabel.value"
        :ancestor-sessions="props.state.ancestorSessions.value"
        :execution-facts="props.state.executionFacts.value"
        :session-usage-summary-facts="props.state.sessionUsageSummaryFacts.value"
        :sibling-sessions="props.state.siblingSessions.value"
        :child-sessions="props.state.childSessions.value"
        :parent-session="props.state.parentSession.value"
        :select-session="props.state.sidebar.selectSession"
        :rename-current-session="props.state.renameCurrentSession"
        :fork-current-session="props.state.forkCurrentSession"
        :delete-current-session="props.state.deleteCurrentSession"
        :export-current-session="props.state.exportCurrentSession"
        :continue-current-session="props.state.continueCurrentSession"
        :cancel-current-session-turn="props.state.cancelCurrentSessionTurn"
        :format-message-time="props.state.formatMessageTime"
      />

      <ChatSessionTreePanel
        :selected-session-id="props.state.sidebar.selectedSessionId.value"
        :session-tree-rows="props.state.sessionTreeRows.value"
        :ancestor-sessions="props.state.ancestorSessions.value"
        :load-session-tree="props.state.loadSessionTree"
        :select-session="props.state.sidebar.selectSession"
      />

      <ChatRewindCheckpointsPanel
        :selected-session-id="props.state.sidebar.selectedSessionId.value"
        :rewind-checkpoint-facts="props.state.rewindCheckpointFacts.value"
        :load-rewind-checkpoints="props.state.loadRewindCheckpoints"
        :unrewind-to-message="props.state.unrewindToMessage"
      />

      <ChatUsagePanel
        :selected-session-id="props.state.sidebar.selectedSessionId.value"
        :session-usage-headline="props.state.sessionUsageHeadline.value"
        :session-usage-summary-facts="props.state.sessionUsageSummaryFacts.value"
        :session-usage-summary="props.state.sessionUsageSummary.value"
        :session-usage-model-lines="props.state.sessionUsageModelLines.value"
        :format-usage-count="props.state.formatUsageCount"
        :format-usage-usd="props.state.formatUsageUsd"
        :copy-summary="() => props.state.copySessionUsageSummary(props.state.sessionUsageSummaryFacts.value)"
      />

      <ChatRunOptionsPanel
        :selected-provider-id="props.state.selectedProviderId.value"
        :selected-adapter-id="props.state.selectedAdapterId.value"
        :selected-model-id="props.state.selectedModelId.value"
        :selected-thinking-mode="props.state.selectedThinkingMode.value"
        :selected-speed-mode="props.state.selectedSpeedMode.value"
        :providers="props.state.providers.value"
        :provider-default-adapter="props.state.providerDefaultAdapter"
        :provider-default-model="props.state.providerDefaultModel"
        :provider-adapter-options="props.state.providerAdapterOptions"
        :provider-model-options="props.state.providerModelOptions"
        :provider-model-label="props.state.providerModelLabel"
        :model-thinking-mode-options="props.state.modelThinkingModeOptions"
        :model-speed-mode-options="props.state.modelSpeedModeOptions"
        @update:selected-provider-id="props.state.selectedProviderId.value = $event"
        @update:selected-adapter-id="props.state.selectedAdapterId.value = $event"
        @update:selected-model-id="props.state.selectedModelId.value = $event"
        @update:selected-thinking-mode="props.state.selectedThinkingMode.value = $event"
        @update:selected-speed-mode="props.state.selectedSpeedMode.value = $event"
      />

      <ChatSessionTransferPanel
        :loading="props.state.loading.value"
        :session-import-jsonl="props.state.sessionImportJsonl.value"
        :import-session-from-jsonl="props.state.importSessionFromJsonl"
        @update:session-import-jsonl="props.state.sessionImportJsonl.value = $event"
      />

      <ChatMessagesPanel
        :selected-session-id="props.state.sidebar.selectedSessionId.value"
        :loading="props.state.loading.value"
        :messages="props.state.messages.value"
        :inspected-message="props.state.inspectedMessage.value"
        :inspected-message-parts="props.state.inspectedMessageParts.value"
        :inspected-part="props.state.inspectedPart.value"
        :refresh-conversation="props.state.refreshConversation"
        :inspect-message="props.state.inspectMessage"
        :rewind-to-message="props.state.rewindToMessage"
        :format-message-time="props.state.formatMessageTime"
        :message-tags="props.state.messageTags"
        :message-usage-facts="props.state.messageUsageFacts"
        :message-blocks="props.state.messageBlocks"
      />

      <ChatTimelinePanel
        :timeline-events="props.state.timelineEvents.value"
        :format-message-time="props.state.formatMessageTime"
        :format-event-time="props.state.formatEventTime"
        :read-payload-message-id="props.state.readPayloadMessageId"
        :read-payload-part-id="props.state.readPayloadPartId"
        :inspect-message="props.state.inspectMessage"
        :scroll-to-message="props.state.scrollToMessage"
      />

      <ChatPendingPermissionsPanel
        :requests="props.state.sessionState.value?.pending_permission_requests ?? []"
        :permission-action-view="props.state.permissionActionView"
        :permission-risk-label="props.state.permissionRiskLabel"
        :permission-explainability="props.state.permissionExplainability"
        :permission-reply-preview="props.state.permissionReplyPreview"
        :approve-permission="props.state.approvePermission"
      />

      <ChatPendingUserInputPanel
        :requests="props.state.sessionState.value?.pending_user_input_requests ?? []"
        :read-user-answer="props.state.readUserAnswer"
        :update-user-answer="props.state.updateUserAnswer"
        :submit-user-answers="props.state.submitUserAnswers"
        :cancel-user-answers="props.state.cancelUserAnswers"
      />

      <ChatComposerPanel
        :composer="props.state.composer.value"
        :slash-suggestions="props.state.slashSuggestions.value"
        :sending="props.state.sending.value"
        :selected-session-id="props.state.sidebar.selectedSessionId.value"
        :open-palette="props.state.openGlobalCommandPalette"
        :send-prompt="props.state.sendPrompt"
        @update:composer="props.state.composer.value = $event"
      />
    </section>
  </div>
</template>
