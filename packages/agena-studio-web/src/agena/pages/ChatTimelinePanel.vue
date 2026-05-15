<script setup lang="ts">
import type { TimelineEventRecord } from '@/agena/lib/agenaApi'

const props = defineProps<{
  timelineEvents: TimelineEventRecord[]
  formatMessageTime: (value: string) => string
  formatEventTime: (timestampMs: number) => string
  readPayloadMessageId: (payload: Record<string, unknown>) => number | null
  readPayloadPartId: (payload: Record<string, unknown>) => number | null
  inspectMessage: (messageId: number, partId?: number) => void | Promise<void>
  scrollToMessage: (messageId: number) => void
}>()

function summaryFor(event: TimelineEventRecord): string {
  return typeof event.payload.summary === 'string'
    ? event.payload.summary
    : typeof event.payload.command === 'string'
      ? event.payload.command
      : typeof event.payload.message === 'string'
        ? event.payload.message
        : event.kind
}

function inspectTargetFor(payload: Record<string, unknown>): { messageId: number; partId: number } | null {
  const messageId = props.readPayloadMessageId(payload)
  const partId = props.readPayloadPartId(payload)
  if (messageId === null || partId === null) return null
  return { messageId, partId }
}
</script>

<template>
  <section class="card">
    <div class="page-header" style="margin-bottom: 12px">
      <h3 style="margin: 0">Timeline</h3>
      <div class="muted mono">events={{ props.timelineEvents.length }}</div>
    </div>
    <div v-if="props.timelineEvents.length" class="list">
      <div v-for="event in props.timelineEvents" :key="event.seq_global" class="list-item">
        <div>
          <strong>{{ event.kind }}</strong>
        </div>
        <div class="muted">{{ summaryFor(event) }}</div>
        <div v-if="props.readPayloadMessageId(event.payload) !== null" class="button-row" style="margin-top: 6px">
          <span class="muted mono">message_id={{ props.readPayloadMessageId(event.payload) }}</span>
          <span v-if="props.readPayloadPartId(event.payload) !== null" class="muted mono">
            part_id={{ props.readPayloadPartId(event.payload) }}
          </span>
          <button class="button ghost" @click="props.scrollToMessage(props.readPayloadMessageId(event.payload)!)">
            Jump to Message
          </button>
          <button
            v-if="inspectTargetFor(event.payload)"
            class="button ghost"
            @click="props.inspectMessage(inspectTargetFor(event.payload)!.messageId, inspectTargetFor(event.payload)!.partId)"
          >
            Inspect Activity
          </button>
        </div>
        <div class="muted mono">
          seq={{ event.seq_global }} · session={{ event.session_id ?? 'n/a' }} ·
          {{ event.created_at ? props.formatMessageTime(event.created_at) : props.formatEventTime(event.ts_ms ?? 0) }}
        </div>
      </div>
    </div>
    <p v-else class="muted">No timeline events yet.</p>
  </section>
</template>
