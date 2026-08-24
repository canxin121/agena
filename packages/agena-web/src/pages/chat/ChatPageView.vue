<script setup lang="ts">
import { computed, isRef, ref, unref } from 'vue'
import { useWindowSize } from '@vueuse/core'
import { useI18n } from 'vue-i18n'
import {
  RiArrowDownLine,
  RiArrowDownDoubleLine,
  RiArrowUpLine,
  RiAttachmentLine,
  RiLoader4Line,
  RiMore2Line,
  RiSendPlane2Line,
  RiStackLine,
  RiStopCircleLine,
  RiBrainAi3Line,
  RiSpeedUpLine,
  RiSearchLine,
  RiCloseLine,
  RiCommandLine,
} from '@remixicon/vue'

import VerticalSplitPane from '@/components/ui/VerticalSplitPane.vue'
import MessageList from '@/components/chat/MessageList.vue'
import ChatRuntimeStatusOverlay from '@/components/chat/ChatRuntimeStatusOverlay.vue'
import ChatHeader from '@/components/chat/ChatHeader.vue'
import Composer from '@/components/chat/Composer.vue'
import RenameSessionDialog from '@/components/chat/RenameSessionDialog.vue'
import AttachProjectDialog from '@/components/chat/AttachProjectDialog.vue'
import AttachmentsPanel from '@/components/chat/AttachmentsPanel.vue'
import CommandPalette from '@/components/chat/CommandPalette.vue'
import PromptHistoryPalette from '@/components/chat/PromptHistoryPalette.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import ToolbarChipButton from '@/components/ui/ToolbarChipButton.vue'
import type { ChatPageViewContext } from './chatPageViewContext'
import { hasDisplayableAssistantError } from './assistantError'
import { resolveComposerToolbarLayout } from './composerToolbarLayout'

// This view is template-only: it takes a context bag from ChatPage.
// Keep it "dumb" so we can aggressively split ChatPage logic into composables.
const props = defineProps<{ ctx: ChatPageViewContext }>()
const ctx = props.ctx

const { t } = useI18n()

const {
  // Template refs (these are refs created in ChatPage).
  pageRef,
  scrollEl,
  contentEl,
  bottomEl,
  composerBarRef,
  composerRef,
  composerControlsRef,
  composerPickerRef,
  modelTriggerRef,
  thinkingTriggerRef,
  speedTriggerRef,
  sessionActionsMenuRef,
  transcriptSearchInputRef,

  // Stores / state.
  chat,
  ui,
  attachedFiles,
  attachmentsBusy,
  attachmentsPanelOpen,
  draft,

  // Message list.
  renderBlocks,
  pendingInitialScrollSessionId,
  loadingOlder,
  showTimestamps,
  formatTime,
  copiedMessageId,
  revertBusyMessageId,
  revertMarkerBusy,
  sessionEnded,
  retryStatus,
  currentPhase,
  awaitingAssistant,
  showAssistantPlaceholder,
  optimisticUser,
  showOptimisticUser,

  // Activity.
  activityCollapseSignal,
  transcriptPartExpanded,
  setTranscriptPartExpanded,
  loadFoldedActivity,
  transcriptVimModeLabel,
  transcriptVimCommandLabel,
  transcriptSearchOpen,
  transcriptSearchQuery,
  transcriptSearchSummary,
  selectTranscriptNode,
  isTranscriptNodeSelected,
  isTranscriptNodeSearchMatch,
  setTranscriptSearchQuery,
  handleTranscriptSearchKeydown,
  closeTranscriptSearch,

  // Scroll/nav.
  handleScroll,
  handleWheel,
  isAtBottom,
  navigableMessageIds,
  navBottomOffset,
  navIndex,
  navTotalLabel,
  navPrev,
  navNext,
  scrollToBottom,

  // Composer layout.
  composerFullscreenActive,
  composerSplitTopCollapsed,
  composerTargetHeight,
  handleComposerResize,
  resetComposerHeight,
  toggleEditorFullscreen,
  formatBytes,
  handleDrop,
  handlePaste,
  handleDraftInput,
  handleDraftKeydown,
  handlePromptHistoryKeydown,
  selectPromptHistoryEntry,
  updatePromptHistoryQuery,
  handleCommandPaletteKeydown,
  commandOpen,
  commandQuery,
  commandIndex,
  commandFocusSearch,
  commands,
  commandsLoading,
  commandIcon,
  openCommandPalette,
  selectCommand,
  setCommandQuery,
  promptHistoryOpen,
  promptHistoryQuery,
  promptHistoryEntries,
  promptHistoryIndex,
  promptHistoryAutoFocus,
  handleFileInputChange,
  removeAttachment,
  clearAttachments,
  openFilePicker,
  openProjectAttachDialog,
  toggleAttachmentsPanel,
  setAttachmentsPanelOpen,
  closeAttachmentsPanel,

  // Header actions.
  canAbort,
  retryCountdownLabel,
  retryNextLabel,
  abortRun,

  // Composer action menu.
  composerActionMenuOpen,
  composerActionMenuQuery,
  composerActionMenuGroups,
  toggleComposerActionMenu,
  closeComposerActionMenu,
  runComposerActionMenu,

  // Model and run mode picker.
  composerPickerOpen,
  composerPickerStyle,
  composerPickerTitle,
  composerPickerSearchable,
  composerPickerSearchPlaceholder,
  composerPickerQuery,
  setComposerPickerQuery,
  composerPickerHelperText,
  composerPickerEmptyText,
  composerPickerGroups,
  composerPickerLoading,
  composerPickerRefreshable,
  refreshComposerPickerOptions,
  setComposerPickerOpen,
  handleComposerPickerSelect,
  hasThinkingModesForSelection,
  hasSpeedModesForSelection,

  // Chip labels.
  modelHint,
  modelStatusLabel,
  toggleComposerPicker,
  thinkingModeHint,
  thinkingModeChipLabel,
  speedModeHint,
  speedModeChipLabel,
  composerBottomLeftStatus,
  composerBottomRightStatus,
  composerStatusExtra,

  // Usage + primary action.
  sessionUsage,
  showComposerStopAction,
  composerStopDisabled,
  composerPrimaryDisabled,
  handleComposerPrimaryAction,
  handleComposerStopAction,
  aborting,
  sending,

  // Dialogs.
  renameDialogOpen,
  renameDraft,
  renameBusy,
  saveRename,
  attachProjectDialogOpen,
  attachProjectPath,
  sessionDirectory,
  sessionTitle,
  addProjectAttachment,

  // Message actions.
  isStreamingAssistantMessage,
  handleForkFromMessage,
  handleRevertFromMessage,
  handleCopyMessage,
  handleCopySessionError,
  handleRedoFromRevertMarker,
  handleUnrevertFromRevertMarker,
} = ctx

type TooltipAnchorLike = { triggerEl?: unknown; $el?: unknown } | HTMLElement | null

const attachmentsTriggerRef = ref<TooltipAnchorLike>(null)
const actionsTriggerRef = ref<TooltipAnchorLike>(null)
const { width: viewportWidth } = useWindowSize()
const COMPOSER_DESKTOP_MENU_GAP_PX = 16
const COMPOSER_DESKTOP_MENU_VIEWPORT_MARGIN_PX = 8

const composerToolbarLayout = computed(() => resolveComposerToolbarLayout(ui.isMobilePointer, viewportWidth.value))
const splitComposerChipRows = computed(() => composerToolbarLayout.value.splitChipRows)
const modelChipTextClass = computed(() =>
  splitComposerChipRows.value
    ? 'text-[11px] font-mono font-medium truncate max-w-[88px]'
    : 'text-[11px] sm:text-xs font-mono font-medium truncate max-w-[150px] sm:max-w-[220px]',
)
const modeChipTextClass = computed(() =>
  splitComposerChipRows.value
    ? 'text-[11px] font-mono font-medium truncate max-w-[64px]'
    : 'text-[11px] sm:text-xs font-mono font-medium truncate max-w-[96px] sm:max-w-[140px]',
)

const attachmentsCount = computed(() => {
  const list = unref(attachedFiles)
  return Array.isArray(list) ? list.length : 0
})

const attachmentsCountLabel = computed(() => {
  const n = attachmentsCount.value
  if (n > 99) return '99+'
  return String(n)
})

function handleAttachProjectFromPanel() {
  closeAttachmentsPanel()
  openProjectAttachDialog()
}

const overlayReservePx = ref(0)

function handleOverlayReserve(px: number) {
  if (!Number.isFinite(px) || px <= 0) {
    overlayReservePx.value = 0
    return
  }
  overlayReservePx.value = Math.max(0, Math.floor(px))
}

// Resolve popover anchors to the trigger button element.
// This keeps desktop popups aligned with the button that opened them.
function unwrapAnchorCandidate(value: unknown): unknown {
  let current = value
  for (let i = 0; i < 4; i += 1) {
    if (!isRef(current)) return current
    current = current.value
  }
  return current
}

function resolveAnchorEl(target: TooltipAnchorLike): HTMLElement | null {
  const raw = unwrapAnchorCandidate(target)
  if (raw instanceof HTMLElement) return raw
  if (!raw || typeof raw !== 'object') return null

  const triggerEl = unwrapAnchorCandidate((raw as { triggerEl?: unknown }).triggerEl)
  if (triggerEl instanceof HTMLElement) return triggerEl

  if (triggerEl && typeof triggerEl === 'object') {
    const triggerHostEl = unwrapAnchorCandidate((triggerEl as { $el?: unknown }).$el)
    if (triggerHostEl instanceof HTMLElement) return triggerHostEl
  }

  const rootEl = unwrapAnchorCandidate((raw as { $el?: unknown }).$el)
  if (rootEl instanceof HTMLElement) return rootEl
  return null
}

const activePickerAnchor = computed(() => {
  const mode = unref(composerPickerOpen)
  if (mode === 'model') return resolveAnchorEl(unref(modelTriggerRef) as TooltipAnchorLike)
  if (mode === 'thinking') return resolveAnchorEl(unref(thinkingTriggerRef) as TooltipAnchorLike)
  if (mode === 'speed') return resolveAnchorEl(unref(speedTriggerRef) as TooltipAnchorLike)
  return null
})

const activeAttachmentsAnchor = computed(() => resolveAnchorEl(unref(attachmentsTriggerRef) as TooltipAnchorLike))
const activeActionsAnchor = computed(() => resolveAnchorEl(unref(actionsTriggerRef) as TooltipAnchorLike))

const timelineSessionError = computed(() => {
  if (!chat.selectedSessionError) return null
  const messages = Array.isArray(chat.messages) ? chat.messages : []
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (hasDisplayableAssistantError(messages[i]?.info)) {
      return null
    }
  }
  return chat.selectedSessionError
})

// `ref="..."` in templates doesn't count as usage for TS.
void pageRef
void scrollEl
void contentEl
void bottomEl
void composerBarRef
void composerRef
void composerControlsRef
void composerPickerRef
void modelTriggerRef
void thinkingTriggerRef
void speedTriggerRef
void sessionActionsMenuRef
</script>

<template>
  <section ref="pageRef" class="h-full min-h-0 flex flex-col overflow-hidden relative">
    <VerticalSplitPane
      :model-value="composerTargetHeight"
      :collapse-top="composerSplitTopCollapsed"
      @update:model-value="handleComposerResize"
      @dblclick="resetComposerHeight"
      :min-height="ui.isCompactLayout ? 170 : 190"
      :disabled="ui.isCompactLayout"
    >
      <template #top>
        <div class="relative flex h-full min-h-0 flex-col" data-vim-chat-surface="true">
          <header class="shrink-0 border-b border-border/70 bg-background/92 backdrop-blur">
            <div class="chat-message-column flex min-h-12 items-center gap-4 py-2">
              <div class="min-w-0 flex-1">
                <div class="truncate text-sm font-semibold">
                  {{ sessionTitle || (chat.selectedSessionId ? `Session ${chat.selectedSessionId}` : t('nav.chat')) }}
                </div>
                <div v-if="sessionDirectory" class="truncate font-mono text-[10px] text-muted-foreground">
                  {{ sessionDirectory }}
                </div>
              </div>
              <div class="flex shrink-0 items-center gap-2 font-mono text-[10px] text-muted-foreground">
                <span v-if="currentPhase !== 'idle'">{{ currentPhase }}</span>
                <span v-if="transcriptVimCommandLabel" class="text-foreground">{{ transcriptVimCommandLabel }}</span>
                <span
                  class="font-semibold"
                  :class="{
                    'text-primary': transcriptVimModeLabel === 'INSERT',
                    'text-amber-600 dark:text-amber-400': transcriptVimModeLabel.startsWith('VISUAL'),
                    'text-emerald-700 dark:text-emerald-300': transcriptVimModeLabel === 'NAVIGATE',
                  }"
                  >{{ transcriptVimModeLabel }}</span
                >
              </div>
            </div>

            <div
              v-if="transcriptSearchOpen"
              class="chat-message-column flex items-center gap-2 border-t border-border/50 py-1.5"
            >
              <RiSearchLine class="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span class="font-mono text-xs text-primary">{{
                transcriptVimCommandLabel.startsWith('?') ? '?' : '/'
              }}</span>
              <input
                ref="transcriptSearchInputRef"
                :value="transcriptSearchQuery"
                type="text"
                class="h-7 min-w-0 flex-1 border-0 bg-transparent px-0 font-mono text-xs outline-none"
                autocomplete="off"
                spellcheck="false"
                aria-label="Search transcript"
                @input="setTranscriptSearchQuery(($event.target as HTMLInputElement).value)"
                @keydown="handleTranscriptSearchKeydown"
              />
              <span class="font-mono text-[10px] text-muted-foreground">{{ transcriptSearchSummary }}</span>
              <button
                type="button"
                class="inline-flex h-6 w-6 items-center justify-center text-muted-foreground hover:text-foreground"
                aria-label="Close transcript search"
                @click="closeTranscriptSearch(false)"
              >
                <RiCloseLine class="h-3.5 w-3.5" />
              </button>
            </div>
          </header>

          <div
            ref="scrollEl"
            class="min-h-0 chat-scroll flex-1 overflow-y-auto"
            data-scrollbar="chat"
            @scroll="handleScroll"
            @wheel="handleWheel"
          >
            <div ref="contentEl" class="chat-message-column py-3">
              <MessageList
                :is-compact-layout="ui.isCompactLayout"
                :is-mobile-pointer="ui.isMobilePointer"
                :selected-session-id="chat.selectedSessionId"
                :messages-loading="chat.messagesLoading"
                :messages-error="chat.messagesError"
                :session-error="timelineSessionError"
                :render-blocks="renderBlocks"
                :pending-initial-scroll-session-id="pendingInitialScrollSessionId"
                :loading-older="loadingOlder"
                :activity-page-size="chat.transcriptPartPageSize"
                :show-timestamps="showTimestamps"
                :format-time="formatTime"
                :copied-message-id="copiedMessageId"
                :revert-busy-message-id="revertBusyMessageId"
                :is-streaming-assistant-message="isStreamingAssistantMessage"
                :show-assistant-placeholder="showAssistantPlaceholder"
                :revert-marker-busy="revertMarkerBusy"
                :session-ended="sessionEnded"
                :retry-status="retryStatus"
                :current-phase="currentPhase"
                :awaiting-assistant="awaitingAssistant"
                :activity-collapse-signal="activityCollapseSignal"
                :is-part-expanded="transcriptPartExpanded"
                :is-node-selected="isTranscriptNodeSelected"
                :is-node-search-match="isTranscriptNodeSearchMatch"
                :optimistic-user="optimisticUser"
                :show-optimistic-user="showOptimisticUser"
                :pending-attention="chat.selectedAttention"
                :open-mobile-sidebar="() => ui.setSessionSwitcherOpen(true)"
                @fork="handleForkFromMessage"
                @revert="handleRevertFromMessage"
                @copy="handleCopyMessage"
                @part-toggle="setTranscriptPartExpanded"
                @fold-expand="loadFoldedActivity"
                @node-select="selectTranscriptNode"
                @redo-from-revert="handleRedoFromRevertMarker"
                @unrevert-from-revert="handleUnrevertFromRevertMarker"
                @copySessionError="handleCopySessionError"
                @clearSessionError="chat.selectedSessionId ? chat.clearSessionError(chat.selectedSessionId) : undefined"
                @set-activity-page-size="ctx.setTranscriptPartPageSize"
              />

              <div v-if="overlayReservePx > 0" :style="{ height: `${overlayReservePx}px` }" aria-hidden="true" />

              <div ref="bottomEl" class="h-px w-full" aria-hidden="true" />
            </div>
          </div>

          <!-- Floating message navigation (user messages only) -->
          <div
            v-if="
              !composerFullscreenActive &&
              !(ui.isCompactLayout && ui.isSessionSwitcherOpen) &&
              (navigableMessageIds.length > 1 ||
                (!isAtBottom && chat.messages.length) ||
                (navigableMessageIds.length > 0 && !chat.selectedHistory.exhausted))
            "
            class="pointer-events-none absolute right-3 z-20 flex flex-col items-center gap-2"
            :style="{ bottom: navBottomOffset }"
          >
            <IconButton
              v-if="
                navigableMessageIds.length > 1 || (navigableMessageIds.length > 0 && !chat.selectedHistory.exhausted)
              "
              variant="outline"
              class="pointer-events-auto h-8 w-8 rounded-full bg-background/80 backdrop-blur"
              :tooltip="t('chat.page.nav.previousUserMessage')"
              :is-touch-pointer="ui.isTouchPointer"
              :aria-label="t('chat.page.nav.previousUserMessage')"
              @click="navPrev"
              :disabled="(navIndex <= 0 && chat.selectedHistory.exhausted) || loadingOlder"
            >
              <RiArrowUpLine class="h-4 w-4" />
            </IconButton>
            <IconButton
              v-if="navigableMessageIds.length > 1"
              variant="outline"
              class="pointer-events-auto h-8 w-8 rounded-full bg-background/80 backdrop-blur"
              :tooltip="t('chat.page.nav.nextUserMessage')"
              :is-touch-pointer="ui.isTouchPointer"
              :aria-label="t('chat.page.nav.nextUserMessage')"
              @click="navNext"
              :disabled="navIndex >= navigableMessageIds.length - 1"
            >
              <RiArrowDownLine class="h-4 w-4" />
            </IconButton>

            <div
              v-if="navigableMessageIds.length > 0"
              class="pointer-events-none text-[10px] text-muted-foreground/80 bg-background/80 backdrop-blur rounded-full px-2 py-0.5 border border-border/60 select-none"
            >
              {{ navIndex + 1 }} / {{ navTotalLabel }}
            </div>

            <!-- Keep this slot fixed so other controls don't move -->
            <IconButton
              variant="outline"
              class="h-8 w-8 rounded-full bg-background/80 backdrop-blur"
              :tooltip="t('chat.page.nav.bottom')"
              :is-touch-pointer="ui.isTouchPointer"
              :aria-label="t('chat.page.nav.bottom')"
              :class="!isAtBottom && chat.messages.length ? 'pointer-events-auto' : 'invisible pointer-events-none'"
              @click="scrollToBottom('smooth')"
            >
              <RiArrowDownDoubleLine class="h-4 w-4" />
            </IconButton>
          </div>

          <div
            v-if="chat.selectedSessionId && !ui.isSessionSwitcherOpen && !composerFullscreenActive"
            class="pointer-events-none absolute inset-x-0 bottom-2 z-30"
          >
            <div class="chat-column">
              <ChatRuntimeStatusOverlay
                :is-mobile-pointer="ui.isMobilePointer"
                @reserve-change="handleOverlayReserve"
              />
            </div>
          </div>
        </div>
      </template>

      <template #bottom>
        <div
          ref="composerBarRef"
          class="h-full flex flex-col min-h-0 bg-background/85 backdrop-blur ios-keyboard-safe-area"
          :data-keyboard-avoid="composerFullscreenActive ? 'resize' : 'shift'"
        >
          <div class="chat-column flex flex-col min-h-0 h-full" :class="ui.isCompactLayout ? 'py-2' : 'py-3'">
            <div class="relative flex flex-1 flex-col min-h-0">
              <ChatHeader
                :session-id="chat.selectedSessionId"
                :can-abort="canAbort"
                :retry-status="retryStatus"
                :retry-countdown-label="retryCountdownLabel"
                :retry-next-label="retryNextLabel"
                :mobile-pointer="ui.isMobilePointer"
                @abort="abortRun"
              />
              <Composer
                ref="composerRef"
                v-model:draft="draft"
                :fullscreen="composerFullscreenActive"
                class="flex-1 shrink-0 sm:shrink min-h-min"
                @toggleFullscreen="toggleEditorFullscreen"
                @drop="handleDrop"
                @paste="handlePaste"
                @draftInput="handleDraftInput"
                @draftKeydown="handleDraftKeydown"
                @filesSelected="handleFileInputChange"
              >
                <template #status>
                  <span class="flex items-center gap-1">
                    <button
                      ref="modelTriggerRef"
                      type="button"
                      data-oc-keyboard-tap="blur"
                      class="pointer-events-auto flex items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
                      :class="composerPickerOpen === 'model' ? 'bg-secondary/60 text-foreground' : ''"
                      :title="modelHint"
                      :aria-label="t('chat.composer.picker.modelTitle')"
                      @mousedown.prevent
                      @click.stop="toggleComposerPicker('model')"
                    >
                      <RiStackLine class="h-3 w-3" />
                      <span :class="modelChipTextClass">{{ modelStatusLabel }}</span>
                    </button>
                    <template v-if="hasThinkingModesForSelection">
                      <span class="text-muted-foreground/50">|</span>
                      <button
                        ref="thinkingTriggerRef"
                        type="button"
                        data-oc-keyboard-tap="blur"
                        class="pointer-events-auto flex items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
                        :class="composerPickerOpen === 'thinking' ? 'bg-secondary/60 text-foreground' : ''"
                        :title="thinkingModeHint"
                        :aria-label="t('chat.composer.picker.thinkingTitle')"
                        @mousedown.prevent
                        @click.stop="toggleComposerPicker('thinking')"
                      >
                        <RiBrainAi3Line class="h-3 w-3" />
                        <span :class="modeChipTextClass">{{ thinkingModeChipLabel }}</span>
                      </button>
                    </template>
                    <template v-if="hasSpeedModesForSelection">
                      <span class="text-muted-foreground/50">|</span>
                      <button
                        ref="speedTriggerRef"
                        type="button"
                        data-oc-keyboard-tap="blur"
                        class="pointer-events-auto flex items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
                        :class="composerPickerOpen === 'speed' ? 'bg-secondary/60 text-foreground' : ''"
                        :title="speedModeHint"
                        :aria-label="t('chat.composer.picker.speedTitle')"
                        @mousedown.prevent
                        @click.stop="toggleComposerPicker('speed')"
                      >
                        <RiSpeedUpLine class="h-3 w-3" />
                        <span :class="modeChipTextClass">{{ speedModeChipLabel || t('common.default') }}</span>
                      </button>
                    </template>
                    <template v-if="sessionUsage">
                      <span class="text-muted-foreground/50">|</span>
                      <span class="text-muted-foreground">{{
                        sessionUsage.percentUsed !== null ? `${sessionUsage.percentUsed}%` : sessionUsage.tokensLabel
                      }}</span>
                    </template>
                    <template v-if="composerStatusExtra">
                      <span class="text-muted-foreground/50">|</span>
                      <span class="text-muted-foreground">{{ composerStatusExtra }}</span>
                    </template>
                  </span>
                </template>
                <template #bottomLeft>
                  <span v-if="composerBottomLeftStatus" class="text-muted-foreground">
                    {{ composerBottomLeftStatus }}
                  </span>
                </template>
                <template #bottomRight>
                  <span v-if="composerBottomRightStatus" class="text-muted-foreground">
                    {{ composerBottomRightStatus }}
                  </span>
                </template>
                <template #overlay>
                  <PromptHistoryPalette
                    :open="promptHistoryOpen"
                    :auto-focus="promptHistoryAutoFocus"
                    :query="promptHistoryQuery"
                    :entries="promptHistoryEntries"
                    :active-index="promptHistoryIndex"
                    @update:query="updatePromptHistoryQuery"
                    @update:active-index="(value) => (promptHistoryIndex = value)"
                    @keydown="handlePromptHistoryKeydown"
                    @select="selectPromptHistoryEntry"
                  />
                  <CommandPalette
                    :open="commandOpen"
                    :auto-focus="commandFocusSearch"
                    :loading="commandsLoading"
                    :query="commandQuery"
                    :commands="commands"
                    :active-index="commandIndex"
                    :command-icon="commandIcon"
                    @update:query="setCommandQuery"
                    @update:active-index="(value) => (commandIndex = value)"
                    @keydown="handleCommandPaletteKeydown"
                    @select="selectCommand"
                  />
                </template>
                <template #controls>
                  <div ref="composerControlsRef" class="relative">
                    <OptionMenu
                      ref="composerPickerRef"
                      :open="Boolean(composerPickerOpen)"
                      :query="composerPickerQuery"
                      :groups="composerPickerGroups"
                      :title="composerPickerTitle"
                      :mobile-title="composerPickerTitle"
                      :searchable="composerPickerSearchable"
                      :search-placeholder="composerPickerSearchPlaceholder"
                      :empty-text="composerPickerEmptyText"
                      :helper-text="composerPickerHelperText"
                      :loading="composerPickerLoading"
                      :refreshable="composerPickerRefreshable"
                      :on-refresh="refreshComposerPickerOptions"
                      :is-mobile-pointer="ui.isMobilePointer"
                      :desktop-fixed="true"
                      :desktop-style="composerPickerStyle"
                      :desktop-anchor-el="activePickerAnchor"
                      :desktop-gap-px="COMPOSER_DESKTOP_MENU_GAP_PX"
                      :desktop-viewport-margin-px="COMPOSER_DESKTOP_MENU_VIEWPORT_MARGIN_PX"
                      :paginated="composerPickerOpen === 'model'"
                      :page-size="80"
                      pagination-mode="group"
                      :collapsible-groups="composerPickerOpen === 'model'"
                      desktop-placement="top-start"
                      desktop-class="w-[min(420px,calc(100%-1rem))]"
                      filter-mode="external"
                      @update:open="setComposerPickerOpen"
                      @update:query="setComposerPickerQuery"
                      @select="handleComposerPickerSelect"
                    />

                    <div
                      class="composer-controls-surface flex w-full flex-row items-center justify-between gap-2 rounded-b-xl border-t border-border/60 bg-background/60 p-2 sm:px-2.5"
                    >
                      <!-- Region 1: attachments and actions. -->
                      <div
                        class="flex-1 flex flex-nowrap items-center gap-1 sm:gap-1.5 min-w-0 overflow-x-auto oc-scrollbar-hidden [&>*]:shrink-0"
                        data-oc-keyboard-tap="blur"
                      >
                        <IconButton
                          class="text-muted-foreground hover:text-foreground hover:bg-secondary/40"
                          :class="[commandOpen ? 'bg-secondary/60 text-foreground' : '']"
                          :tooltip="t('chat.commandPalette.title')"
                          :is-touch-pointer="ui.isTouchPointer"
                          :title="t('chat.commandPalette.title')"
                          :aria-label="t('chat.commandPalette.title')"
                          @mousedown.prevent
                          @click.stop="openCommandPalette()"
                        >
                          <RiCommandLine class="h-4 w-4" />
                        </IconButton>

                        <ToolbarChipButton
                          ref="attachmentsTriggerRef"
                          :active="attachmentsPanelOpen"
                          :tooltip="
                            attachmentsCount > 0
                              ? t('chat.page.attachmentsWithCount', { count: attachmentsCount })
                              : t('chat.page.attachments')
                          "
                          :is-touch-pointer="ui.isTouchPointer"
                          :title="t('chat.page.attachments')"
                          :aria-label="t('chat.page.attachments')"
                          @mousedown.prevent
                          @click.stop="toggleAttachmentsPanel"
                        >
                          <RiAttachmentLine class="h-3.5 w-3.5 sm:h-4 sm:w-4 text-muted-foreground" />
                          <span
                            v-if="attachmentsBusy"
                            class="inline-flex items-center justify-center h-5 w-5 rounded-full border border-border/60 bg-background/60"
                            aria-hidden="true"
                          >
                            <RiLoader4Line class="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                          </span>
                          <span
                            v-else-if="attachmentsCount > 0"
                            class="inline-flex items-center justify-center h-5 min-w-5 px-1 rounded-full border border-border/60 bg-secondary/60 text-[10px] font-mono tabular-nums"
                            aria-hidden="true"
                          >
                            {{ attachmentsCountLabel }}
                          </span>
                        </ToolbarChipButton>

                        <IconButton
                          ref="actionsTriggerRef"
                          class="text-muted-foreground hover:text-foreground hover:bg-secondary/40"
                          :class="[composerActionMenuOpen ? 'bg-secondary/60 text-foreground' : '']"
                          :tooltip="t('chat.page.tools')"
                          :is-touch-pointer="ui.isTouchPointer"
                          :title="t('chat.page.tools')"
                          :aria-label="t('chat.page.tools')"
                          @mousedown.prevent
                          @click.stop="toggleComposerActionMenu($event)"
                        >
                          <RiMore2Line class="h-4 w-4" />
                        </IconButton>

                        <OptionMenu
                          ref="sessionActionsMenuRef"
                          :open="composerActionMenuOpen"
                          v-model:query="composerActionMenuQuery"
                          :groups="composerActionMenuGroups"
                          :title="t('chat.page.tools')"
                          :mobile-title="t('chat.page.tools')"
                          :searchable="true"
                          :search-placeholder="t('common.searchActions')"
                          :empty-text="t('common.noActionsFound')"
                          :is-mobile-pointer="ui.isMobilePointer"
                          :desktop-fixed="true"
                          :desktop-anchor-el="activeActionsAnchor"
                          :desktop-gap-px="COMPOSER_DESKTOP_MENU_GAP_PX"
                          :desktop-viewport-margin-px="COMPOSER_DESKTOP_MENU_VIEWPORT_MARGIN_PX"
                          desktop-placement="top-start"
                          desktop-class="w-64"
                          filter-mode="external"
                          @update:open="(v) => (!v ? closeComposerActionMenu() : undefined)"
                          @close="closeComposerActionMenu"
                          @select="runComposerActionMenu"
                        />
                      </div>

                      <!-- Region 3: Stop & Send Actions -->
                      <div class="flex-none flex items-center gap-1.5">
                        <IconButton
                          v-if="showComposerStopAction"
                          variant="outline"
                          class="h-8 w-8 text-destructive hover:text-destructive"
                          data-oc-keyboard-tap="blur"
                          :tooltip="t('chat.page.primary.stopRun')"
                          :is-touch-pointer="ui.isTouchPointer"
                          :aria-label="t('chat.page.primary.stopRun')"
                          :disabled="composerStopDisabled"
                          @click="handleComposerStopAction"
                        >
                          <RiLoader4Line v-if="aborting" class="h-4 w-4 animate-spin" />
                          <RiStopCircleLine v-else class="h-4 w-4" />
                        </IconButton>

                        <IconButton
                          variant="outline"
                          class="h-8 w-8"
                          data-oc-keyboard-tap="blur"
                          :tooltip="t('chat.page.primary.send')"
                          :is-touch-pointer="ui.isTouchPointer"
                          :aria-label="t('chat.page.primary.sendMessage')"
                          :disabled="composerPrimaryDisabled"
                          @click="handleComposerPrimaryAction"
                        >
                          <RiLoader4Line v-if="sending" class="h-4 w-4 animate-spin" />
                          <RiSendPlane2Line v-else class="h-4 w-4" />
                        </IconButton>
                      </div>
                    </div>
                  </div>
                </template>
              </Composer>
            </div>
          </div>
        </div>
      </template>
    </VerticalSplitPane>
  </section>

  <!-- Mobile tools and picker menus are handled by OptionMenu. -->

  <RenameSessionDialog
    :open="renameDialogOpen"
    v-model:draft="renameDraft"
    :busy="renameBusy"
    @update:open="(v) => (renameDialogOpen = v)"
    @save="saveRename"
  />

  <AttachProjectDialog
    :open="attachProjectDialogOpen"
    v-model:path="attachProjectPath"
    :base-path="sessionDirectory"
    :attached-count="attachedFiles.length"
    @update:open="(v) => (attachProjectDialogOpen = v)"
    @add="addProjectAttachment"
  />

  <AttachmentsPanel
    :open="attachmentsPanelOpen"
    :is-mobile-pointer="ui.isMobilePointer"
    :desktop-anchor-el="activeAttachmentsAnchor"
    desktop-placement="top-start"
    :desktop-gap-px="COMPOSER_DESKTOP_MENU_GAP_PX"
    :desktop-viewport-margin-px="COMPOSER_DESKTOP_MENU_VIEWPORT_MARGIN_PX"
    :attached-files="attachedFiles"
    :busy="attachmentsBusy"
    :format-bytes="formatBytes"
    @update:open="setAttachmentsPanelOpen"
    @remove="removeAttachment"
    @clear="clearAttachments"
    @attachLocal="openFilePicker"
    @attachProject="handleAttachProjectFromPanel"
  />
</template>

<style scoped>
.chat-scroll {
  /* Prevent browser scroll anchoring from fighting programmatic bottom pinning. */
  overflow-anchor: none;
  /* Avoid width reflow when the vertical scrollbar appears. */
  scrollbar-gutter: stable;
}

/* List motion (subtle reveal). */
.chatlist-enter-active,
.chatlist-leave-active {
  transition:
    opacity 160ms ease,
    transform 180ms ease;
}

.chatlist-enter-from,
.chatlist-leave-to {
  opacity: 0;
  transform: translateY(8px);
}

.activitylist-enter-active,
.activitylist-leave-active {
  transition:
    opacity 140ms ease,
    transform 160ms ease;
}

.activitylist-enter-from,
.activitylist-leave-to {
  opacity: 0;
  transform: translateY(6px);
}
</style>
