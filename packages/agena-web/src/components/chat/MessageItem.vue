<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiArrowGoBackLine, RiCheckLine, RiClipboardLine, RiGitBranchLine, RiLoader4Line } from '@remixicon/vue'

import AgenaTranscriptPart from '@/components/chat/AgenaTranscriptPart.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import type { AttentionLike, MessageLike, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import { getAssistantErrorInfo } from '@/pages/chat/assistantError'
import { foldTranscriptActivityRun } from '@/pages/chat/transcriptActivityFolding'
import { transcriptPartNavigationText } from '@/pages/chat/transcriptNavigation'
import { partStatusPresentation } from '@/pages/chat/transcriptPartPresentation'
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
  isPartExpanded: (part: TranscriptDisplayPart) => boolean
  isNodeSelected?: (key: string) => boolean
  isNodeSearchMatch?: (key: string) => boolean
  sessionId?: string | null
  attention?: AttentionLike
}>()

const emit = defineEmits<{
  (event: 'fork', messageId: string): void
  (event: 'revert', messageId: string): void
  (event: 'copy', message: MessageLike): void
  (event: 'partToggle', part: TranscriptDisplayPart, expanded: boolean): void
  (event: 'nodeSelect', key: string): void
}>()

const { t } = useI18n()
const role = computed(() => String(props.message.info.role || 'assistant'))
const messageId = computed(() => String(props.message.info.id || ''))
const messageNodeKey = computed(() => `message:${messageId.value}`)
const runStatus = computed(() =>
  partStatusPresentation(String(props.message.info.runState || props.message.info.finish || 'completed')),
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
const activityRunExpanded = ref<Record<string, boolean>>({})
const COLLAPSED_ACTIVITY_VISIBLE_COUNT = 5

type TranscriptRow =
  | { kind: 'part'; key: string; part: TranscriptDisplayPart }
  | { kind: 'summary'; key: string; hiddenCount: number; expanded: boolean }

watch(
  () => props.collapseSignal,
  () => {
    activityRunExpanded.value = {}
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
    const summaryKey = `activity-summary:${messageId.value}:${run[0]?.id || index}`
    const expanded = Boolean(activityRunExpanded.value[summaryKey])
    const folded = foldTranscriptActivityRun(run, expanded, COLLAPSED_ACTIVITY_VISIBLE_COUNT)
    if (folded.hiddenCount) {
      rows.push({ kind: 'summary', key: summaryKey, hiddenCount: folded.hiddenCount, expanded })
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

function togglePart(part: TranscriptDisplayPart) {
  emit('nodeSelect', part.key)
  emit('partToggle', part, !props.isPartExpanded(part))
}

function toggleActivitySummary(key: string) {
  activityRunExpanded.value = { ...activityRunExpanded.value, [key]: !activityRunExpanded.value[key] }
  emit('nodeSelect', key)
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
    @pointerdown="$emit('nodeSelect', messageNodeKey)"
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
        v-if="runStatus.label !== 'completed'"
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
            ? t('chat.messages.activity.moreCount', { count: row.hiddenCount })
            : partNavigationText(row.part)
        "
        tabindex="-1"
        @pointerdown="$emit('nodeSelect', row.key)"
        @focus="$emit('nodeSelect', row.key)"
      >
        <button
          v-if="row.kind === 'summary'"
          type="button"
          class="flex w-full items-center gap-2 rounded-md px-1 py-1 text-left font-mono text-[11px] text-muted-foreground hover:bg-muted/35 hover:text-foreground"
          data-transcript-toggle="true"
          :aria-expanded="row.expanded"
          @click="toggleActivitySummary(row.key)"
        >
          <span class="w-3 text-center" aria-hidden="true">{{ row.expanded ? '▾' : '▸' }}</span>
          <span>{{
            row.expanded
              ? t('chat.messages.activity.hide')
              : t('chat.messages.activity.moreCount', { count: row.hiddenCount })
          }}</span>
        </button>
        <AgenaTranscriptPart
          v-else
          :part="row.part"
          :expanded="isPartExpanded(row.part)"
          :collapse-signal="collapseSignal"
          :streaming="isStreaming && row.part.kind === 'answer'"
          :source-path="sourcePath"
          :session-id="sessionId"
          :attention="attention"
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
