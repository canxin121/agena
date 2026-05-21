import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { MessageResource, SessionEventRecord, SessionExecutionResource, TimelineEventRecord } from '../lib/agenaApi'
import { useChatConversationRuntime } from './useChatConversationRuntime'

function message(sessionId: number): MessageResource {
  return {
    id: 21,
    session_id: sessionId,
    role: 'assistant',
    state: 'complete',
    created_at: '2026-05-10T00:00:00Z',
    updated_at: '2026-05-10T00:00:00Z',
    metadata: {},
    usage: null,
    finish: null,
    part_count: 0,
    parts: [],
  }
}

function sessionState(sessionId: number, overrides?: Partial<SessionExecutionResource>): SessionExecutionResource {
  return {
    session: {
      id: sessionId,
      workspace_id: 1,
      title: `session-${sessionId}`,
      version: 1,
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      message_count: 0,
      child_session_count: 0,
      parent_id: 1,
    },
    blocked: false,
    run_state: 'idle',
    latest_event_seq: 4,
    execution: { allowed_tools: [] },
    pending_permission_requests: [],
    pending_user_input_requests: [],
    ...overrides,
    usage: overrides?.usage ?? { current_tokens: 0 },
  }
}

describe('useChatConversationRuntime', () => {
  test('refreshConversation loads state, messages, timeline, tree, checkpoints, and opens the event stream', async () => {
    const calls: string[] = []
    const input = {
      errorMessage: ref(''),
      loading: ref(false),
      messages: ref<MessageResource[]>([]),
      selectedSessionId: ref<number | null>(3),
      sessionState: ref<SessionExecutionResource | null>(null),
      timelineEvents: ref<TimelineEventRecord[]>([]),
    }

    const runtime = useChatConversationRuntime(
      input,
      {
        applySessionEvent: (state) => ({ state, shouldRefresh: false }),
        getSessionState: async (sessionId) => {
          calls.push(`getSessionState:${sessionId}`)
          return sessionState(sessionId)
        },
        listMessages: async (sessionId) => {
          calls.push(`listMessages:${sessionId}`)
          return [message(sessionId)]
        },
        listSessionTimeline: async (sessionId) => {
          calls.push(`listSessionTimeline:${sessionId}`)
          return [{ session_id: sessionId, seq_global: 1, kind: 'run_started', created_at: '2026-05-10T00:00:00Z', payload: {} }]
        },
        streamSessionEvents: (sessionId) => {
          calls.push(`streamSessionEvents:${sessionId}`)
          return {
            close() {
              calls.push(`closeStream:${sessionId}`)
            },
          }
        },
      },
      {
        loadRewindCheckpoints: async (sessionId) => {
          calls.push(`loadRewindCheckpoints:${sessionId}`)
        },
        loadSessionTree: async (rootId) => {
          calls.push(`loadSessionTree:${rootId}`)
        },
      },
    )

    await runtime.refreshConversation(true)

    expect(calls).toEqual([
      'getSessionState:3',
      'listMessages:3',
      'listSessionTimeline:3',
      'loadSessionTree:1',
      'loadRewindCheckpoints:3',
      'streamSessionEvents:3',
    ])
    expect(input.loading.value).toBe(false)
    expect(input.sessionState.value?.session.id).toBe(3)
    expect(input.messages.value.map((item) => item.id)).toEqual([21])
    expect(input.timelineEvents.value.map((item) => item.seq_global)).toEqual([1])

    runtime.dispose()
    expect(calls.includes('closeStream:3')).toBe(true)
  })

  test('applies streamed events to the active session state', () => {
    const input = {
      errorMessage: ref(''),
      loading: ref(false),
      messages: ref<MessageResource[]>([]),
      selectedSessionId: ref<number | null>(3),
      sessionState: ref<SessionExecutionResource | null>(sessionState(3)),
      timelineEvents: ref<TimelineEventRecord[]>([]),
    }

    let handlers: { onEvent?: (event: SessionEventRecord) => void } = {}

    const runtime = useChatConversationRuntime(
      input,
      {
        applySessionEvent: (state, event) => ({
          state: {
            ...state,
            messages: [
              ...state.messages,
              {
                ...message(state.selectedSessionId ?? 0),
                id: event.seq,
              },
            ],
          },
          shouldRefresh: false,
        }),
        getSessionState: async () => sessionState(3),
        listMessages: async () => [],
        listSessionTimeline: async () => [],
        streamSessionEvents: (_sessionId, options) => {
          handlers = options
          return { close() {} }
        },
      },
      {
        loadRewindCheckpoints: async () => {},
        loadSessionTree: async () => {},
      },
    )

    runtime.syncEventStream()
    handlers.onEvent?.({ session_id: 3, seq: 9, event_type: 'message_updated', created_at: '2026-05-10T00:00:00Z', payload: {} })

    expect(input.sessionState.value?.latest_event_seq).toBe(9)
    expect(input.messages.value.map((item) => item.id)).toEqual([9])
  })
})
