import type { MessagePart, MessageResource, SessionEventRecord, SessionExecutionResource, TimelineEventRecord } from '@/agena/lib/agenaApi'

export type ChatEventState = {
  messages: MessageResource[]
  timelineEvents: TimelineEventRecord[]
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
  timelineEvents: TimelineEventRecord[],
  event: SessionEventRecord,
): TimelineEventRecord[] {
  const record: TimelineEventRecord = {
    event_id: event.event_id,
    session_id: event.session_id,
    seq_global: event.seq,
    causation_id: event.causation_id,
    correlation_id: event.correlation_id,
    created_at: event.created_at,
    kind: event.event_type,
    payload: event.payload,
  }
  if (timelineEvents.some((item) => item.seq_global === record.seq_global)) {
    return timelineEvents
  }
  return [...timelineEvents, record].sort((left, right) => left.seq_global - right.seq_global)
}

function applyMessagePartUpdatedEvent(
  state: ChatEventState,
  payload: Record<string, unknown>,
): { state: ChatEventState; shouldRefresh: boolean } {
  const messageId = readFiniteNumber(payload.message_id)
  const messageRole = readString(payload.message_role) as MessageResource['role'] | null
  const messageState = readString(payload.message_state)
  const messageCreatedAt = readString(payload.message_created_at)
  const part = asRecord(payload.part) as MessagePart | null

  if (
    !state.selectedSessionId ||
    messageId === null ||
    !messageRole ||
    !messageState ||
    !messageCreatedAt ||
    !part
  ) {
    return { state, shouldRefresh: true }
  }

  const nextMessages = state.messages.slice()
  const messageIndex = nextMessages.findIndex((message) => message.id === messageId)
  if (messageIndex < 0) {
    nextMessages.push({
      id: messageId,
      session_id: state.selectedSessionId,
      role: messageRole,
      state: messageState,
      created_at: messageCreatedAt,
      updated_at: messageCreatedAt,
      metadata: {},
      usage: null,
      finish: null,
      part_count: 1,
      parts: [part],
    })
    return {
      state: {
        ...state,
        messages: sortMessages(nextMessages),
      },
      shouldRefresh: part.status !== 'pending' || messageState !== 'pending',
    }
  }

  const existing = nextMessages[messageIndex]
  const nextParts = Array.isArray(existing.parts) ? existing.parts.slice() : []
  const partIndex = nextParts.findIndex((item) => item.id === part.id)
  if (partIndex >= 0) {
    nextParts[partIndex] = part
  } else {
    nextParts.push(part)
  }

  nextMessages[messageIndex] = {
    ...existing,
    role: messageRole,
    state: messageState,
    created_at: messageCreatedAt,
    part_count: Math.max(existing.part_count, nextParts.length),
    parts: sortMessageParts(nextParts),
  }
  return {
    state: {
      ...state,
      messages: sortMessages(nextMessages),
    },
    shouldRefresh: part.status !== 'pending' || messageState !== 'pending',
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

function patchSessionStateFromEvent(
  state: ChatEventState,
  event: SessionEventRecord,
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

  switch (event.event_type) {
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
    case 'assistant_message_completed':
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
    case 'message_revised':
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
        },
        shouldRefresh: true,
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

export function applySessionEvent(state: ChatEventState, event: SessionEventRecord): { state: ChatEventState; shouldRefresh: boolean } {
  const payload = asRecord(event.payload)
  if (!payload) {
    return { state, shouldRefresh: true }
  }

  switch (event.event_type) {
    case 'message_part_updated': {
      const withTimeline = {
        ...state,
        timelineEvents: appendTimelineEvent(state.timelineEvents, event),
      }
      return applyMessagePartUpdatedEvent(withTimeline, payload)
    }
    case 'message_part_delta':
      return applyMessagePartDeltaEvent(state, payload)
    case 'user_message_appended':
    case 'run_started':
    case 'run_failed':
    case 'turn_started':
    case 'turn_completed':
    case 'turn_aborted':
    case 'assistant_message_completed':
    case 'message_revised':
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
