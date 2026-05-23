import {
  messageResourceFromEvent,
  type DomainEventRecord,
  type MessagePart,
  type MessageResource,
  type SessionExecutionResource,
} from '../lib/agenaApi'

export type ChatEventState = {
  messages: MessageResource[]
  timelineEvents: DomainEventRecord[]
  sessionState: SessionExecutionResource | null
  selectedSessionId: number | null
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

function readFiniteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

export function sortMessages(items: MessageResource[]): MessageResource[] {
  return [...items].sort((left, right) => {
    const leftTime = Date.parse(left.created_at)
    const rightTime = Date.parse(right.created_at)
    if (Number.isFinite(leftTime) && Number.isFinite(rightTime) && leftTime !== rightTime) {
      return leftTime - rightTime
    }
    return left.id - right.id
  })
}

export function sortMessageParts(items: MessagePart[]): MessagePart[] {
  return [...items].sort((left, right) => {
    if (left.part_index !== right.part_index) {
      return left.part_index - right.part_index
    }
    return left.id - right.id
  })
}

export function appendTimelineEvent(
  timelineEvents: DomainEventRecord[],
  event: DomainEventRecord,
): DomainEventRecord[] {
  if (timelineEvents.some((item) => item.seq_global === event.seq_global)) {
    return timelineEvents
  }
  return [...timelineEvents, event].sort((left, right) => left.seq_global - right.seq_global)
}

function applyMessagePartUpdatedEvent(
  state: ChatEventState,
  event: DomainEventRecord,
): { state: ChatEventState; shouldRefresh: boolean } {
  const message = messageResourceFromEvent(event)
  const part = Array.isArray(message?.parts) ? message.parts[0] || null : null
  if (!message || !part) {
    return { state, shouldRefresh: true }
  }

  return {
    state: {
      ...state,
      messages: upsertMessage(state.messages, message),
    },
    shouldRefresh: part.status !== 'pending' || message.state !== 'pending',
  }
}

function applyMessagePartDeltaEvent(
  state: ChatEventState,
  payload: Record<string, unknown>,
): { state: ChatEventState; shouldRefresh: boolean } {
  const messageId = readFiniteNumber(payload.message_id)
  const partId = readFiniteNumber(payload.part_id)
  const field = readString(payload.field)
  const delta = typeof payload.delta === 'string' ? payload.delta : ''

  if (messageId === null || partId === null || !field) {
    return { state, shouldRefresh: true }
  }
  if (field !== 'text') {
    return { state, shouldRefresh: true }
  }

  const nextMessages = state.messages.slice()
  const messageIndex = nextMessages.findIndex((message) => message.id === messageId)
  if (messageIndex < 0) {
    return { state, shouldRefresh: true }
  }

  const existing = nextMessages[messageIndex]
  const nextParts = Array.isArray(existing.parts) ? existing.parts.slice() : []
  const targetIndex = nextParts.findIndex((part) => part.id === partId)
  if (targetIndex < 0) {
    return { state, shouldRefresh: true }
  }

  const target = nextParts[targetIndex]
  const content = asRecord(target.content)
  if (!content || content.type !== 'text') {
    return { state, shouldRefresh: true }
  }

  nextParts[targetIndex] = {
    ...target,
    content: {
      ...content,
      text: `${typeof content.text === 'string' ? content.text : ''}${delta}`,
    },
  }
  nextMessages[messageIndex] = {
    ...existing,
    parts: sortMessageParts(nextParts),
  }
  return {
    state: {
      ...state,
      messages: sortMessages(nextMessages),
    },
    shouldRefresh: false,
  }
}

function applyHistoryMessageEvent(
  state: ChatEventState,
  event: DomainEventRecord,
): { state: ChatEventState; shouldRefresh: boolean } {
  const message = messageResourceFromEvent(event)
  if (!message) {
    return { state, shouldRefresh: true }
  }
  return {
    state: {
      ...state,
      messages: upsertMessage(state.messages, message),
    },
    shouldRefresh: false,
  }
}

function patchSessionStateFromEvent(
  state: ChatEventState,
  event: DomainEventRecord,
  payload: Record<string, unknown>,
): { state: ChatEventState; shouldRefresh: boolean } {
  const nextTimelineEvents = appendTimelineEvent(state.timelineEvents, event)
  if (!state.sessionState) {
    return {
      state: {
        ...state,
        timelineEvents: nextTimelineEvents,
      },
      shouldRefresh: true,
    }
  }

  switch (event.kind) {
    case 'run_started':
    case 'turn_started':
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
          sessionState: {
            ...state.sessionState,
            blocked: false,
            run_state: 'awaiting_model',
          },
        },
        shouldRefresh: false,
      }
    case 'turn_completed':
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
          sessionState: {
            ...state.sessionState,
            blocked: false,
            run_state: 'idle',
          },
        },
        shouldRefresh: false,
      }
    case 'run_failed':
    case 'turn_aborted':
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
          sessionState: {
            ...state.sessionState,
            blocked: true,
            run_state: readString(payload.run_state) || state.sessionState.run_state,
          },
        },
        shouldRefresh: false,
      }
    default:
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
        },
        shouldRefresh: false,
      }
  }
}

export function applySessionEvent(
  state: ChatEventState,
  event: DomainEventRecord,
): { state: ChatEventState; shouldRefresh: boolean } {
  const payload = asRecord(event.payload)
  if (!payload) {
    return { state, shouldRefresh: true }
  }

  switch (event.kind) {
    case 'message_part_updated': {
      const withTimeline = {
        ...state,
        timelineEvents: appendTimelineEvent(state.timelineEvents, event),
      }
      return applyMessagePartUpdatedEvent(withTimeline, event)
    }
    case 'message_part_delta':
      return applyMessagePartDeltaEvent(state, payload)
    case 'user_message_appended': {
      const withTimeline = {
        ...state,
        timelineEvents: appendTimelineEvent(state.timelineEvents, event),
      }
      return applyHistoryMessageEvent(withTimeline, event)
    }
    case 'assistant_message_completed': {
      const withTimeline = {
        ...state,
        timelineEvents: appendTimelineEvent(state.timelineEvents, event),
      }
      const result = applyHistoryMessageEvent(withTimeline, event)
      if (!result.state.sessionState) {
        return result
      }
      return {
        state: {
          ...result.state,
          sessionState: {
            ...result.state.sessionState,
            blocked: false,
            run_state: 'idle',
          },
        },
        shouldRefresh: result.shouldRefresh,
      }
    }
    case 'run_started':
    case 'run_failed':
    case 'turn_started':
    case 'turn_completed':
    case 'turn_aborted':
      return patchSessionStateFromEvent(state, event, payload)
    default:
      return {
        state: {
          ...state,
          timelineEvents: appendTimelineEvent(state.timelineEvents, event),
        },
        shouldRefresh: true,
      }
  }
}

function upsertMessage(messages: MessageResource[], incoming: MessageResource): MessageResource[] {
  const nextMessages = messages.slice()
  const messageIndex = nextMessages.findIndex((message) => message.id === incoming.id)
  if (messageIndex < 0) {
    nextMessages.push({
      ...incoming,
      parts: Array.isArray(incoming.parts) ? sortMessageParts(incoming.parts) : incoming.parts,
    })
    return sortMessages(nextMessages)
  }
  nextMessages[messageIndex] = mergeMessageResources(nextMessages[messageIndex], incoming)
  return sortMessages(nextMessages)
}

function mergeMessageResources(existing: MessageResource, incoming: MessageResource): MessageResource {
  const parts = mergeMessageParts(existing.parts, incoming.parts)
  return {
    ...existing,
    ...incoming,
    state: messageStatusRank(incoming.state) >= messageStatusRank(existing.state) ? incoming.state : existing.state,
    updated_at: laterTimestamp(existing.updated_at, incoming.updated_at),
    metadata: Object.keys(incoming.metadata || {}).length > 0 ? incoming.metadata : existing.metadata,
    usage: incoming.usage ?? existing.usage ?? null,
    finish: incoming.finish ?? existing.finish ?? null,
    part_count: Math.max(existing.part_count, incoming.part_count, Array.isArray(parts) ? parts.length : 0),
    parts,
  }
}

function mergeMessageParts(
  existing: MessageResource['parts'],
  incoming: MessageResource['parts'],
): MessageResource['parts'] {
  if (Array.isArray(existing) && Array.isArray(incoming)) {
    const merged = new Map<number, MessagePart>()
    for (const part of existing) merged.set(part.id, part)
    for (const part of incoming) merged.set(part.id, part)
    return sortMessageParts([...merged.values()])
  }
  if (Array.isArray(incoming)) {
    return sortMessageParts(incoming)
  }
  if (Array.isArray(existing)) {
    return sortMessageParts(existing)
  }
  return incoming ?? existing
}

function laterTimestamp(left: string, right: string): string {
  const leftTime = Date.parse(left)
  const rightTime = Date.parse(right)
  if (!Number.isFinite(leftTime)) return right
  if (!Number.isFinite(rightTime)) return left
  return rightTime >= leftTime ? right : left
}

function messageStatusRank(status: string): number {
  switch (status) {
    case 'completed':
    case 'failed':
    case 'cancelled':
    case 'aborted':
      return 2
    case 'in_progress':
    case 'awaiting_model':
      return 1
    default:
      return 0
  }
}
