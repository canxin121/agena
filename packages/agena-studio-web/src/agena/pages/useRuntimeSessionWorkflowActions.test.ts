import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { GlobalEventRecord, SessionExecutionResource, SessionResource, TimelineEventRecord } from '../lib/agenaApi'
import { useRuntimeSessionWorkflowActions } from './useRuntimeSessionWorkflowActions'

function createExecution(sessionId: number): SessionExecutionResource {
  return {
    session: {
      id: sessionId,
      workspace_id: 3,
      title: `Session ${sessionId}`,
      version: 1,
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      message_count: 1,
      child_session_count: 0,
    },
    blocked: false,
    run_state: 'idle',
    execution: {
      allowed_tools: [],
    },
    pending_permission_requests: [],
    pending_user_input_requests: [],
  }
}

function createTimeline(sessionId: number): TimelineEventRecord[] {
  return [{
    session_id: sessionId,
    seq_global: sessionId,
    kind: 'run.started',
    payload: {},
    created_at: '2026-05-10T00:00:00Z',
  }]
}

function createGlobalEvents(): GlobalEventRecord[] {
  return [{
    id: 'event-1',
    seq_global: 100,
    session_id: 10,
    workspace_id: 1,
    created_at: '2026-05-10T00:00:00Z',
    kind: 'turn_started',
    payload: {},
  }]
}

function createState() {
  const calls: string[] = []
  const state = {
    actionError: ref(''),
    actionMessage: ref(''),
    load: async () => {
      calls.push('load')
    },
    selectedSessionId: ref<number | null>(10),
    selectedWorkspaceId: ref<number | null>(1),
    sessionExecution: ref<SessionExecutionResource | null>(null),
    globalEvents: ref<GlobalEventRecord[]>([]),
    sessionTimeline: ref<TimelineEventRecord[]>([]),
    sessions: ref<SessionResource[]>([]),
    workflowLoading: ref(false),
  }

  return { calls, state }
}

describe('useRuntimeSessionWorkflowActions', () => {
  test('loadSessionExecution hydrates execution and timeline', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeSessionWorkflowActions(state, {
      getSessionState: async (sessionId) => {
        calls.push(`getSessionState:${sessionId}`)
        return createExecution(sessionId)
      },
      listGlobalEvents: async () => {
        calls.push('listGlobalEvents')
        return createGlobalEvents()
      },
      listSessions: async () => [],
      listSessionTimeline: async (sessionId) => {
        calls.push(`listSessionTimeline:${sessionId}`)
        return createTimeline(sessionId)
      },
      pickSessionId: () => null,
      reloadRuntime: async () => ({
        cause: 'manual',
        previous_generation: 1,
        generation: 2,
        loaded_at: '2026-05-10T00:00:00Z',
      }),
    })

    await actions.loadSessionExecution(10)

    expect(calls).toEqual([
      'getSessionState:10',
      'listSessionTimeline:10',
      'listGlobalEvents',
    ])
    expect(state.sessionExecution.value?.session.id).toBe(10)
    expect(state.sessionTimeline.value.map((entry) => entry.kind)).toEqual(['run.started'])
    expect(state.globalEvents.value.map((entry) => entry.kind)).toEqual(['turn_started'])
    expect(state.workflowLoading.value).toBe(false)
  })

  test('selectWorkspace loads sessions and follows picked session', async () => {
    const { calls, state } = createState()
    const nextSessions: SessionResource[] = [{
      id: 22,
      workspace_id: 7,
      title: 'Session 22',
      version: 1,
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      message_count: 1,
      child_session_count: 0,
    }]
    const actions = useRuntimeSessionWorkflowActions(state, {
      getSessionState: async (sessionId) => {
        calls.push(`getSessionState:${sessionId}`)
        return createExecution(sessionId)
      },
      listGlobalEvents: async () => {
        calls.push('listGlobalEvents')
        return createGlobalEvents()
      },
      listSessions: async (workspaceId) => {
        calls.push(`listSessions:${workspaceId}`)
        return nextSessions
      },
      listSessionTimeline: async (sessionId) => {
        calls.push(`listSessionTimeline:${sessionId}`)
        return createTimeline(sessionId)
      },
      pickSessionId: (selectedSessionId, sessions) => {
        calls.push(`pickSessionId:${selectedSessionId}:${sessions.length}`)
        return 22
      },
      reloadRuntime: async () => ({
        cause: 'manual',
        previous_generation: 1,
        generation: 2,
        loaded_at: '2026-05-10T00:00:00Z',
      }),
    })

    await actions.selectWorkspace(7)

    expect(calls).toEqual([
      'listSessions:7',
      'pickSessionId:10:1',
      'getSessionState:22',
      'listSessionTimeline:22',
      'listGlobalEvents',
    ])
    expect(state.selectedWorkspaceId.value).toBe(7)
    expect(state.selectedSessionId.value).toBe(22)
    expect(state.sessions.value.map((session) => session.id)).toEqual([22])
    expect(state.sessionExecution.value?.session.id).toBe(22)
  })

  test('selectWorkspace clears execution when no session remains', async () => {
    const { calls, state } = createState()
    state.sessionExecution.value = createExecution(10)
    state.sessionTimeline.value = createTimeline(10)
    const actions = useRuntimeSessionWorkflowActions(state, {
      getSessionState: async () => createExecution(0),
      listGlobalEvents: async () => createGlobalEvents(),
      listSessions: async (workspaceId) => {
        calls.push(`listSessions:${workspaceId}`)
        return []
      },
      listSessionTimeline: async () => [],
      pickSessionId: () => null,
      reloadRuntime: async () => ({
        cause: 'manual',
        previous_generation: 1,
        generation: 2,
        loaded_at: '2026-05-10T00:00:00Z',
      }),
    })

    await actions.selectWorkspace(8)

    expect(calls).toEqual(['listSessions:8'])
    expect(state.selectedSessionId.value).toBe(null)
    expect(state.sessionExecution.value === null).toBe(true)
    expect(state.globalEvents.value).toEqual([])
    expect(state.sessionTimeline.value).toEqual([])
  })

  test('selectSession and triggerReload refresh expected state', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeSessionWorkflowActions(state, {
      getSessionState: async (sessionId) => {
        calls.push(`getSessionState:${sessionId}`)
        return createExecution(sessionId)
      },
      listGlobalEvents: async () => {
        calls.push('listGlobalEvents')
        return createGlobalEvents()
      },
      listSessions: async () => [],
      listSessionTimeline: async (sessionId) => {
        calls.push(`listSessionTimeline:${sessionId}`)
        return createTimeline(sessionId)
      },
      pickSessionId: () => null,
      reloadRuntime: async () => {
        calls.push('reloadRuntime')
        return {
          cause: 'manual',
          previous_generation: 1,
          generation: 5,
          loaded_at: '2026-05-10T00:00:00Z',
        }
      },
    })

    await actions.selectSession(33)
    await actions.triggerReload()

    expect(calls).toEqual([
      'getSessionState:33',
      'listSessionTimeline:33',
      'listGlobalEvents',
      'reloadRuntime',
      'load',
    ])
    expect(state.selectedSessionId.value).toBe(33)
    expect(state.actionMessage.value).toBe('Runtime reloaded to generation 5.')
  })
})
