<script setup lang="ts">
import type { MessageResource } from '@/agena/lib/agenaApi'

type ChatMessageRenderBlock = {
  body: string
  kind: 'text' | 'diff'
  summary?: string
}

const props = defineProps<{
  selectedSessionId: number | null
  loading: boolean
  messages: MessageResource[]
  refreshConversation: (foreground: boolean) => void | Promise<void>
  rewindToMessage: (messageId: number) => void | Promise<void>
  formatMessageTime: (value: string) => string
  messageTags: (message: MessageResource) => string[]
  messageUsageFacts: (message: MessageResource) => string[]
  messageBlocks: (message: MessageResource) => ChatMessageRenderBlock[]
}>()
</script>

<template>
  <section class="card">
    <div class="page-header" style="margin-bottom: 12px">
      <h3 style="margin: 0">Messages</h3>
      <div class="button-row">
        <button class="button ghost" :disabled="!props.selectedSessionId || props.loading" @click="props.refreshConversation(true)">
          Refresh
        </button>
      </div>
    </div>

    <div v-if="props.messages.length" class="message-list">
      <article
        v-for="message in props.messages"
        :key="message.id"
        class="message"
        :class="message.role"
        :data-message-id="message.id"
      >
        <div class="message-head">
          <div class="message-role">{{ message.role }}</div>
          <div class="button-row">
            <button class="button ghost" :disabled="props.loading" @click="props.rewindToMessage(message.id)">Rewind Here</button>
            <div>{{ props.formatMessageTime(message.created_at) }}</div>
          </div>
        </div>
        <div
          v-if="props.messageTags(message).length || props.messageUsageFacts(message).length || message.finish"
          class="stack"
        >
          <div v-if="props.messageTags(message).length" class="button-row">
            <span v-for="tag in props.messageTags(message)" :key="`${message.id}-tag-${tag}`" class="badge">
              {{ tag }}
            </span>
          </div>
          <div v-if="props.messageUsageFacts(message).length" class="muted mono">
            usage={{ props.messageUsageFacts(message).join(' · ') }}
          </div>
          <div v-if="message.finish" class="muted mono">finish={{ message.finish }}</div>
        </div>
        <div v-if="props.messageBlocks(message).length" class="stack">
          <template v-for="(block, index) in props.messageBlocks(message)" :key="`${message.id}-${index}`">
            <details v-if="block.kind === 'diff'" class="message-diff">
              <summary>{{ block.summary || 'Patch diff' }}</summary>
              <pre class="message-block mono">{{ block.body }}</pre>
            </details>
            <pre v-else class="message-block mono">{{ block.body }}</pre>
          </template>
        </div>
        <div v-else class="muted">No renderable parts.</div>
      </article>
    </div>
    <p v-else class="muted">No messages yet.</p>
  </section>
</template>
