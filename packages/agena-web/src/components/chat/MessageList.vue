<script setup lang="ts">
import { RiCheckLine, RiFileLine, RiLoader4Line, RiSparkling2Line, RiTimeLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import Button from '@/components/ui/Button.vue'
import ToolbarChipButton from '@/components/ui/ToolbarChipButton.vue'
import MobileSidebarEmptyState from '@/components/ui/MobileSidebarEmptyState.vue'
import MessageItem from '@/components/chat/MessageItem.vue'
import type {
  AttentionLike,
  MessageLike,
  RenderBlock,
  RetryStatusLike,
  SessionErrorLike,
  TranscriptDisplayPart,
} from '@/components/chat/messageList.types'
import { formatTimeHMS } from '@/i18n/intl'
import type { OptimisticUserMessage } from '@/composables/chat/useMessageStreaming'
import { buildWorkspaceRawFileUrl, extractWorkspacePathFromFileUrl } from '@/lib/workspaceLinks'
import { useDirectoryStore } from '@/stores/directory'
import { useUiStore } from '@/stores/ui'

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
  isPartExpanded: (part: TranscriptDisplayPart) => boolean
  isNodeActive?: (key: string) => boolean
  isNodeSelected?: (key: string) => boolean
  isNodeSearchMatch?: (key: string) => boolean
  optimisticUser: OptimisticUserMessage | null
  showOptimisticUser: boolean
  openMobileSidebar?: () => void | Promise<void>
  attention?: AttentionLike
}>()

const emit = defineEmits<{
  (event: 'fork', messageId: string): void
  (event: 'revert', messageId: string): void
  (event: 'copy', message: MessageLike): void
  (event: 'partToggle', part: TranscriptDisplayPart, expanded: boolean): void
  (event: 'nodeSelect', key: string): void
  (event: 'redoFromRevert'): void
  (event: 'unrevertFromRevert'): void
  (event: 'copySessionError'): void
  (event: 'clearSessionError'): void
}>()

const { t } = useI18n()
const directoryStore = useDirectoryStore()
const ui = useUiStore()
type OptimisticFile = OptimisticUserMessage['files'][number]

function optimisticWorkspacePath(file: OptimisticFile): string {
  const workspace = String(directoryStore.currentDirectory || '').trim()
  if (!workspace) return ''
  const candidate = String(file.serverPath || file.url || '').trim()
  return candidate ? extractWorkspacePathFromFileUrl(candidate, workspace) || candidate : ''
}

function optimisticFileUrl(file: OptimisticFile): string {
  const workspace = String(directoryStore.currentDirectory || '').trim()
  const path = optimisticWorkspacePath(file)
  if (workspace && path && !path.startsWith('data:') && !path.startsWith('http')) {
    return buildWorkspaceRawFileUrl(workspace, path)
  }
  return String(file.url || file.serverPath || '').trim()
}

function optimisticFileLabel(file: OptimisticFile): string {
  return String(file.filename || file.serverPath || file.url || t('chat.messageItem.fileFallback')).trim()
}

function openOptimisticFile(file: OptimisticFile) {
  const path = optimisticWorkspacePath(file)
  if (path) {
    ui.requestWorkspaceDockFile(path, 'open')
    return
  }
  const url = optimisticFileUrl(file)
  if (url) window.open(url, '_blank', 'noopener,noreferrer')
}

function optimisticIsImage(file: OptimisticFile): boolean {
  return String(file.mime || '').startsWith('image/') || /\.(png|jpe?g|gif|webp|avif)$/i.test(optimisticFileLabel(file))
}

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
          :is-part-expanded="isPartExpanded"
          :is-node-active="isNodeActive"
          :is-node-selected="isNodeSelected"
          :is-node-search-match="isNodeSearchMatch"
          :session-id="selectedSessionId"
          :attention="attention"
          @fork="$emit('fork', $event)"
          @revert="$emit('revert', $event)"
          @copy="$emit('copy', $event)"
          @part-toggle="forwardPartToggle"
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
        <div class="mt-0.5 border-l-2 border-primary/35 py-1 pl-9 text-sm leading-relaxed">
          <MarkdownRenderer v-if="optimisticUser.text" :content="optimisticUser.text" />
          <div v-if="optimisticUser.files.length" class="mt-2 flex flex-wrap gap-2">
            <button
              v-for="file in optimisticUser.files"
              :key="String(file.url || file.serverPath || file.filename)"
              type="button"
              class="inline-flex min-w-0 items-center gap-2 rounded-md border border-border/50 px-2 py-1 font-mono text-[11px] hover:bg-muted/35 hover:text-primary"
              @click="openOptimisticFile(file)"
            >
              <img
                v-if="optimisticIsImage(file) && optimisticFileUrl(file)"
                :src="optimisticFileUrl(file)"
                alt=""
                class="h-8 w-8 rounded object-cover"
              />
              <RiFileLine v-else class="h-3.5 w-3.5 shrink-0" />
              <span class="max-w-56 truncate">{{ optimisticFileLabel(file) }}</span>
            </button>
          </div>
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
