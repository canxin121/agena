import { describe, expect, test } from 'bun:test'

import type { DomainEventRecord, MessageResource, SessionExecutionResource } from '../lib/agenaApi'
import { applySessionEvent } from './chatPageModel'

function baseState(input?: {
  messages?: MessageResource[]
  sessionState?: SessionExecutionResource | null
  timelineEvents?: DomainEventRecord[]
}) {
  return {
    messages: input?.messages ?? [],
    timelineEvents: input?.timelineEvents ?? [],
    sessionState: input?.sessionState ?? null,
    selectedSessionId: 11,
  }
}

function sessionState(runState: SessionExecutionResource['run_state']): SessionExecutionResource {
  return {
    session: {
      id: 11,
      workspace_id: 3,
      title: 'Chat',
      version: 1,
      created_at: '2026-05-23T00:00:00.000Z',
      updated_at: '2026-05-23T00:00:00.000Z',
      message_count: 1,
      child_session_count: 0,
    },
    blocked: true,
    run_state: runState,
    latest_event_seq: 0,
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

describe('applySessionEvent', () => {
  test('message_part_updated creates a message shell from the shared event helper', () => {
    const event: DomainEventRecord = {
      seq_global: 17,
      session_id: 11,
      created_at: '2026-05-23T00:00:01.000Z',
      kind: 'message_part_updated',
      payload: {
        session_id: 11,
        message_id: 101,
        message_role: 'assistant',
        message_state: 'pending',
        message_created_at: '2026-05-23T00:00:01.000Z',
        ts_ms: 1_748_000_001_250,
        part: {
          id: 201,
          message_id: 101,
          part_index: 0,
          status: 'pending',
          kind: 'text',
          name: 'text',
          summary: 'hello',
          has_detail: true,
          created_at: '2026-05-23T00:00:01.000Z',
          content: {
            type: 'text',
            text: 'hello',
          },
        },
      },
    }

    const result = applySessionEvent(baseState(), event)

    expect(result.shouldRefresh).toBe(false)
    expect(result.state.timelineEvents).toEqual([event])
    expect(result.state.messages).toEqual([
      {
        id: 101,
        session_id: 11,
        role: 'assistant',
        state: 'pending',
        created_at: '2026-05-23T00:00:01.000Z',
        updated_at: new Date(1_748_000_001_250).toISOString(),
        metadata: {},
        usage: null,
        finish: null,
        part_count: 1,
        parts: [
          {
            id: 201,
            message_id: 101,
            part_index: 0,
            status: 'pending',
            kind: 'text',
            name: 'text',
            summary: 'hello',
            has_detail: true,
            created_at: '2026-05-23T00:00:01.000Z',
            content: {
              type: 'text',
              text: 'hello',
            },
          },
        ],
      },
    ])
  })

  test('assistant_message_completed reuses the same message shape and clears run state', () => {
    const existingMessage: MessageResource = {
      id: 101,
      session_id: 11,
      role: 'assistant',
      state: 'in_progress',
      created_at: '2026-05-23T00:00:01.000Z',
      updated_at: '2026-05-23T00:00:02.000Z',
      metadata: {},
      usage: null,
      finish: null,
      part_count: 1,
      parts: [
        {
          id: 201,
          message_id: 101,
          part_index: 0,
          status: 'in_progress',
          kind: 'text',
          name: 'text',
          summary: 'hello',
          has_detail: true,
          created_at: '2026-05-23T00:00:01.000Z',
          content: {
            type: 'text',
            text: 'hello',
          },
        },
      ],
    }
    const event: DomainEventRecord = {
      seq_global: 18,
      session_id: 11,
      created_at: '2026-05-23T00:00:03.000Z',
      kind: 'assistant_message_completed',
      payload: {
        message_id: 101,
        created_at: '2026-05-23T00:00:01.000Z',
        finish_reason: 'stop',
        metadata: { tags: ['final'] },
        usage: { input_tokens: 3, output_tokens: 5 },
        parts: [
          {
            id: 201,
            message_id: 101,
            part_index: 0,
            status: 'completed',
            kind: 'text',
            name: 'text',
            summary: 'hello world',
            has_detail: true,
            created_at: '2026-05-23T00:00:01.000Z',
            content: {
              type: 'text',
              text: 'hello world',
            },
          },
        ],
      },
    }

    const result = applySessionEvent(
      baseState({
        messages: [existingMessage],
        sessionState: sessionState('awaiting_model'),
      }),
      event,
    )

    expect(result.shouldRefresh).toBe(false)
    expect(result.state.sessionState?.run_state).toBe('idle')
    expect(result.state.sessionState?.blocked).toBe(false)
    expect(result.state.timelineEvents).toEqual([event])
    expect(result.state.messages[0]).toEqual({
      id: 101,
      session_id: 11,
      role: 'assistant',
      state: 'completed',
      created_at: '2026-05-23T00:00:01.000Z',
      updated_at: '2026-05-23T00:00:02.000Z',
      metadata: { tags: ['final'] },
      usage: { input_tokens: 3, output_tokens: 5 },
      finish: 'stop',
      part_count: 1,
      parts: [
        {
          id: 201,
          message_id: 101,
          part_index: 0,
          status: 'completed',
          kind: 'text',
          name: 'text',
          summary: 'hello world',
          has_detail: true,
          created_at: '2026-05-23T00:00:01.000Z',
          content: {
            type: 'text',
            text: 'hello world',
          },
        },
      ],
    })
  })

  test('user_message_appended inserts a completed user message without a refresh', () => {
    const event: DomainEventRecord = {
      seq_global: 19,
      session_id: 11,
      created_at: '2026-05-23T00:00:04.000Z',
      kind: 'user_message_appended',
      payload: {
        message_id: 102,
        created_at: '2026-05-23T00:00:04.000Z',
        metadata: {},
        parts: [
          {
            id: 301,
            message_id: 102,
            part_index: 0,
            status: 'completed',
            kind: 'text',
            name: 'text',
            summary: 'ping',
            has_detail: true,
            created_at: '2026-05-23T00:00:04.000Z',
            content: {
              type: 'text',
              text: 'ping',
            },
          },
        ],
      },
    }

    const result = applySessionEvent(baseState(), event)

    expect(result.shouldRefresh).toBe(false)
    expect(result.state.timelineEvents).toEqual([event])
    expect(result.state.messages[0]?.role).toBe('user')
    expect(result.state.messages[0]?.state).toBe('completed')
    expect(result.state.messages[0]?.parts?.[0]?.content).toEqual({
      type: 'text',
      text: 'ping',
    })
  })
})
