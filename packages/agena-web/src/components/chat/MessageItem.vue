<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue'
import { RiArrowGoBackLine, RiCheckLine, RiClipboardLine, RiGitBranchLine, RiLoader4Line } from '@remixicon/vue'

import AgenaTranscriptPart from '@/components/chat/AgenaTranscriptPart.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import ToolbarChipButton from '@/components/ui/ToolbarChipButton.vue'
import PartLoadingIndicator from '@/components/chat/PartLoadingIndicator.vue'
import type { MessageLike, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import type { MessageFold } from '@/types/chat'
import { getAssistantErrorInfo } from '@/pages/chat/assistantError'
import {
  foldTranscriptReply,
  preserveActivityVisibility,
  transcriptActivityRunKey,
  type ActivityVisibility,
} from '@/pages/chat/transcriptActivityFolding'
import { transcriptPartNavigationText } from '@/pages/chat/transcriptNavigation'
import { partHasPendingInteraction, partPendingInteractionRequestIds } from '@/pages/chat/transcriptProjection'
import { focusTranscriptPartControl } from '@/pages/chat/transcriptControls'
import { partStatusPresentation } from '@/pages/chat/transcriptPartPresentation'
import { DEFAULT_TRANSCRIPT_PART_PAGE_SIZE, normalizeTranscriptPartPageSize } from '@/pages/chat/transcriptPartPaging'
import { useI18n } from 'vue-i18n'
import { transcriptFoldKey } from '@/stores/chat/transcriptFolds'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useNearViewport } from '@/composables/useNearViewport'

const props = defineProps<{
  message: MessageLike
  displayParts: TranscriptDisplayPart[]
  showTimestamps: boolean
  formatTime: (ms?: number) => string
  copiedMessageId: string
  revertBusyMessageId: string
  isStreaming: boolean
  collapseSignal: number
  activityPageSize: number
  activityVisibility?: ActivityVisibility
  foldLoadingByKey?: Record<string, boolean>
  foldErrorByKey?: Record<string, string>
  isCompactTouch: boolean
  isPartExpanded: (part: TranscriptDisplayPart) => boolean
  isNodeSelected?: (key: string) => boolean
  isNodeSearchMatch?: (key: string) => boolean
  sessionId?: string | null
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
const pane = useWorkspacePaneContext()
const articleRef = ref<HTMLElement | null>(null)
const nearViewport = useNearViewport(articleRef)
const role = computed(() => String(props.message.info.role || 'assistant'))
const messageId = computed(() => String(props.message.info.id || ''))
const messageNodeKey = computed(() => `message:${messageId.value}`)
const runStatus = computed(() =>
  partStatusPresentation(String(props.message.info.runState || props.message.info.finish || '')),
)
const sourcePath = computed(() => {
  for (const part of props.message.parts || []) {
    const candidate = String(part.serverPath || '').trim()
    if (candidate) return candidate
  }
  return ''
})
const hasErrorPart = computed(() => props.displayParts.some((part) => part.kind === 'error'))
const assistantError = computed(() =>
  getAssistantErrorInfo({ role: props.message.info.role, error: props.message.info.error }),
)
const fallbackError = computed(() => {
  if (hasErrorPart.value || !assistantError.value || assistantError.value.interrupted) return ''
  return assistantError.value.message || ''
})
const localVisibility = reactive<ActivityVisibility>({ ids: [] })
const visibility = computed(() => props.activityVisibility ?? localVisibility)
const activityPageSize = computed(() => normalizeTranscriptPartPageSize(props.activityPageSize))
const summaryKey = computed(() => transcriptActivityRunKey(messageId.value, [], 0))
const requestedFoldKey = ref('')

// Explicitly revealed rows remain visible when a live reply appends content.
watch(
  () => props.displayParts.filter((part) => part.kind !== 'lifecycle').map((part) => part.id),
  (next) => {
    const state = visibility.value
    if (state.count !== undefined) state.count = preserveActivityVisibility(next, state.ids, state.count)
    state.ids = next
  },
  { flush: 'sync', immediate: true },
)
watch(
  () => props.collapseSignal,
  () => {
    if (!visibility.value.keepOpen) visibility.value.count = undefined
  },
)

function keepVisibleParts() {
  const current = visibility.value.count ?? DEFAULT_TRANSCRIPT_PART_PAGE_SIZE
  const visible = foldTranscriptReply(props.displayParts, [], current, (part) =>
    partHasPendingInteraction(part.source),
  ).visibleParts.filter((part) => part.kind !== 'lifecycle').length
  visibility.value.count = Math.max(current, visible)
  visibility.value.keepOpen = true
}

// Vim toggles go through the page's expansion map rather than togglePart.
// Preserve the same visible suffix for either input path.
watch(
  () => props.displayParts.map((part) => [part.id, props.isPartExpanded(part)] as const),
  (next, previous) => {
    const before = new Map(previous)
    if (next.some(([id, expanded]) => expanded && before.get(id) === false)) keepVisibleParts()
  },
  { flush: 'sync' },
)

const canCollapseParts = computed(
  () => (visibility.value.count ?? DEFAULT_TRANSCRIPT_PART_PAGE_SIZE) > DEFAULT_TRANSCRIPT_PART_PAGE_SIZE,
)

watch(
  () => props.displayParts.map((part) => ({ part, ids: partPendingInteractionRequestIds(part.source) })),
  (parts) => {
    const seen = new Set(visibility.value.pendingRequestIds || [])
    visibility.value.pendingRequestIds = parts.flatMap(({ ids }) => ids)
    for (const { part, ids } of parts) {
      if (ids.some((id) => !seen.has(id)) && !props.isPartExpanded(part)) emit('partToggle', part, true)
    }
  },
  { immediate: true, flush: 'sync' },
)

type TranscriptRow =
  | { kind: 'part'; key: string; part: TranscriptDisplayPart }
  | { kind: 'summary'; key: string; hiddenCount: number; fold: MessageFold | null }

const transcriptRows = computed<TranscriptRow[]>(() => {
  if (role.value !== 'assistant') {
    return props.displayParts.map((part) => ({ kind: 'part' as const, key: part.key, part }))
  }
  const rows: TranscriptRow[] = []
  const key = summaryKey.value
  // The remote prefix belongs to the reply, even when its first visible part
  // is filtered out by the reasoning preference. Keep its control independent
  // of individual presentation rows.
  const visibleCount = visibility.value.count ?? DEFAULT_TRANSCRIPT_PART_PAGE_SIZE
  const folded = foldTranscriptReply(props.displayParts, props.message.folds || [], visibleCount, (part) =>
    partHasPendingInteraction(part.source),
  )
  if (folded.hiddenCount || canCollapseParts.value || foldLoading(folded.fold))
    rows.push({ kind: 'summary', key, hiddenCount: folded.hiddenCount, fold: folded.fold })
  for (const part of folded.visibleParts) rows.push({ kind: 'part', key: part.key, part })
  return rows
})

function foldLoading(fold: MessageFold | null): boolean {
  const key = fold ? transcriptFoldKey(props.sessionId || '', fold) : requestedFoldKey.value
  return Boolean(key && props.foldLoadingByKey?.[key])
}

function foldError(fold: MessageFold | null): string {
  const key = fold ? transcriptFoldKey(props.sessionId || '', fold) : requestedFoldKey.value
  return (key && props.foldErrorByKey?.[key]) || ''
}

function selected(key: string): boolean {
  return props.isNodeSelected?.(key) === true
}

function searchMatch(key: string): boolean {
  return props.isNodeSearchMatch?.(key) === true
}

function selectMessageNode(event: PointerEvent) {
  const target = event.target
  // Part rows own their selection. Letting this article-level handler run
  // after a part handler rewrites the active key to `message:<id>`, whose
  // first text entry may be the synthetic folded-activity summary. That
  // causes the custom transcript cursor to jump to "expand more" before the
  // browser's release position restores the real character.
  if (target instanceof Element && target.closest('[data-transcript-node="part"]')) return
  emit('nodeSelect', messageNodeKey.value)
}

function togglePart(part: TranscriptDisplayPart) {
  keepVisibleParts()
  emit('revealParts')
  emit('nodeSelect', part.key)
  emit('partToggle', part, !props.isPartExpanded(part))
}

function revealActivitySummary(
  row: Extract<TranscriptRow, { kind: 'summary' }>,
  all = false,
  requestRemote = true,
  pageSize?: number,
) {
  if (foldLoading(row.fold)) return
  restoreSummaryControlFocusAfterUpdate()
  emit('revealParts')
  visibility.value.keepOpen = true
  const current = visibility.value.count ?? DEFAULT_TRANSCRIPT_PART_PAGE_SIZE
  const step = Math.max(1, Math.min(pageSize ?? activityPageSize.value, row.hiddenCount))
  const next = all ? Number.MAX_SAFE_INTEGER : current + step
  visibility.value.count = next
  const cachedCount = props.displayParts.filter((part) => part.kind !== 'lifecycle').length
  if (row.fold && requestRemote && (all || next > cachedCount)) {
    requestedFoldKey.value = transcriptFoldKey(props.sessionId || '', row.fold)
    emit('foldExpand', row.fold, all)
  }
  emit('nodeSelect', row.key)
}

function revealSummary(row: Extract<TranscriptRow, { kind: 'summary' }>, all = false) {
  revealActivitySummary(row, all)
}

function handleSummaryPageSizeInput(event: Event) {
  const input = event.target
  if (input instanceof HTMLInputElement) {
    emit('setActivityPageSize', normalizeTranscriptPartPageSize(input.value))
  }
}

// The inline page-size input must page the fold with the value the user sees in
// the same interaction; waiting for the parent prop round-trip would page with
// the previous size.
function inlinePageSize(scope: Element | null): number | null {
  const input = scope instanceof HTMLInputElement ? scope : scope?.querySelector('input[data-part-page-size="true"]')
  return input instanceof HTMLInputElement ? normalizeTranscriptPartPageSize(input.value) : null
}

function revealSummaryFromChip(event: Event, row: Extract<TranscriptRow, { kind: 'summary' }>) {
  const size = inlinePageSize(
    event.currentTarget instanceof Element ? event.currentTarget.closest('[data-part-expand-next]') : null,
  )
  if (size !== null) emit('setActivityPageSize', size)
  revealActivitySummary(row, false, true, size ?? activityPageSize.value)
}

function revealSummaryFromPageSizeInput(event: Event, row: Extract<TranscriptRow, { kind: 'summary' }>) {
  const size = inlinePageSize(event.target instanceof Element ? event.target : null)
  if (size === null) {
    revealSummary(row)
    return
  }
  emit('setActivityPageSize', size)
  revealActivitySummary(row, false, true, size)
}

function partNavigationText(part: TranscriptDisplayPart): string {
  return transcriptPartNavigationText(part, props.isPartExpanded(part))
}

function collapseParts() {
  restoreSummaryControlFocusAfterUpdate()
  emit('revealParts')
  visibility.value.count = undefined
  visibility.value.keepOpen = false
  emit('nodeSelect', summaryKey.value)
}

function restoreSummaryControlFocusAfterUpdate() {
  if (!articleRef.value) return
  const focused = document.activeElement
  if (
    !(focused instanceof HTMLElement) ||
    !articleRef.value?.contains(focused) ||
    !focused.closest('[data-part-controls="true"]')
  )
    return
  nextTick(() => {
    if (focused.isConnected && !focused.matches(':disabled')) return
    const row = articleRef.value?.querySelector<HTMLElement>('[data-part-controls="true"]')
    if (row) focusTranscriptPartControl(row, 1)
  })
}

function selectPartRow(row: TranscriptRow) {
  // Interacting with an already open default-expanded part is deliberate
  // viewing too; new siblings must not fold it away while it is being read.
  if (row.kind === 'part' && props.isPartExpanded(row.part)) {
    keepVisibleParts()
    emit('revealParts')
  }
  emit('nodeSelect', row.key)
}

function handlePartControlsKeydown(event: KeyboardEvent) {
  if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.isComposing) return
  const root = event.currentTarget
  if (!(root instanceof HTMLElement)) return
  if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
    if (
      focusTranscriptPartControl(
        root,
        event.key === 'ArrowRight' ? 1 : -1,
        event.target instanceof Element ? event.target : null,
      )
    ) {
      event.preventDefault()
      event.stopPropagation()
    }
  } else if (event.key === 'Escape') {
    root.closest<HTMLElement>('[data-transcript-node="part"]')?.focus({ preventScroll: true })
    event.preventDefault()
    event.stopPropagation()
  }
}
</script>

<template>
  <article
    ref="articleRef"
    :id="`${pane?.windowId.value || 'page'}-msg-${messageId}`"
    class="group/message relative min-w-0 scroll-mt-16 rounded-lg px-1 py-2"
    style="content-visibility: auto; contain-intrinsic-size: auto 120px"
    :class="[
      selected(messageNodeKey) ? 'bg-primary/10' : '',
      searchMatch(messageNodeKey) ? 'ring-1 ring-inset ring-amber-400/55' : '',
    ]"
    data-transcript-node="message"
    :data-transcript-key="messageNodeKey"
    :data-message-id="messageId"
    :data-chat-message-anchor="role === 'user' ? 'true' : undefined"
    :data-role="role"
    :data-near-viewport="nearViewport ? 'true' : 'false'"
    tabindex="-1"
    @pointerdown="selectMessageNode"
    @focus="$emit('nodeSelect', messageNodeKey)"
  >
    <header class="flex min-h-6 items-center gap-2 px-1 text-[11px] text-muted-foreground">
      <span
        class="font-semibold"
        :class="{
          'text-primary': role === 'user',
          'text-emerald-700 dark:text-emerald-300': role === 'assistant' && !fallbackError,
          'text-amber-700 dark:text-amber-300': role === 'system' || role === 'runtime',
          'text-rose-700 dark:text-rose-300': Boolean(fallbackError),
        }"
        >{{ role }}</span
      >
      <span
        v-if="runStatus.label && runStatus.label !== 'completed'"
        class="font-mono"
        :class="{
          'text-primary': runStatus.tone === 'pending',
          'text-amber-600 dark:text-amber-400': runStatus.tone === 'warning',
          'text-rose-600 dark:text-rose-400': runStatus.tone === 'danger',
        }"
        >{{ runStatus.icon }}</span
      >
      <span v-if="showTimestamps">{{ formatTime(message.info.time?.created) }}</span>
      <span v-if="message.info.providerID || message.info.modelID" class="min-w-0 truncate font-mono text-[10px]">
        {{ [message.info.providerID, message.info.adapterID, message.info.modelID].filter(Boolean).join('/') }}
      </span>
      <span v-if="assistantError?.interrupted" class="text-muted-foreground">{{
        t('chat.messageItem.interrupted')
      }}</span>

      <span class="flex-1" />

      <div
        v-if="role === 'user' || role === 'assistant'"
        class="flex items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover/message:opacity-100"
        data-transcript-chrome="true"
      >
        <ConfirmPopover
          v-if="role === 'user'"
          :title="t('chat.messageItem.fork.confirmTitle')"
          :description="t('chat.messageItem.fork.confirmDescription')"
          :confirm-text="t('chat.messageItem.fork.confirmAction')"
          :cancel-text="t('common.cancel')"
          :anchor-to-cursor="false"
          @confirm="$emit('fork', messageId)"
        >
          <IconButton
            variant="ghost"
            class="h-6 w-6"
            :tooltip="t('chat.messageItem.fork.actionTitle')"
            :aria-label="t('chat.messageItem.fork.actionTitle')"
          >
            <RiGitBranchLine class="h-3.5 w-3.5" />
          </IconButton>
        </ConfirmPopover>

        <ConfirmPopover
          v-if="role === 'user'"
          :title="t('chat.messageItem.revert.confirmTitle')"
          :description="t('chat.messageItem.revert.confirmDescription')"
          :confirm-text="t('chat.messageItem.revert.confirmAction')"
          :cancel-text="t('common.cancel')"
          variant="destructive"
          :anchor-to-cursor="false"
          @confirm="$emit('revert', messageId)"
        >
          <IconButton
            variant="ghost"
            class="h-6 w-6"
            :tooltip="t('chat.messageItem.revert.actionTitle')"
            :aria-label="t('chat.messageItem.revert.actionTitle')"
            :disabled="revertBusyMessageId === messageId"
          >
            <RiLoader4Line v-if="revertBusyMessageId === messageId" class="h-3.5 w-3.5 animate-spin" />
            <RiArrowGoBackLine v-else class="h-3.5 w-3.5" />
          </IconButton>
        </ConfirmPopover>

        <IconButton
          variant="ghost"
          class="h-6 w-6"
          :tooltip="t('chat.messageItem.copy.actionTitle')"
          :aria-label="t('chat.messageItem.copy.actionTitle')"
          @click="$emit('copy', message)"
        >
          <RiCheckLine v-if="copiedMessageId === messageId" class="h-3.5 w-3.5 text-emerald-500" />
          <RiClipboardLine v-else class="h-3.5 w-3.5" />
        </IconButton>
      </div>
    </header>

    <div
      class="mt-0.5 min-w-0"
      :class="role === 'user' ? 'rounded-r-md border-l-2 border-primary/35 pl-2' : ''"
      data-transcript-copy-root="true"
    >
      <div
        v-for="row in transcriptRows"
        :key="row.key"
        class="min-w-0 scroll-mt-20 rounded-md px-1"
        :class="[
          selected(row.key) ? 'bg-primary/10' : '',
          searchMatch(row.key) ? 'ring-1 ring-inset ring-amber-400/55' : '',
        ]"
        data-transcript-node="part"
        :data-transcript-key="row.key"
        :data-message-id="messageId"
        :data-part-id="row.kind === 'part' ? row.part.id : undefined"
        :data-part-kind="row.kind === 'part' ? row.part.kind : 'activity_summary'"
        :data-toggleable="row.kind === 'summary' || row.part.toggleable ? 'true' : 'false'"
        :data-copy-text="
          row.kind === 'summary'
            ? row.hiddenCount > 0
              ? t('chat.messages.activity.moreCount', { count: row.hiddenCount })
              : t('chat.messages.controls.collapseParts')
            : partNavigationText(row.part)
        "
        tabindex="-1"
        @pointerdown="selectPartRow(row)"
        @focus="$emit('nodeSelect', row.key)"
      >
        <div
          v-if="row.kind === 'summary'"
          class="flex min-w-0 flex-wrap items-center gap-1 py-1"
          data-part-controls="true"
          :aria-busy="foldLoading(row.fold)"
          @keydown="handlePartControlsKeydown"
        >
          <span v-if="row.hiddenCount > 0" class="inline-flex min-w-0 items-center" data-part-expand-next="true">
            <span
              class="inline-flex h-7 min-w-0 items-center gap-1 rounded-md px-1.5 text-[11px] text-muted-foreground sm:h-8 sm:px-2"
            >
              <button
                type="button"
                class="h-full min-w-0 rounded px-1 transition-colors outline-none hover:bg-secondary/40 hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring/50"
                :aria-label="t('chat.messages.controls.expandNextCount', { count: activityPageSize })"
                :title="t('chat.messages.controls.expandNextCount', { count: activityPageSize })"
                data-transcript-toggle="true"
                data-part-control="true"
                :disabled="foldLoading(row.fold)"
                @click.stop="revealSummaryFromChip($event, row)"
              >
                {{ t('chat.messages.controls.expandNextLead') }}
              </button>
              <input
                :value="activityPageSize"
                type="number"
                min="1"
                max="50"
                step="1"
                class="h-5 w-9 rounded border border-border/60 bg-background/70 px-0 text-center font-mono text-[10px] outline-none focus:border-primary/60 sm:h-6"
                :aria-label="t('chat.messages.controls.pageSizeInputLabel')"
                :title="t('chat.messages.controls.pageSizeInputLabel')"
                data-part-page-size="true"
                data-part-control="true"
                @click.stop
                @change="handleSummaryPageSizeInput"
                @keydown.enter.prevent="revealSummaryFromPageSizeInput($event, row)"
              />
              <span class="min-w-0 truncate">{{ t('chat.messages.controls.expandNextTail') }}</span>
            </span>
          </span>
          <ToolbarChipButton
            v-if="row.hiddenCount > 0"
            :tooltip="t('chat.messages.controls.collectAll')"
            :title="t('chat.messages.controls.collectAll')"
            :is-compact-touch="isCompactTouch"
            :disabled="row.hiddenCount <= 0 || foldLoading(row.fold)"
            data-part-collect-all="true"
            data-part-control="true"
            @click.stop="revealSummary(row, true)"
          >
            {{ t('chat.messages.controls.collectAll') }}
          </ToolbarChipButton>
          <ToolbarChipButton
            v-if="canCollapseParts"
            :tooltip="t('chat.messages.controls.collapsePartsHint')"
            :is-compact-touch="isCompactTouch"
            data-part-collapse="true"
            data-part-control="true"
            :data-transcript-toggle="row.hiddenCount === 0 ? 'true' : undefined"
            @click.stop="collapseParts"
          >
            {{ t('chat.messages.controls.collapseParts') }}
          </ToolbarChipButton>
          <PartLoadingIndicator v-if="foldLoading(row.fold)" class="px-1" />
          <span v-if="row.hiddenCount > 0" class="min-w-0 px-1 font-mono text-[11px] text-muted-foreground">
            {{ t('chat.messages.activity.moreCount', { count: row.hiddenCount }) }}
          </span>
          <span v-if="foldError(row.fold)" role="alert" class="basis-full text-xs text-destructive">
            {{ foldError(row.fold) }}
          </span>
        </div>
        <AgenaTranscriptPart
          v-else
          :part="row.part"
          :expanded="isPartExpanded(row.part)"
          :collapse-signal="collapseSignal"
          :streaming="isStreaming && row.part.kind === 'answer'"
          :source-path="sourcePath"
          :session-id="sessionId"
          @toggle="togglePart(row.part)"
          @select="$emit('nodeSelect', row.part.key)"
        />
      </div>

      <div
        v-if="fallbackError"
        class="ml-7 rounded-r-md border-l border-rose-400/60 py-1 pl-3 text-sm text-rose-700 dark:text-rose-300"
      >
        {{ fallbackError }}
      </div>
    </div>
  </article>
</template>
