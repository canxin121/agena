<script setup lang="ts">
// This dialog is a plan state inspector. It may refresh the persisted
// autorun setting, but it never owns a pending permission or user-input
// request. Plan approval decisions (including approve-with-autorun choices)
// are operation-owned user_input records rendered by AgenaInteractionPart.
import { computed } from 'vue'
import { RiLoader4Line, RiRefreshLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import Dialog from '@/components/ui/Dialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import { buildPlanToolInvocationRequest, type PlanTool, type PlanToolInput } from '@/pages/chat/planViewerRequest'
import { useUiStore } from '@/stores/ui'
import { usePlanViewer } from '@/pages/chat/usePlanViewer'
import type { JsonValue } from '@/types/json'

const props = defineProps<{
  open: boolean
  sessionId: string | null
}>()

const emit = defineEmits<{
  (event: 'update:open', open: boolean): void
}>()

const { t } = useI18n()
const ui = useUiStore()

type JsonRecord = Record<string, JsonValue>

const { loading, toggling, markdown, error, autorun, refresh, toggleAutorun } = usePlanViewer(
  () => [props.open, props.sessionId] as const,
  async (sessionId: string | null, tool: PlanTool, input: PlanToolInput, signal: AbortSignal): Promise<JsonRecord> => {
    const body = buildPlanToolInvocationRequest(sessionId, tool, input)
    if (!body) throw new Error(String(t('chat.planViewer.requiresSession')))
    return await apiJson<JsonRecord>('/api/v1/plugins/tools/invoke', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal,
    })
  },
)

function handleKeydown(event: KeyboardEvent) {
  if (event.metaKey || event.altKey || event.ctrlKey) return
  if (event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable="true"]'))
    return
  if (event.key === 'q') {
    event.preventDefault()
    emit('update:open', false)
  } else if (event.key === 'r' || event.key === 'R') {
    event.preventDefault()
    void refresh()
  } else if (event.key === 'a' || event.key === 'A') {
    event.preventDefault()
    void toggleAutorun()
  }
}

const description = computed(() =>
  props.sessionId
    ? String(t('chat.planViewer.session', { id: props.sessionId }))
    : String(t('chat.planViewer.requiresSession')),
)
</script>

<template>
  <Dialog
    :open="open"
    :title="t('chat.planViewer.title')"
    :description="description"
    max-width="max-w-4xl"
    mobile-fullscreen
    :body-scroll="false"
    @update:open="$emit('update:open', $event)"
  >
    <div
      class="flex min-h-0 flex-col"
      :aria-busy="loading || toggling"
      :class="ui.isCompactTouch ? 'h-full' : 'h-[min(65dvh,44rem)]'"
      data-transcript-chrome="true"
      data-plan-state-viewer="true"
      tabindex="-1"
      @keydown="handleKeydown"
    >
      <div class="flex shrink-0 flex-wrap items-center gap-3 border-b border-border/50 pb-2">
        <label v-if="autorun !== null" class="inline-flex min-h-9 cursor-pointer items-center gap-2 text-xs">
          <input
            type="checkbox"
            :checked="autorun"
            :disabled="toggling || loading"
            class="h-4 w-4"
            @change="toggleAutorun"
          />
          <span>{{ t('chat.planViewer.autorun') }}</span>
          <RiLoader4Line v-if="toggling" class="h-3.5 w-3.5 animate-spin text-muted-foreground" />
        </label>
        <span v-else class="text-xs text-muted-foreground">{{ t('chat.planViewer.noAutorun') }}</span>

        <IconButton
          variant="ghost"
          :class="ui.isTouchPointer ? 'h-10 w-10' : 'h-8 w-8'"
          :tooltip="t('chat.planViewer.refresh')"
          :aria-label="t('chat.planViewer.refresh')"
          :disabled="loading || toggling"
          @click="refresh"
        >
          <RiLoader4Line v-if="loading" class="h-4 w-4 animate-spin" />
          <RiRefreshLine v-else class="h-4 w-4" />
        </IconButton>
      </div>

      <div
        v-if="error"
        role="alert"
        class="my-2 border-l-2 border-destructive/60 pl-3 text-sm text-destructive [overflow-wrap:anywhere]"
      >
        {{ error }}
      </div>
      <div class="min-h-0 flex-1 overflow-auto overscroll-contain py-3">
        <div v-if="loading && !markdown" class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
          <RiLoader4Line class="h-4 w-4 animate-spin" />
          {{ t('chat.planViewer.loading') }}
        </div>
        <MarkdownRenderer v-else-if="markdown" :content="markdown" mode="markdown" :stream="false" />
        <div v-else class="py-8 text-sm text-muted-foreground">{{ t('chat.planViewer.empty') }}</div>
      </div>
    </div>
  </Dialog>
</template>
