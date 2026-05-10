import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { MessagePart, MessageResource, RewindCheckpointResource, SessionExecutionResource, SessionResource, SessionTreeResource, WorkspaceResource } from '@/agena/lib/agenaApi'

import { useChatDerivedState } from './useChatDerivedState'

function messagePart(id: number, overrides?: Partial<MessagePart>): MessagePart {
  return {
    id,
    message_id: 1,
    part_index: 0,
    status: 'complete',
    kind: 'output',
    created_at: '2026-05-10T00:00:00Z',
    ...overrides,
  }
}

function message(id: number, overrides?: Partial<MessageResource>): MessageResource {
  return {
    id,
    session_id: 3,
    role: 'assistant',
    state: 'complete',
    created_at: '2026-05-10T00:00:00Z',
    updated_at: '2026-05-10T00:00:00Z',
    metadata: { model_provider_id: 'anthropic', model_id: 'claude-opus-4-7' },
    usage: {
      input_tokens: 10,
      output_tokens: 5,
      total_cost: 0.0123,
    },
    finish: null,
    part_count: 1,
    parts: [messagePart(id, { content: { type: 'text', text: 'hello' } })],
    ...overrides,
  }
}

function session(id: number, overrides?: Partial<SessionResource>): SessionResource {
  return {
    id,
    workspace_id: 1,
    title: `session-${id}`,
    version: 1,
    created_at: '2026-05-10T00:00:00Z',
    updated_at: '2026-05-10T00:00:00Z',
    message_count: 0,
    child_session_count: 0,
    ...overrides,
  }
}

function sessionState(overrides?: Partial<SessionExecutionResource>): SessionExecutionResource {
  return {
    session: session(3, { parent_id: 2 }),
    blocked: false,
    run_state: 'idle',
    latest_event_seq: 1,
    execution: {
      agent_profile: 'opus',
      active_skill_name: 'review',
      task_id: 'task-7',
      model_provider_id: 'anthropic',
      model_id: 'claude-opus-4-7',
      effective_workspace_root: '/repo',
      allowed_tools: ['Read', 'Edit'],
    },
    pending_permission_requests: [],
    pending_user_input_requests: [],
    ...overrides,
  }
}

describe('useChatDerivedState', () => {
  test('computes selected entities, usage, tree rows, rewind facts, and lineage', () => {
    const sessions = ref<SessionResource[]>([
      session(1),
      session(2, { parent_id: 1 }),
      session(3, { parent_id: 2 }),
      session(4, { parent_id: 2 }),
      session(5, { parent_id: 3 }),
    ])
    const workspaces = ref<WorkspaceResource[]>([
      { id: 1, path: '/repo', created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' },
    ])
    const sessionTree = ref<SessionTreeResource[]>([
      session(1),
      session(2, { parent_id: 1 }),
      session(3, { parent_id: 2 }),
      session(4, { parent_id: 2 }),
    ])
    const rewindCheckpoints = ref<RewindCheckpointResource[]>([
      {
        schema: 1,
        at_ms: 1715299200000,
        target_message_id: 17,
        dropped: [{ message_id: 9, role: 'assistant', preview: 'old' }],
      },
    ])
    const derived = useChatDerivedState({
      formatEventTime: (timestampMs) => `t=${timestampMs}`,
      messages: ref<MessageResource[]>([message(11), message(12, { usage: { input_tokens: 4, output_tokens: 6, total_cost: 0.0101 } })]),
      rewindCheckpoints,
      selectedSessionId: ref<number | null>(3),
      selectedWorkspaceId: ref<number | null>(1),
      sessionState: ref<SessionExecutionResource | null>(sessionState()),
      sessionTree,
      sessions,
      workspaces,
    })

    expect(derived.selectedWorkspace.value?.path).toBe('/repo')
    expect(derived.selectedSession.value?.id).toBe(3)
    expect(derived.sessionUsageSummary.value.turns).toBe(2)
    expect(derived.sessionUsageSummaryFacts.value).toEqual(['turns 2', 'in 14', 'out 11', 'cost $0.0224'])
    expect(derived.sessionUsageHeadline.value).toBe('2 turns · 25 visible tokens · $0.0224')
    expect(derived.sessionUsageModelLines.value).toEqual([
      {
        key: 'anthropic/claude-opus-4-7',
        label: 'anthropic/claude-opus-4-7',
        facts: ['turns 2', 'in 14', 'out 11', 'cost $0.0224'],
      },
    ])
    expect(derived.sessionTreeRows.value.map((item) => `${item.depth}:${item.session.id}`)).toEqual(['0:1', '1:2', '2:3', '2:4'])
    expect(derived.rewindCheckpointFacts.value).toEqual([
      {
        key: '17-1715299200000',
        label: 'message #17',
        summary: 't=1715299200000 · dropped 1 message(s)',
      },
    ])
    expect(derived.ancestorSessions.value.map((item) => item.id)).toEqual([1, 2])
    expect(derived.parentSession.value?.id).toBe(2)
    expect(derived.siblingSessions.value.map((item) => item.id)).toEqual([4])
    expect(derived.childSessions.value.map((item) => item.id)).toEqual([5])
    expect(derived.sessionLineageLabel.value).toBe('root=#1 · parent=#2 · siblings=1 · children=1')
    expect(derived.executionFacts.value).toEqual([
      'agent=opus',
      'skill=review',
      'task=task-7',
      'model=anthropic/claude-opus-4-7',
      'workspace=/repo',
      'allowed_tools=2',
    ])
  })

  test('returns empty summaries when there is no usage or execution', () => {
    const derived = useChatDerivedState({
      formatEventTime: (timestampMs) => String(timestampMs),
      messages: ref<MessageResource[]>([]),
      rewindCheckpoints: ref<RewindCheckpointResource[]>([]),
      selectedSessionId: ref<number | null>(null),
      selectedWorkspaceId: ref<number | null>(null),
      sessionState: ref<SessionExecutionResource | null>(null),
      sessionTree: ref<SessionTreeResource[]>([]),
      sessions: ref<SessionResource[]>([]),
      workspaces: ref<WorkspaceResource[]>([]),
    })

    expect(derived.selectedWorkspace.value).toBe(null)
    expect(derived.selectedSession.value).toBe(null)
    expect(derived.sessionUsageSummaryFacts.value).toEqual([])
    expect(derived.sessionUsageHeadline.value).toBe('No assistant usage yet.')
    expect(derived.sessionUsageModelLines.value).toEqual([])
    expect(derived.rewindCheckpointFacts.value).toEqual([])
    expect(derived.ancestorSessions.value).toEqual([])
    expect(derived.childSessions.value).toEqual([])
    expect(derived.siblingSessions.value).toEqual([])
    expect(derived.sessionLineageLabel.value).toBe('')
    expect(derived.executionFacts.value).toEqual([])
  })
})
