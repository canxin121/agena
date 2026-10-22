import type { MessageLike } from '@/components/chat/messageList.types'

type MessageIdentity = { info?: { id?: string; role?: string; runState?: string; finish?: string; messageId?: number } }

export function isQueuedUserMessage(message: MessageIdentity): boolean {
  return message.info?.role === 'user' && (message.info.runState || message.info.finish) === 'pending'
}

export function isQueueOnlyMessage(message: MessageLike): boolean {
  const content = message.info.runContent
  return (
    message.info.role === 'user' &&
    (isQueuedUserMessage(message) ||
      (typeof content === 'object' && content !== null && !Array.isArray(content) && content.queue_only === true))
  )
}

function compareIds(left: string, right: string): number {
  if (/^\d+$/.test(left) && /^\d+$/.test(right)) {
    const a = BigInt(left)
    const b = BigInt(right)
    return a < b ? -1 : a > b ? 1 : 0
  }
  return left.localeCompare(right)
}

/** Keep the indexed store in append order; only presentation follows input ownership. */
export function conversationMessages<T extends MessageLike>(messages: readonly T[]): T[] {
  const visible = messages.filter((message) => !isQueueOnlyMessage(message))
  if (!visible.some((message) => message.info.messageId !== undefined)) return visible
  return visible.sort((left, right) => {
    const owner = compareIds(
      String(left.info.messageId ?? left.info.id ?? ''),
      String(right.info.messageId ?? right.info.id ?? ''),
    )
    if (owner) return owner
    if (left.info.role === 'user' && right.info.role !== 'user') return -1
    if (right.info.role === 'user' && left.info.role !== 'user') return 1
    return compareIds(String(left.info.id ?? ''), String(right.info.id ?? ''))
  })
}

export function sameAssistantReply(left: MessageLike, right: MessageLike): boolean {
  return (
    left.info.role === 'assistant' &&
    right.info.role === 'assistant' &&
    (left.info.messageId === undefined ||
      right.info.messageId === undefined ||
      left.info.messageId === right.info.messageId)
  )
}
