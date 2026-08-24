<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  RiArrowGoBackLine,
  RiCheckLine,
  RiClipboardLine,
  RiGitBranchLine,
  RiLoader4Line,
  RiMore2Line,
} from '@remixicon/vue'

import AgenaTranscriptPart from '@/components/chat/AgenaTranscriptPart.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import ToolbarChipButton from '@/components/ui/ToolbarChipButton.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import type { MessageLike, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import type { MessageFold } from '@/types/chat'
import { getAssistantErrorInfo } from '@/pages/chat/assistantError'
import { foldTranscriptActivityRun } from '@/pages/chat/transcriptActivityFolding'
import { transcriptPartNavigationText } from '@/pages/chat/transcriptNavigation'
import { partStatusPresentation } from '@/pages/chat/transcriptPartPresentation'
import { partHasPendingInteraction } from '@/pages/chat/transcriptProjection'
import { normalizeTranscriptPartPageSize, TRANSCRIPT_PART_PAGE_SIZE_OPTIONS } from '@/pages/chat/transcriptPartPaging'
import { useI18n } from 'vue-i18n'

const props = defineProps<{
  message: MessageLike
  displayParts: TranscriptDisplayPart[]
  showTimestamps: boolean
  formatTime: (ms?: number) => string
  copiedMessageId: string
  revertBusyMessageId: string
  isStreaming: boolean
  collapseSignal: number
  expandAllSignal: number
  activityPageSize: number
  isMobilePointer: boolean
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
  (event: 'expandAll'): void
  (event: 'collapseAll'): void
  (event: 'setActivityPageSize', size: number): void
}>()

const { t } = useI18n()
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
const activityRunVisibleCount = ref<Record<string, number>>({})
const allActivityRunsExpanded = ref(props.expandAllSignal > props.collapseSignal)
const activityPageSize = computed(() => normalizeTranscriptPartPageSize(props.activityPageSize))
const summaryMenuOpenKey = ref('')
const summaryMenuAnchor = ref<HTMLElement | null>(null)

type TranscriptRow =
  | { kind: 'part'; key: string; part: TranscriptDisplayPart }
  | { kind: 'summary'; key: string; hiddenCount: number; expanded: boolean }

watch(
  () => props.collapseSignal,
  () => {
    activityRunVisibleCount.value = {}
    allActivityRunsExpanded.value = false
  },
)

watch(
  () => props.expandAllSignal,
  (signal) => {
    if (signal > 0) allActivityRunsExpanded.value = true
  },
)

const transcriptRows = computed<TranscriptRow[]>(() => {
  if (role.value === 'user') {
    return props.displayParts.map((part) => ({ kind: 'part' as const, key: part.key, part }))
  }
  const rows: TranscriptRow[] = []
  let index = 0
  while (index < props.displayParts.length) {
    const current = props.displayParts[index]
    if (!current) break
    if (current.kind === 'text') {
      rows.push({ kind: 'part', key: current.key, part: current })
      index += 1
      continue
    }
    const run: TranscriptDisplayPart[] = []
    while (index < props.displayParts.length && props.displayParts[index]?.kind !== 'text') {
      const part = props.displayParts[index]
      if (part) run.push(part)
      index += 1
    }
    const remoteFold = props.message.folds?.find((fold) =>
      run.some((part) => String(part.id) === String(fold.anchorPartId)),
    )
    const summaryKey = `activity-summary:${messageId.value}:${remoteFold?.anchorPartId || run[0]?.id || index}`
    const visibleCount = allActivityRunsExpanded.value
      ? Number.MAX_SAFE_INTEGER
      : (activityRunVisibleCount.value[summaryKey] ?? activityPageSize.value)
    // An unanswered permission/question is an active control surface, not
    // passive activity. Keep its operation visible even when the surrounding
    // activity run is collapsed; otherwise the user can receive attention but
    // has no keyboard-reachable control in the transcript.
    const preservesPendingInteraction = run.some(
      (part) => part.kind === 'operation' && partHasPendingInteraction(part.source),
    )
    const folded = preservesPendingInteraction
      ? { hiddenCount: 0, visibleParts: run }
      : foldTranscriptActivityRun(run, visibleCount)
    const hiddenCount = (remoteFold?.hiddenCount || 0) + folded.hiddenCount
    if (hiddenCount) {
      rows.push({ kind: 'summary', key: summaryKey, hiddenCount, expanded: false })
    }
    for (const part of folded.visibleParts) {
      rows.push({ kind: 'part', key: part.key, part })
    }
  }
  return rows
})

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
  emit('nodeSelect', part.key)
  emit('partToggle', part, !props.isPartExpanded(part))
}

function revealActivitySummary(key: string, hiddenCount: number, all = false, requestRemote = true) {
  const current = activityRunVisibleCount.value[key] ?? activityPageSize.value
  const next = all ? Number.MAX_SAFE_INTEGER : current + Math.max(1, Math.min(activityPageSize.value, hiddenCount))
  activityRunVisibleCount.value = { ...activityRunVisibleCount.value, [key]: next }
  const anchorPartId = key.slice(key.lastIndexOf(':') + 1)
  const fold = props.message.folds?.find((candidate) => String(candidate.anchorPartId) === anchorPartId)
  if (fold && requestRemote) emit('foldExpand', fold, all)
  emit('nodeSelect', key)
}

function revealSummary(row: Extract<TranscriptRow, { kind: 'summary' }>, all = false) {
  revealActivitySummary(row.key, row.hiddenCount, all)
}

function handleSummaryPageSizeInput(event: Event) {
  const input = event.target
  if (input instanceof HTMLInputElement) {
    emit('setActivityPageSize', normalizeTranscriptPartPageSize(input.value))
  }
}

function toggleSummaryMenu(key: string, event: MouseEvent) {
  if (event.currentTarget instanceof HTMLElement) summaryMenuAnchor.value = event.currentTarget
  summaryMenuOpenKey.value = summaryMenuOpenKey.value === key ? '' : key
}

function closeSummaryMenu() {
  summaryMenuOpenKey.value = ''
}

function summaryMenuGroups(row: Extract<TranscriptRow, { kind: 'summary' }>): OptionMenuGroup[] {
  return [
    {
      id: 'part-actions',
      title: String(t('chat.messages.controls.actionsTitle')),
      items: [
        {
          id: 'expand-next',
          label: String(t('chat.messages.controls.expandNextCount', { count: activityPageSize.value })),
          disabled: row.hiddenCount <= 0,
        },
        {
          id: 'collect-all',
          label: String(t('chat.messages.controls.collectAll')),
          disabled: row.hiddenCount <= 0,
        },
        {
          id: 'expand-all',
          label: String(t('chat.messages.activity.expandAll')),
        },
        {
          id: 'collapse-all',
          label: String(t('chat.messages.activity.collapseAll')),
        },
      ],
    },
    {
      id: 'part-page-size',
      title: String(t('chat.messages.controls.pageSizeTitle')),
      items: TRANSCRIPT_PART_PAGE_SIZE_OPTIONS.map<OptionMenuItem>((size) => ({
        id: `page-size:${size}`,
        label: String(t('chat.messages.controls.pageSizeOption', { count: size })),
        checked: size === activityPageSize.value,
      })),
    },
  ]
}

function runSummaryMenu(row: Extract<TranscriptRow, { kind: 'summary' }>, item: OptionMenuItem) {
  const id = String(item.id || '')
  if (id === 'expand-next') revealSummary(row)
  else if (id === 'collect-all') revealSummary(row, true)
  else if (id === 'expand-all') emit('expandAll')
  else if (id === 'collapse-all') emit('collapseAll')
  else if (id.startsWith('page-size:')) {
    emit('setActivityPageSize', normalizeTranscriptPartPageSize(id.slice('page-size:'.length)))
  }
  closeSummaryMenu()
}

function partNavigationText(part: TranscriptDisplayPart): string {
  return transcriptPartNavigationText(part, props.isPartExpanded(part))
}
</script>

<template>
  <article
    :id="`msg-${messageId}`"
    class="group/message relative min-w-0 scroll-mt-16 rounded-lg px-1 py-2"
    :class="[
      selected(messageNodeKey) ? 'bg-primary/10' : '',
      searchMatch(messageNodeKey) ? 'ring-1 ring-inset ring-amber-400/55' : '',
    ]"
    data-transcript-node="message"
    :data-transcript-key="messageNodeKey"
    :data-message-id="messageId"
    :data-chat-message-anchor="role === 'user' ? 'true' : undefined"
    :data-role="role"
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
        :data-transcript-chrome="row.kind === 'summary' ? 'true' : undefined"
        :data-toggleable="row.kind === 'summary' || row.part.toggleable ? 'true' : 'false'"
        :data-copy-text="
          row.kind === 'summary'
            ? t('chat.messages.activity.moreCount', { count: row.hiddenCount })
            : partNavigationText(row.part)
        "
        tabindex="-1"
        @pointerdown="$emit('nodeSelect', row.key)"
        @focus="$emit('nodeSelect', row.key)"
      >
        <div v-if="row.kind === 'summary'" class="flex min-w-0 flex-wrap items-center gap-1" data-part-controls="true">
          <button
            type="button"
            class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1 py-1 text-left font-mono text-[11px] text-muted-foreground hover:bg-muted/35 hover:text-foreground"
            data-transcript-toggle="true"
            :aria-expanded="false"
            @click="revealActivitySummary(row.key, row.hiddenCount)"
          >
            <span class="w-3 text-center" aria-hidden="true">▸</span>
            <span>{{ t('chat.messages.activity.expandMoreCount', { count: row.hiddenCount }) }}</span>
          </button>
          <div class="flex shrink-0 flex-wrap items-center gap-1" data-transcript-chrome="true">
            <ToolbarChipButton
              :tooltip="t('chat.messages.controls.expandNextCount', { count: activityPageSize })"
              :title="t('chat.messages.controls.expandNextCount', { count: activityPageSize })"
              :is-mobile-pointer="isMobilePointer"
              :disabled="row.hiddenCount <= 0"
              data-part-expand-next="true"
              @click.stop="revealSummary(row)"
            >
              {{ t('chat.messages.controls.expandNextCount', { count: activityPageSize }) }}
            </ToolbarChipButton>
            <ToolbarChipButton
              :tooltip="t('chat.messages.controls.collectAll')"
              :title="t('chat.messages.controls.collectAll')"
              :is-mobile-pointer="isMobilePointer"
              :disabled="row.hiddenCount <= 0"
              data-part-collect-all="true"
              @click.stop="revealSummary(row, true)"
            >
              {{ t('chat.messages.controls.collectAll') }}
            </ToolbarChipButton>
            <input
              :value="activityPageSize"
              type="number"
              min="1"
              max="50"
              step="1"
              class="h-6 w-11 rounded border border-border/60 bg-background/70 px-1 text-center font-mono text-[10px] outline-none focus:border-primary/60"
              :aria-label="t('chat.messages.controls.pageSizeInputLabel')"
              :title="t('chat.messages.controls.pageSizeInputLabel')"
              data-part-page-size="true"
              @change="handleSummaryPageSizeInput"
              @keydown.enter="handleSummaryPageSizeInput"
            />
            <IconButton
              variant="ghost"
              size="sm"
              :tooltip="t('chat.messages.controls.menuTitle')"
              :is-mobile-pointer="isMobilePointer"
              :aria-label="t('chat.messages.controls.menuTitle')"
              :class="summaryMenuOpenKey === row.key ? 'bg-secondary/60 text-foreground' : 'text-muted-foreground'"
              data-part-menu="true"
              @click.stop="toggleSummaryMenu(row.key, $event)"
            >
              <RiMore2Line class="h-4 w-4" />
            </IconButton>
            <OptionMenu
              v-if="summaryMenuOpenKey === row.key"
              :open="true"
              :groups="summaryMenuGroups(row)"
              :title="t('chat.messages.controls.menuTitle')"
              :mobile-title="t('chat.messages.controls.menuTitle')"
              :searchable="false"
              :is-mobile-pointer="isMobilePointer"
              :desktop-fixed="true"
              :desktop-anchor-el="summaryMenuAnchor"
              desktop-placement="bottom-end"
              desktop-class="w-72"
              @update:open="summaryMenuOpenKey = $event ? row.key : ''"
              @close="closeSummaryMenu"
              @select="runSummaryMenu(row, $event)"
            />
          </div>
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
