<script setup lang="ts">
import { computed } from 'vue'
import { RiCheckLine, RiLoader4Line, RiSparkling2Line, RiTimeLine } from '@remixicon/vue'
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
import type { MessageFold } from '@/types/chat'
import { formatTimeHMS } from '@/i18n/intl'
import type { OptimisticUserMessage } from '@/composables/chat/useMessageStreaming'

const props = defineProps<{
  isCompactLayout: boolean
  selectedSessionId: string | null
  messagesLoading: boolean
  messagesError: string | null
  sessionError?: SessionErrorLike
  renderBlocks: RenderBlock[]
  pendingInitialScrollSessionId: string | null
  loadingOlder: boolean
  showTimestamps: boolean
  formatTime: (ms?: number) => string
  copiedMessageId: string
  revertBusyMessageId: string
  isStreamingAssistantMessage: (message: MessageLike) => boolean
  showAssistantPlaceholder: boolean
  revertMarkerBusy: boolean
  sessionEnded: boolean
  retryStatus: RetryStatusLike
  currentPhase: string
  awaitingAssistant: boolean
  activityCollapseSignal: number
  activityExpandAllSignal: number
  isPartExpanded: (part: TranscriptDisplayPart) => boolean
  isNodeSelected?: (key: string) => boolean
  isNodeSearchMatch?: (key: string) => boolean
  optimisticUser: OptimisticUserMessage | null
  showOptimisticUser: boolean
  openMobileSidebar?: () => void | Promise<void>
}>()

const emit = defineEmits<{
  (event: 'fork', messageId: string): void
  (event: 'revert', messageId: string): void
  (event: 'copy', message: MessageLike): void
  (event: 'partToggle', part: TranscriptDisplayPart, expanded: boolean): void
  (event: 'foldExpand', fold: MessageFold, all: boolean): void
  (event: 'nodeSelect', key: string): void
  (event: 'redoFromRevert'): void
  (event: 'unrevertFromRevert'): void
  (event: 'copySessionError'): void
  (event: 'clearSessionError'): void
  (event: 'expandAll'): void
}>()

const { t } = useI18n()

const hasExpandableTranscript = computed(() =>
  props.renderBlocks.some(
    (block) =>
      block.kind === 'message' &&
      (Boolean(block.message.folds?.length) ||
        block.displayParts.some((part) => part.toggleable) ||
        block.displayParts.filter((part) => part.kind !== 'text' && part.kind !== 'answer').length > 5),
  ),
)

// The pending user turn follows the same canonical part projection as the
// persisted transcript. It is temporary, but it must not be a second prose
// renderer that disappears/reappears with a different shape after the server
// acknowledges the user_send run.
const optimisticDisplayParts = computed<TranscriptDisplayPart[]>(() => {
  const message = props.optimisticUser
  if (!message) return []
  const status = message.status === 'sending' ? 'in_progress' : 'completed'
  const parts: TranscriptDisplayPart[] = []
  if (message.text.trim()) {
    parts.push({
      key: `${message.key}:text`,
      id: `${message.key}:text`,
      kind: 'text',
      status,
      role: 'user',
      source: {
        id: `${message.key}:text`,
        type: 'text',
        partState: status,
        agenaKind: 'text',
        agenaRole: 'user',
        text: message.text,
        agenaContent: { text: message.text },
      },
      title: '',
      summary: '',
      copyText: message.text,
      toggleable: false,
      defaultExpanded: true,
    })
  }
  for (const [index, file] of message.files.entries()) {
    const id = `${message.key}:file:${index}`
    const label = String(file.filename || file.serverPath || file.url || t('chat.messageItem.fileFallback')).trim()
    parts.push({
      key: id,
      id,
      kind: 'resource',
      status,
      role: 'user',
      source: {
        id,
        type: 'file',
        partState: status,
        agenaKind: 'file_ref',
        agenaRole: 'user',
        ...(file.filename ? { filename: file.filename } : {}),
        ...(file.mime ? { mime: file.mime } : {}),
        ...(file.url ? { url: file.url } : {}),
        ...(file.serverPath ? { serverPath: file.serverPath } : {}),
        agenaContent: {
          ...(file.filename ? { name: file.filename } : {}),
          ...(file.mime ? { mime: file.mime } : {}),
          ...(file.url ? { url: file.url } : {}),
          ...(file.serverPath ? { path: file.serverPath } : {}),
        },
      },
      title: 'Attachment',
      summary: label,
      copyText: label,
      toggleable: false,
      defaultExpanded: true,
    })
  }
  return parts
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
    <div v-if="hasExpandableTranscript" class="mb-1 flex justify-end px-1">
      <button
        type="button"
        class="rounded-md px-2 py-1 font-mono text-[10px] text-muted-foreground hover:bg-muted/35 hover:text-foreground"
        data-transcript-expand-all="true"
        @click="emit('expandAll')"
      >
        {{ t('chat.messages.activity.expandAll') }}
      </button>
    </div>
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
          :expand-all-signal="activityExpandAllSignal"
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
        />

        <div v-else class="rounded-md border border-border/60 px-3 py-2 text-sm" data-transcript-node="revert">
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="font-medium text-muted-foreground">
                {{
                  block.revert.revertedUserCount === 1
                    ? t('chat.revertMarker.revertedMessageCountOne')
                    : t('chat.revertMarker.revertedMessageCountMany', { count: block.revert.revertedUserCount })
                }}
              </div>
              <div class="mt-0.5 font-mono text-[11px] text-muted-foreground/70">
                {{ t('chat.revertMarker.boundaryLine', { id: block.revert.messageID }) }}
              </div>
            </div>
            <div class="flex shrink-0 items-center gap-2">
              <Button size="sm" variant="ghost" :disabled="revertMarkerBusy" @click="$emit('redoFromRevert')">
                <RiLoader4Line v-if="revertMarkerBusy" class="h-4 w-4 animate-spin" />
                <span v-else>{{ t('chat.revertMarker.redo') }}</span>
              </Button>
              <Button size="sm" variant="ghost" :disabled="revertMarkerBusy" @click="$emit('unrevertFromRevert')">
                {{ t('chat.revertMarker.restoreAll') }}
              </Button>
            </div>
          </div>
        </div>
      </template>

      <article v-if="optimisticUser && showOptimisticUser" :key="optimisticUser.key" class="py-2">
        <header class="flex min-h-6 items-center gap-2 px-1 text-[11px] text-muted-foreground">
          <span class="font-semibold text-primary">user</span>
          <span v-if="showTimestamps">{{ formatTime(optimisticUser.createdAt) }}</span>
          <span class="inline-flex items-center gap-1 font-mono">
            <template v-if="optimisticUser.status === 'sending'">
              <RiLoader4Line class="h-3.5 w-3.5 animate-spin" />
              {{ t('chat.messages.optimistic.sending') }}
            </template>
            <template v-else-if="optimisticUser.status === 'queued'">
              <RiTimeLine class="h-3.5 w-3.5" />
              {{ t('chat.messages.optimistic.queued') }}
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
        <div class="ml-7 flex items-center gap-2 py-1 text-[13px] text-muted-foreground">
          <span class="font-mono text-primary">▸ ⠋</span>
          <span class="font-semibold">Response running</span>
        </div>
      </article>
    </TransitionGroup>

    <article v-if="sessionError" class="mt-3 py-2">
      <header class="flex min-h-6 items-center gap-2 px-1 text-[11px] text-muted-foreground">
        <span class="font-semibold text-rose-700 dark:text-rose-300">system</span>
        <span v-if="sessionErrorAtLabel()" class="font-mono text-[10px]">{{ sessionErrorAtLabel() }}</span>
      </header>
      <div class="ml-7 rounded-r-md border-l-2 border-rose-500/60 py-1 pl-3 text-sm text-rose-800 dark:text-rose-200">
        <div class="font-semibold">{{ sessionErrorClassificationLabel() }}</div>
        <div class="mt-1 break-words">{{ sessionErrorBody() }}</div>
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
