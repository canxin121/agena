<script setup lang="ts">
import { RiListOrdered2 } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import type { QueuedMessagePreview } from '@/pages/chat/queuedMessages'

defineProps<{ messages: QueuedMessagePreview[] }>()
const { t } = useI18n()
</script>

<template>
  <section
    v-if="messages.length"
    class="shrink-0 border-t border-border/60 bg-muted/20"
    :aria-label="t('chat.page.queue.title', { count: messages.length })"
    data-queued-messages
  >
    <div class="chat-column py-2">
      <div class="flex items-center gap-2 text-xs text-muted-foreground" role="status">
        <RiListOrdered2 class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span class="font-medium text-foreground/80">{{ t('chat.page.queue.title', { count: messages.length }) }}</span>
        <span class="truncate">{{ t('chat.page.queue.hint') }}</span>
      </div>
      <ol
        class="mt-1 max-h-[min(8rem,20vh)] overflow-y-auto overscroll-contain"
        :aria-label="t('chat.page.queue.order')"
      >
        <li v-for="(message, index) in messages" :key="message.messageId" class="flex items-start gap-2 py-1 text-xs">
          <span class="w-3.5 shrink-0 text-center font-mono tabular-nums text-muted-foreground" aria-hidden="true">{{
            index + 1
          }}</span>
          <p
            class="min-w-0 flex-1 line-clamp-2 break-words whitespace-pre-wrap text-foreground/80"
            :title="message.preview"
          >
            {{ message.preview || t('chat.page.queue.loading') }}
          </p>
        </li>
      </ol>
    </div>
  </section>
</template>
