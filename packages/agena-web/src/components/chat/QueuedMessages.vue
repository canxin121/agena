<script setup lang="ts">
import { RiDeleteBinLine, RiEditLine, RiListOrdered2 } from '@remixicon/vue'
import { ref, watch } from 'vue'
import QueuedMessageEditor from './QueuedMessageEditor.vue'
import { useI18n } from 'vue-i18n'
import IconButton from '@/components/ui/IconButton.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import type { QueuedMessagePreview } from '@/pages/chat/queuedMessages'

const props = defineProps<{
  sessionId?: string | null
  messages: QueuedMessagePreview[]
  paused?: boolean
  busy?: boolean
  stopChoiceOpen?: boolean
}>()
const emit = defineEmits<{
  remove: [executionId: string]
  clear: []
  pause: []
  resume: []
  keepStopped: []
}>()
const { t } = useI18n()
const editing = ref('')
watch(
  () => props.sessionId,
  () => {
    editing.value = ''
  },
)
</script>

<template>
  <section
    v-if="messages.length || paused"
    class="shrink-0 border-t border-border/60 bg-muted/20"
    :aria-label="t('chat.page.queue.title', { count: messages.length })"
    :aria-busy="busy"
    data-queued-messages
  >
    <div class="chat-column py-2">
      <div class="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
        <RiListOrdered2 class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span class="font-medium text-foreground/80" role="status">{{
          t('chat.page.queue.title', { count: messages.length })
        }}</span>
        <span v-if="paused" class="rounded bg-muted px-1.5 py-0.5 font-medium">{{ t('chat.page.queue.paused') }}</span>
        <div class="ml-auto flex items-center gap-3">
          <button
            type="button"
            class="min-h-7 text-foreground/80 hover:text-foreground disabled:opacity-40"
            :disabled="busy"
            @click="paused ? emit('resume') : emit('pause')"
          >
            {{ t(paused ? 'chat.page.queue.resume' : 'chat.page.queue.pause') }}
          </button>
          <button
            v-if="messages.length"
            type="button"
            class="min-h-7 hover:text-destructive disabled:opacity-40"
            :disabled="busy"
            @click="emit('clear')"
          >
            {{ t('chat.page.queue.clear') }}
          </button>
        </div>
      </div>
      <p class="mt-0.5 text-xs text-muted-foreground">
        {{ t(paused ? 'chat.page.queue.pausedHint' : 'chat.page.queue.hint') }}
      </p>
      <ol
        v-if="messages.length"
        class="mt-1 max-h-[min(10rem,24vh)] overflow-y-auto overscroll-contain"
        :aria-label="t('chat.page.queue.order')"
      >
        <li
          v-for="(message, index) in messages"
          :key="message.messageId"
          class="flex items-start gap-2 border-t border-border/30 py-1.5 text-xs"
        >
          <span
            class="mt-1.5 w-3.5 shrink-0 text-center font-mono tabular-nums text-muted-foreground"
            aria-hidden="true"
            >{{ index + 1 }}</span
          >
          <p
            class="min-w-0 flex-1 self-center line-clamp-2 break-words whitespace-pre-wrap text-foreground/80"
            :title="message.preview"
          >
            {{ message.preview || t('chat.page.queue.loading') }}
          </p>
          <div class="flex shrink-0 items-center gap-0.5">
            <IconButton
              :title="t('chat.page.queue.edit')"
              :aria-label="t('chat.page.queue.edit')"
              :disabled="busy || !message.executionId || !sessionId"
              @click="editing = message.executionId"
            >
              <RiEditLine class="h-3.5 w-3.5" aria-hidden="true" />
            </IconButton>
            <IconButton
              :title="t('chat.page.queue.remove')"
              :aria-label="t('chat.page.queue.remove')"
              :disabled="busy || !message.executionId"
              @click="emit('remove', message.executionId)"
            >
              <RiDeleteBinLine class="h-3.5 w-3.5" aria-hidden="true" />
            </IconButton>
          </div>
        </li>
      </ol>
    </div>
  </section>
  <QueuedMessageEditor
    v-if="editing && sessionId"
    :key="`${sessionId}:${editing}`"
    :session-id="sessionId"
    :execution-id="editing"
    @close="editing = ''"
  />
  <FormDialog
    :open="Boolean(stopChoiceOpen)"
    :title="t('chat.page.queue.stoppedTitle')"
    :description="t('chat.page.queue.stoppedHint', { count: messages.length })"
    max-width="max-w-md"
    @close="emit('keepStopped')"
  >
    <p class="text-sm text-muted-foreground">{{ t('chat.page.queue.pausedHint') }}</p>
    <template #footer>
      <button
        type="button"
        class="ml-auto rounded-md border border-border px-3 py-2 text-sm hover:bg-muted disabled:opacity-40"
        :disabled="busy"
        @click="emit('keepStopped')"
      >
        {{ t('chat.page.queue.keepStopped') }}
      </button>
      <button
        type="button"
        class="rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-40"
        :disabled="busy || !messages.length"
        @click="emit('resume')"
      >
        {{ t('chat.page.queue.sendNext') }}
      </button>
    </template>
  </FormDialog>
</template>
