<script setup lang="ts">
import type { MessagePart, MessageResource } from '@/agena/lib/agenaApi'

type ChatMessageRenderBlock = {
  body: string
  kind: 'text' | 'diff'
  summary?: string
}

const props = defineProps<{
  selectedSessionId: number | null
  loading: boolean
  messages: MessageResource[]
  inspectedMessage: MessageResource | null
  inspectedMessageParts: MessagePart[]
  inspectedPart: MessagePart | null
  refreshConversation: (foreground: boolean) => void | Promise<void>
  inspectMessage: (messageId: number, partId?: number) => void | Promise<void>
  rewindToMessage: (messageId: number) => void | Promise<void>
  formatMessageTime: (value: string) => string
  messageUsageFacts: (message: MessageResource) => string[]
  messageBlocks: (message: MessageResource) => ChatMessageRenderBlock[]
}>()
</script>

<template>
  <section id="chat-messages-panel" class="card" tabindex="-1">
    <div class="page-header" style="margin-bottom: 12px">
      <h3 style="margin: 0">Messages</h3>
      <div class="button-row">
        <button
          class="button ghost"
          :disabled="!props.selectedSessionId || props.loading"
          @click="props.refreshConversation(true)"
        >
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
            <button class="button ghost" :disabled="props.loading" @click="props.inspectMessage(message.id)">
              Inspect
            </button>
            <button class="button ghost" :disabled="props.loading" @click="props.rewindToMessage(message.id)">
              Rewind Here
            </button>
            <div>{{ props.formatMessageTime(message.created_at) }}</div>
          </div>
        </div>
        <div v-if="props.messageUsageFacts(message).length" class="stack">
          <div v-if="props.messageUsageFacts(message).length" class="muted mono">
            usage={{ props.messageUsageFacts(message).join(' · ') }}
          </div>
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

    <section v-if="props.inspectedMessage" class="card" style="margin-top: 16px">
      <div class="page-header" style="margin-bottom: 12px">
        <h3 style="margin: 0">Message Inspector</h3>
        <div class="muted mono">
          message={{ props.inspectedMessage.id }} · parts={{ props.inspectedMessageParts.length }} · summary list · full
          detail on demand
        </div>
      </div>
      <div class="stack">
        <div><strong>Role:</strong> {{ props.inspectedMessage.role }}</div>
        <div><strong>State:</strong> {{ props.inspectedMessage.state }}</div>
        <div class="muted mono">metadata={{ JSON.stringify(props.inspectedMessage.metadata, null, 2) }}</div>
        <div v-if="props.inspectedMessageParts.length" class="list">
          <div v-for="part in props.inspectedMessageParts" :key="part.id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div>
                  <strong>#{{ part.part_index }}</strong> · {{ part.kind }}
                </div>
                <div class="muted">{{ part.summary || 'No summary' }}</div>
                <div class="muted mono">
                  part={{ part.id }} · status={{ part.status }} · detail={{ part.has_detail ? 'yes' : 'no' }}
                </div>
              </div>
              <button
                class="button ghost"
                :disabled="props.loading || !part.has_detail"
                @click="props.inspectMessage(props.inspectedMessage!.id, part.id)"
              >
                {{ part.has_detail ? 'Load Detail' : 'Summary Only' }}
              </button>
            </div>
          </div>
        </div>
        <div v-if="props.inspectedPart" class="stack">
          <strong>Selected Part</strong>
          <div class="muted mono">
            part={{ props.inspectedPart.id }} · operation={{ props.inspectedPart.operation_id || 'n/a' }} · detail={{
              props.inspectedPart.has_detail ? 'full' : 'summary-only'
            }}
          </div>
          <pre v-if="props.inspectedPart.content != null" class="message-block mono">{{
            JSON.stringify(props.inspectedPart.content, null, 2)
          }}</pre>
          <div v-else class="muted">{{ props.inspectedPart.summary || 'No full detail stored for this part.' }}</div>
        </div>
      </div>
    </section>
  </section>
</template>
