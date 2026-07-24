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
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
          sessionState: {
            ...state.sessionState,
            active_execution: {
              execution_id: readString(payload.execution_id) || 'unknown',
              phase: 'starting',
            },
          },
        },
        shouldRefresh: false,
      }
    case 'execution_finished':
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
          sessionState: {
            ...state.sessionState,
            active_execution: null,
          },
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

export function applySessionEvent(
  state: ChatEventState,
  event: DomainEventRecord,
): { state: ChatEventState; shouldRefresh: boolean } {
  const payload = asRecord(event.payload)
  if (!payload) {
    return { state, shouldRefresh: true }
  }

  switch (event.kind) {
    case 'message_part_checkpointed':
    case 'message_part_delta':
    case 'user_message_appended':
      return requestConversationRefresh(state, event)
    case 'assistant_message_finished': {
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
        },
        shouldRefresh: true,
      }
    }
    case 'execution_started':
    case 'execution_finished':
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
