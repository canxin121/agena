import { type DomainEventRecord, type MessageResource, type SessionExecutionResource } from '../lib/agenaApi'

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

export function appendTimelineEvent(
  timelineEvents: DomainEventRecord[],
  event: DomainEventRecord,
): DomainEventRecord[] {
  if (timelineEvents.some((item) => item.seq_global === event.seq_global)) {
    return timelineEvents
  }
  return [...timelineEvents, event].sort((left, right) => left.seq_global - right.seq_global)
}

function requestConversationRefresh(
  state: ChatEventState,
  event: DomainEventRecord,
): { state: ChatEventState; shouldRefresh: boolean } {
  return {
    state: {
      ...state,
      timelineEvents: appendTimelineEvent(state.timelineEvents, event),
    },
    shouldRefresh: true,
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
    case 'execution_started':
    case 'run_started':
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
    case 'run_completed':
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
    case 'execution_failed':
    case 'run_aborted':
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
    case 'message_part_updated':
    case 'message_part_delta':
    case 'user_message_appended':
      return requestConversationRefresh(state, event)
    case 'assistant_message_completed': {
      const withTimeline = {
        ...state,
        timelineEvents: appendTimelineEvent(state.timelineEvents, event),
      }
      if (!withTimeline.sessionState) {
        return {
          state: withTimeline,
          shouldRefresh: true,
        }
      }
      return {
        state: {
          ...withTimeline,
          sessionState: {
            ...withTimeline.sessionState,
            blocked: false,
            run_state: 'idle',
          },
        },
        shouldRefresh: true,
      }
    }
    case 'execution_started':
    case 'execution_failed':
    case 'run_started':
    case 'run_completed':
    case 'run_aborted':
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
