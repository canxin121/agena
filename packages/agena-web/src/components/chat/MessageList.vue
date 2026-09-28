<script setup lang="ts">
import { computed } from 'vue'
import { RiCheckLine, RiLoader4Line, RiSparkling2Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import Button from '@/components/ui/Button.vue'
import ToolbarChipButton from '@/components/ui/ToolbarChipButton.vue'
import MobileSidebarEmptyState from '@/components/ui/MobileSidebarEmptyState.vue'
import MessageItem from '@/components/chat/MessageItem.vue'
import AgenaTranscriptPart from '@/components/chat/AgenaTranscriptPart.vue'
import type {
  MessageLike,
  RenderBlock,
  RetryStatusLike,
  SessionErrorLike,
  TranscriptDisplayPart,
} from '@/components/chat/messageList.types'
import type { AttentionEvent, MessageFold } from '@/types/chat'
import { formatTimeHMS } from '@/i18n/intl'
import type { OptimisticUserMessage } from '@/composables/chat/useMessageStreaming'
import { partInteractionRequestIds, pendingInteractionPartSource } from '@/pages/chat/transcriptPartPresentation'
import { optimisticUserParts, projectLocalPart } from '@/pages/chat/transcriptProjection'

const props = defineProps<{
  isCompactLayout: boolean
  isCompactTouch: boolean
  selectedSessionId: string | null
  messagesLoading: boolean
  messagesError: string | null
  sessionError?: SessionErrorLike
  renderBlocks: RenderBlock[]
  pendingInitialScrollSessionId: string | null
  loadingOlder: boolean
  activityPageSize: number
  showTimestamps: boolean
  formatTime: (ms?: number) => string
  copiedMessageId: string
  revertBusyMessageId: string
  isStreamingAssistantMessage: (message: MessageLike) => boolean
  showAssistantPlaceholder: boolean
  sessionEnded: boolean
  retryStatus: RetryStatusLike
  currentPhase: string
  awaitingAssistant: boolean
  activityCollapseSignal: number
  isPartExpanded: (part: TranscriptDisplayPart) => boolean
  isNodeSelected?: (key: string) => boolean
  isNodeSearchMatch?: (key: string) => boolean
  optimisticUser: OptimisticUserMessage | null
  showOptimisticUser: boolean
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
  (event: 'copySessionError'): void
  (event: 'clearSessionError'): void
  (event: 'setActivityPageSize', size: number): void
}>()

const { t } = useI18n()

const durableInteractionRequestIds = computed(() => {
  const ids = new Set<string>()
  for (const block of props.renderBlocks) {
    for (const part of block.displayParts) {
      for (const requestId of partInteractionRequestIds(part)) ids.add(requestId)
    }
  }
  return ids
})

const pendingInteractionFallback = computed(() =>
  pendingInteractionPartSource(props.pendingAttention, durableInteractionRequestIds.value),
)

// The outstanding-request row is a part projection too: it carries the
// durable tool_call shape, so the persistent part that replaces it renders
// through exactly the same path (and the same label vocabulary).
const pendingInteractionPart = computed<TranscriptDisplayPart | null>(() => {
  const source = pendingInteractionFallback.value
  if (!source) return null
  return projectLocalPart(
    {
      id: `interaction:${source.requestId}`,
      type: 'tool',
      partState: 'pending',
      agenaKind: 'tool_call',
      agenaRole: 'assistant',
      agenaContent: { call_id: 0, name: 'interaction', metadata: {}, ...source.content },
    },
    'assistant',
  )
})

// The pending user turn follows the same canonical part projection as the
// persisted transcript. It is temporary, but it must not be a second prose
// renderer that disappears/reappears with a different shape after the server
// acknowledges the user_send run.
const optimisticDisplayParts = computed<TranscriptDisplayPart[]>(() => {
  const message = props.optimisticUser
  if (!message) return []
  if (message.parts.length) return message.parts
  return optimisticUserParts({
    key: message.key,
    text: message.text,
    files: message.files,
    status: message.status,
    fileFallbackLabel: String(t('chat.messageItem.fileFallback')).trim(),
    attachmentTitle: String(t('chat.attachments.rowTitle')).trim(),
  })
})

const assistantPlaceholderPart = computed<TranscriptDisplayPart>(() => {
  // The in-flight assistant row is a part projection like every other row:
  // the lifecycle part the durable projection uses, in its running state,
  // so this element shares one label vocabulary with the server-rendered one.
  // Once the accepted send reports the reply id, the live row already renders
  // under the identity the durable row will use.
  const replyId = props.optimisticUser?.replyId || ''
  return projectLocalPart(
    {
      id: replyId ? `lifecycle:${replyId}` : 'lifecycle:pending',
      type: 'tool',
      partState: 'in_progress',
      agenaKind: 'assistant_reply_lifecycle',
      agenaRole: 'assistant',
      agenaContent: { state: 'in_progress' },
    },
    'assistant',
  )
})

const sessionErrorPart = computed<TranscriptDisplayPart | null>(() => {
  const error = props.sessionError
  if (!error) return null
  // The session error row is a part projection as well: one runtime error
  // element, with its classification carried as presentation metadata so the
  // label vocabulary lives in the projection instead of the template.
  const body = sessionErrorBody()
  return projectLocalPart(
    {
      id: 'session-error',
      type: 'tool',
      partState: 'failed',
      agenaKind: 'error',
      agenaRole: 'runtime',
      agenaContent: {
        message: body,
        classification: String(error.error?.classification || '') || null,
      },
      agenaPresentation: { title: sessionErrorClassificationLabel(), summary: body },
    },
    'runtime',
  )
})

function sessionErrorClassificationLabel(): string {
  const classification = String(props.sessionError?.error?.classification || '').trim()
  if (classification === 'context_overflow') return String(t('chat.sessionError.classification.contextOverflow'))
  if (classification === 'provider_auth') return String(t('chat.sessionError.classification.providerAuth'))
  if (classification === 'network') return String(t('chat.sessionError.classification.network'))
  if (classification === 'provider_api') return String(t('chat.sessionError.classification.providerApi'))
  return String(t('chat.sessionError.classification.sessionError'))
}

function sessionErrorBody(): string {
  const detail = props.sessionError?.error
  return String(detail?.rendered || detail?.message || detail?.code || t('chat.sessionError.body.default')).trim()
}

function sessionErrorAtLabel(): string {
  const at = Number(props.sessionError?.at || 0)
  return Number.isFinite(at) && at > 0 ? formatTimeHMS(at) : ''
}

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
    />
    <template v-else>
      <RiSparkling2Line class="mx-auto h-8 w-8 opacity-25" />
      <div class="typography-ui-label mt-3 font-semibold">{{ t('chat.messages.empty.title') }}</div>
      <div class="typography-meta mt-1">{{ t('chat.messages.empty.desktopDescription') }}</div>
    </template>
  </div>

  <div v-else-if="messagesLoading" class="space-y-6 py-8 animate-pulse">
    <div v-for="index in 3" :key="index" class="space-y-2">
      <div class="h-3 w-28 bg-muted/35" />
      <div class="ml-7 h-3 bg-muted/25" :class="index === 2 ? 'w-2/3' : 'w-5/6'" />
      <div class="ml-7 h-3 w-1/2 bg-muted/20" />
    </div>
  </div>

  <div
    v-else-if="messagesError"
    class="border-l-2 border-rose-500/60 py-2 pl-3 text-sm text-rose-700 dark:text-rose-300"
  >
    {{ messagesError }}
  </div>

  <template v-else>
    <div v-if="loadingOlder" class="mb-2 flex items-center gap-2 px-1 text-[11px] text-muted-foreground">
      <RiLoader4Line class="h-3.5 w-3.5 animate-spin" />
      {{ t('chat.messages.loadingOlder') }}
    </div>

    <TransitionGroup
      :key="selectedSessionId || 'none'"
      :name="pendingInitialScrollSessionId ? '' : 'chatlist'"
      tag="div"
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
        />
      </template>

      <!--
        Pending requests are state-driven and can briefly exist before the
        durable tool_call part reaches the transcript stream. Render the same
        interaction component from that canonical attention state so a page
        refresh is never required. Once the part arrives, request-id de-duping
        above removes this temporary projection.
      -->
      <article
        v-if="pendingInteractionFallback"
        :key="`pending-interaction:${pendingInteractionFallback.requestId}`"
        class="ml-7 min-w-0 rounded-r-md border-l-2 border-primary/45 py-1 pl-3"
        data-transcript-node="interaction"
        :data-interaction-request-id="pendingInteractionFallback.requestId"
      >
        <AgenaTranscriptPart
          :part="pendingInteractionPart!"
          :expanded="true"
          :collapse-signal="activityCollapseSignal"
          :session-id="selectedSessionId"
        />
      </article>

      <article v-if="optimisticUser && showOptimisticUser" :key="optimisticUser.key" class="py-2">
        <header class="flex min-h-6 items-center gap-2 px-1 text-[11px] text-muted-foreground">
          <span class="font-semibold text-primary">user</span>
          <span v-if="showTimestamps">{{ formatTime(optimisticUser.createdAt) }}</span>
          <span class="inline-flex items-center gap-1 font-mono">
            <template v-if="optimisticUser.status === 'sending'">
              <RiLoader4Line class="h-3.5 w-3.5 animate-spin" />
              {{ t('chat.messages.optimistic.sending') }}
            </template>
            <template v-else>
              <RiCheckLine class="h-3.5 w-3.5 text-emerald-500" />
              {{ t('chat.messages.optimistic.sent') }}
            </template>
          </span>
        </header>
        <div class="mt-0.5 border-l-2 border-primary/35 py-1 pl-7 text-sm leading-relaxed">
          <AgenaTranscriptPart
            v-for="part in optimisticDisplayParts"
            :key="part.key"
            :part="part"
            :expanded="true"
            :collapse-signal="activityCollapseSignal"
            :session-id="selectedSessionId"
          />
        </div>
      </article>

      <article v-if="showAssistantPlaceholder" key="assistant-placeholder" class="py-2">
        <header class="flex min-h-6 items-center gap-2 px-1 text-[11px] text-muted-foreground">
          <span class="font-semibold text-emerald-700 dark:text-emerald-300">assistant</span>
          <RiLoader4Line class="h-3.5 w-3.5 animate-spin text-primary" />
        </header>
        <AgenaTranscriptPart
          :part="assistantPlaceholderPart"
          :expanded="true"
          :collapse-signal="activityCollapseSignal"
          :session-id="selectedSessionId"
        />
      </article>
    </TransitionGroup>

    <article v-if="sessionError" class="mt-3 py-2">
      <header class="flex min-h-6 items-center gap-2 px-1 text-[11px] text-muted-foreground">
        <span class="font-semibold text-rose-700 dark:text-rose-300">system</span>
        <span v-if="sessionErrorAtLabel()" class="font-mono text-[10px]">{{ sessionErrorAtLabel() }}</span>
      </header>
      <div class="ml-7 rounded-r-md border-l-2 border-rose-500/60 py-1 pl-3 text-sm text-rose-800 dark:text-rose-200">
        <AgenaTranscriptPart
          :part="sessionErrorPart!"
          :expanded="true"
          :collapse-signal="activityCollapseSignal"
          :session-id="selectedSessionId"
        />
        <div class="mt-2 flex items-center gap-2" data-transcript-chrome="true">
          <ToolbarChipButton
            :tooltip="t('chat.sessionError.actions.copyDetails')"
            :title="t('chat.sessionError.actions.copyDetails')"
            :aria-label="t('chat.sessionError.actions.copyDetails')"
            @click="$emit('copySessionError')"
          >
            {{ t('chat.sessionError.actions.copyDetails') }}
          </ToolbarChipButton>
          <Button size="sm" variant="ghost" @click="$emit('clearSessionError')">{{
            t('chat.sessionError.actions.dismiss')
          }}</Button>
        </div>
      </div>
    </article>
  </template>
</template>
