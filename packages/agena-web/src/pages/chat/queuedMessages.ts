import type { MessageLike } from '@/components/chat/messageList.types'
import type { QueuedMessage } from '@/stores/chat/api'
import { isQueuedUserMessage } from './conversationMessages'

export type QueuedMessagePreview = {
  messageId: string
  preview: string
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function messagePreview(message: MessageLike | undefined): string {
  const content = record(message?.info.runContent)
  const body =
    typeof content.text === 'string' ? content.text : message?.parts.map((part) => part.text || '').join('') || ''
  if (body.trim()) return body.replace(/\s+/g, ' ').trim().slice(0, 320)
  const document = Array.isArray(content.document) ? content.document : []
  return document
    .flatMap((node) => {
      const payload = record(record(record(node).activity).payload)
      const label = payload.name || payload.label || payload.text
      return typeof label === 'string' && label.trim() ? [label.trim()] : []
    })
    .join(' · ')
    .replace(/\s+/g, ' ')
    .slice(0, 320)
}

/** A display projection only. Delivery and removal remain backend operations. */
export function queuedMessagePreviews(
  messages: MessageLike[],
  queue: readonly QueuedMessage[],
): QueuedMessagePreview[] {
  const byId = new Map(messages.map((message) => [String(message.info.id), message]))
  const pending = new Set(queue.map((item) => String(item.message_id)))
  // A live pending boundary may arrive before its execution shell. Conversely,
  // a claimed boundary can retire an older queue reference immediately.
  for (const message of messages) {
    const id = String(message.info.id)
    if (isQueuedUserMessage(message)) pending.add(id)
    else if (message.info.runState && record(message.info.runContent).execution_phase !== 'queued') pending.delete(id)
  }
  return [...pending]
    .sort((left, right) => Number(left) - Number(right))
    .map((messageId) => ({
      messageId,
      preview: messagePreview(byId.get(messageId)),
    }))
}
