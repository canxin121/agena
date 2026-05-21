import { describe, expect, test } from 'bun:test'

import type { MessageResource, SessionEventRecord, SessionExecutionResource } from '@/agena/lib/agenaApi'
import { applySessionEvent, appendTimelineEvent, sortMessages, sortMessageParts, type ChatEventState } from './chatPageModel'

function sampleSessionState(): SessionExecutionResource {
  return {
    session: {
      id: 7,
      workspace_id: 3,
      title: 'session',
      version: 1,
      created_at: '2026-05-08T00:00:00Z',
      updated_at: '2026-05-08T00:00:00Z',
      message_count: 0,
      child_session_count: 0,
    },
    blocked: false,
    run_state: 'idle',
    latest_event_seq: 1,
    execution: {
      allowed_tools: [],
    },
    pending_permission_requests: [],
    pending_user_input_requests: [],
    usage: {
      current_tokens: 0,
    },
  }
}

function sampleState(overrides?: Partial<ChatEventState>): ChatEventState {
  return {
    messages: [],
    timelineEvents: [],
    sessionState: sampleSessionState(),
    selectedSessionId: 7,
    ...overrides,
  }
}

function messagePartUpdatedEvent(): SessionEventRecord {
  return {
    session_id: 7,
    seq: 10,
    event_type: 'message_part_updated',
    created_at: '2026-05-08T00:00:00Z',
    payload: {
      message_id: 11,
      message_role: 'assistant',
      message_state: 'complete',
      message_created_at: '2026-05-08T00:00:00Z',
      part: {
        id: 21,
        message_id: 11,
        part_index: 0,
        status: 'complete',
        kind: 'text',
        created_at: '2026-05-08T00:00:00Z',
        content: {
          type: 'text',
          text: 'hello',
        },
      },
    },
  }
}

describe('chatPageModel', () => {
  test('sortMessages orders by timestamp then id', () => {
    const messages = sortMessages([
      {
        id: 3,
        session_id: 1,
        role: 'user',
        state: 'complete',
        created_at: '2026-05-08T00:00:02Z',
        updated_at: '2026-05-08T00:00:02Z',
        metadata: {},
        part_count: 0,
      },
      {
        id: 2,
        session_id: 1,
        role: 'user',
        state: 'complete',
        created_at: '2026-05-08T00:00:01Z',
        updated_at: '2026-05-08T00:00:01Z',
        metadata: {},
        part_count: 0,
      },
    ] satisfies MessageResource[])

    expect(messages.map((message) => message.id)).toEqual([2, 3])
  })

  test('sortMessageParts orders by part_index then id', () => {
    expect(
      sortMessageParts([
        { id: 4, message_id: 1, part_index: 2, status: 'complete', kind: 'text', created_at: 'a' },
        { id: 3, message_id: 1, part_index: 1, status: 'complete', kind: 'text', created_at: 'a' },
      ]).map((part) => part.id),
    ).toEqual([3, 4])
  })

  test('appendTimelineEvent deduplicates by seq_global', () => {
    const event = messagePartUpdatedEvent()
    const once = appendTimelineEvent([], event)
    const twice = appendTimelineEvent(once, event)
    expect(once.length).toBe(1)
    expect(twice.length).toBe(1)
  })

  test('applySessionEvent adds message on message_part_updated', () => {
    const result = applySessionEvent(sampleState(), messagePartUpdatedEvent())
    expect(result.state.messages.length).toBe(1)
    expect(result.state.messages[0]?.id).toBe(11)
    expect(result.state.timelineEvents.length).toBe(1)
    expect(result.shouldRefresh).toBe(true)
  })

  test('applySessionEvent appends text delta into existing part', () => {
    const base = sampleState({
      messages: [
        {
          id: 11,
          session_id: 7,
          role: 'assistant',
          state: 'streaming',
          created_at: '2026-05-08T00:00:00Z',
          updated_at: '2026-05-08T00:00:00Z',
          metadata: {},
          part_count: 1,
          parts: [
            {
              id: 21,
              message_id: 11,
              part_index: 0,
              status: 'streaming',
              kind: 'text',
              created_at: '2026-05-08T00:00:00Z',
              content: { type: 'text', text: 'hel' },
            },
          ],
        },
      ],
    })

    const result = applySessionEvent(base, {
      session_id: 7,
      seq: 12,
      event_type: 'message_part_delta',
      created_at: '2026-05-08T00:00:01Z',
      payload: {
        message_id: 11,
        part_id: 21,
        field: 'text',
        delta: 'lo',
      },
    })

    expect(result.state.messages[0]?.parts?.[0]?.content).toEqual({ type: 'text', text: 'hello' })
    expect(result.shouldRefresh).toBe(false)
  })

  test('applySessionEvent updates session state for run_started', () => {
    const result = applySessionEvent(sampleState(), {
      session_id: 7,
      seq: 9,
      event_type: 'run_started',
      created_at: '2026-05-08T00:00:00Z',
      payload: {},
    })

    expect(result.state.sessionState?.run_state).toBe('awaiting_model')
    expect(result.state.sessionState?.blocked).toBe(false)
    expect(result.shouldRefresh).toBe(false)
  })
})
