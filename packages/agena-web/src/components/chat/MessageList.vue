<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AgenaLogo from '@/components/AgenaLogo.vue'
import Button from '@/components/ui/Button.vue'
import MobileSidebarEmptyState from '@/components/ui/MobileSidebarEmptyState.vue'
import MessageItem from '@/components/chat/MessageItem.vue'
import PartLoadingIndicator from '@/components/chat/PartLoadingIndicator.vue'
import AgenaInteractionPart from '@/components/chat/AgenaInteractionPart.vue'
import type { MessageLike, RenderBlock, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import type { AttentionEvent, MessageFold } from '@/types/chat'
import {
  partInteractionRequestIds,
  pendingInteractionPresentationFromAttention,
} from '@/pages/chat/transcriptPartPresentation'
import type { ActivityVisibility } from '@/pages/chat/transcriptActivityFolding'

const props = defineProps<{
  isCompactLayout: boolean
  isCompactTouch: boolean
  selectedSessionId: string | null
  messagesLoading: boolean
  messagesError: string | null
  renderBlocks: RenderBlock[]
  pendingInitialScrollSessionId: string | null
  loadingOlder: boolean
  activityPageSize: number
  foldLoadingByKey?: Record<string, boolean>
  foldErrorByKey?: Record<string, string>
  showTimestamps: boolean
  formatTime: (ms?: number) => string
  copiedMessageId: string
  revertBusyMessageId: string
  isStreamingAssistantMessage: (message: MessageLike) => boolean
  showRunLoading: boolean
  sessionEnded: boolean
  currentPhase: string
  activityCollapseSignal: number
  isPartExpanded: (part: TranscriptDisplayPart) => boolean
  isNodeSelected?: (key: string) => boolean
  isNodeSearchMatch?: (key: string) => boolean
  pendingAttention?: AttentionEvent | null
  openMobileSidebar?: () => void | Promise<void>
}>()

const emit = defineEmits<{
  (event: 'fork', messageId: string): void
  (event: 'revert', messageId: string): void
  (event: 'copy', message: MessageLike): void
  (event: 'partToggle', part: TranscriptDisplayPart, expanded: boolean): void
  (event: 'foldExpand', fold: MessageFold, all: boolean): void
  (event: 'nodeSelect', key: string): void
  (event: 'setActivityPageSize', size: number): void
  (event: 'revealParts'): void
}>()

const { t } = useI18n()
// Keep deliberate expansion when a session's keyed message subtree remounts.
const activityVisibility = reactive<Record<string, ActivityVisibility>>({})
watch(
  () => props.selectedSessionId,
  () => {
    for (const key of Object.keys(activityVisibility)) delete activityVisibility[key]
  },
)
function replyVisibility(id: string): ActivityVisibility {
  const key = `${props.selectedSessionId}:${id}`
  return (activityVisibility[key] ??= { ids: [] })
}

const durableInteractionRequestIds = computed(() => {
  const ids = new Set<string>()
  for (const block of props.renderBlocks) {
    for (const part of block.displayParts) {
      for (const requestId of partInteractionRequestIds(part)) ids.add(requestId)
    }
  }
  return ids
})

const pendingInteraction = computed(() =>
  pendingInteractionPresentationFromAttention(props.pendingAttention, durableInteractionRequestIds.value),
)

function forwardPartToggle(part: TranscriptDisplayPart, expanded: boolean) {
  emit('partToggle', part, expanded)
}

function forwardFoldExpand(fold: MessageFold, all: boolean) {
  emit('foldExpand', fold, all)
}
</script>

<template>
  <div
    v-if="!selectedSessionId"
    :class="isCompactLayout ? 'h-full min-h-[240px]' : 'py-16 text-center text-muted-foreground'"
  >
    <MobileSidebarEmptyState
      v-if="isCompactLayout"
      :title="t('chat.messages.empty.title')"
      :description="t('chat.messages.empty.description')"
      :action-label="t('chat.messages.empty.actionLabel')"
      :show-action="true"
      @action="openMobileSidebar?.()"
    >
      <template #icon>
        <AgenaLogo class="mx-auto mb-3 h-12 w-12" aria-hidden="true" />
      </template>
    </MobileSidebarEmptyState>
    <template v-else>
      <AgenaLogo class="mx-auto h-12 w-12" aria-hidden="true" />
      <div class="typography-ui-label mt-3 font-semibold">{{ t('chat.messages.empty.title') }}</div>
      <div class="typography-meta mt-1">{{ t('chat.messages.empty.desktopDescription') }}</div>
    </template>
  </div>

  <div v-else-if="messagesLoading && !renderBlocks.length && !pendingInteraction" class="space-y-6 py-8 animate-pulse">
    <div v-for="index in 3" :key="index" class="space-y-2">
      <div class="ml-2 h-3 w-28 bg-muted/35" />
      <div class="ml-4 h-3 bg-muted/25" :class="index === 2 ? 'w-2/3' : 'w-5/6'" />
      <div class="ml-4 h-3 w-1/2 bg-muted/20" />
    </div>
  </div>

  <div v-else-if="messagesError && !renderBlocks.length && !pendingInteraction" class="px-2">
    <div class="transcript-content border-rose-500/60 py-2 text-sm text-rose-700 dark:text-rose-300">
      {{ messagesError }}
    </div>
  </div>

  <template v-else>
    <p v-if="messagesError" role="alert" class="px-2 py-1 text-sm text-destructive">{{ messagesError }}</p>
    <div
      :key="selectedSessionId || 'none'"
      class="space-y-1 transition-opacity duration-150 ease-out"
      :class="pendingInitialScrollSessionId ? 'pointer-events-none opacity-0' : ''"
      data-transcript-root="true"
    >
      <template v-for="block in renderBlocks" :key="block.key">
        <MessageItem
          v-if="block.kind === 'message'"
          :message="block.message"
          :display-parts="block.displayParts"
          :show-timestamps="showTimestamps"
          :format-time="formatTime"
          :copied-message-id="copiedMessageId"
          :revert-busy-message-id="revertBusyMessageId"
          :is-streaming="isStreamingAssistantMessage(block.message)"
          :collapse-signal="activityCollapseSignal"
          :activity-page-size="activityPageSize"
          :activity-visibility="replyVisibility(String(block.message.info.id))"
          :fold-loading-by-key="foldLoadingByKey"
          :fold-error-by-key="foldErrorByKey"
          :is-compact-touch="isCompactTouch"
          :is-part-expanded="isPartExpanded"
          :is-node-selected="isNodeSelected"
          :is-node-search-match="isNodeSearchMatch"
          :session-id="selectedSessionId"
          @fork="$emit('fork', $event)"
          @revert="$emit('revert', $event)"
          @copy="$emit('copy', $event)"
          @part-toggle="forwardPartToggle"
          @fold-expand="forwardFoldExpand"
          @node-select="$emit('nodeSelect', $event)"
          @set-activity-page-size="$emit('setActivityPageSize', $event)"
          @reveal-parts="$emit('revealParts')"
        />
      </template>

      <aside v-if="pendingInteraction" :key="pendingInteraction.requestId" class="px-2 py-1">
        <AgenaInteractionPart
          :session-id="selectedSessionId"
          :interaction="pendingInteraction.interaction"
          :permission="pendingInteraction.permission"
        />
      </aside>
      <PartLoadingIndicator v-if="showRunLoading" role="status" class="min-h-8 px-2 py-1" />
    </div>
  </template>
</template>
