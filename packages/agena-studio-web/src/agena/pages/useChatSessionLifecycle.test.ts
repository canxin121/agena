import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type {
  MessageResource,
  ProviderModel,
  ProviderSummary,
  RewindCheckpointResource,
  RuntimeStatus,
  SessionExecutionResource,
  SessionResource,
  SessionTreeResource,
  TimelineEventRecord,
  WorkspaceResource,
} from '@/agena/lib/agenaApi'

import {
  useChatSessionLifecycle,
  type ChatSessionLifecycleDeps,
  type ChatSessionLifecycleInput,
} from './useChatSessionLifecycle'

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
    session: session(3, { parent_id: 1 }),
    blocked: false,
    run_state: 'idle',
    latest_event_seq: 4,
    execution: { allowed_tools: [] },
    pending_permission_requests: [],
    pending_user_input_requests: [],
    ...overrides,
  }
}

function runtimeStatus(): RuntimeStatus {
  return {
    generation: 1,
    loaded_at: '2026-05-10T00:00:00Z',
    workspace_root: '/repo',
    config_path: '/repo/.agena/config.json',
    config_found: true,
    auth_store_path: '/repo/.agena/auth.json',
    provider_ids: ['anthropic'],
    plugin_count: 0,
    session_runtime_available: true,
    watch_paths: [],
    reload: { enabled: false, interval_secs: 0 },
    janitor: { enabled: false, interval_secs: 0 },
    automation: {
      enabled: false,
      job_count: 0,
      recent_jobs: [],
    },
    operator: {
      mcp: { server_count: 0, tool_count: 0, servers: [] },
      lsp: { server_count: 0, diagnostics_count: 0, files_with_diagnostics: 0, servers: [] },
      agents: {
        default_agent: 'build',
        total_count: 8,
        primary_count: 7,
        subagent_count: 6,
        hidden_count: 0,
        agents: [],
      },
      skills: { skill_count: 0, command_count: 0, skills: [], commands: [] },
    },
  }
}

function createInput(query: Record<string, unknown> = {}): ChatSessionLifecycleInput {
  return {
    composer: ref(''),
    errorMessage: ref(''),
    loading: ref(false),
    localCommandNotice: ref(''),
    messages: ref<MessageResource[]>([]),
    providerModels: {},
    providers: ref<ProviderSummary[]>([]),
    rewindCheckpoints: ref<RewindCheckpointResource[]>([]),
    route: { query } as ChatSessionLifecycleInput['route'],
    runtime: ref<RuntimeStatus | null>(null),
    selectedModelId: ref(''),
    selectedProviderId: ref(''),
    selectedSessionId: ref<number | null>(null),
    selectedWorkspaceId: ref<number | null>(null),
    sessionSearch: ref(''),
    sessionState: ref<SessionExecutionResource | null>(null),
    sessions: ref<SessionResource[]>([]),
    sessionTree: ref<SessionTreeResource[]>([]),
    timelineEvents: ref<TimelineEventRecord[]>([]),
    workspaces: ref<WorkspaceResource[]>([]),
  }
}

function createDeps() {
  const calls: string[] = []
  const sessionsByWorkspace = new Map<number, SessionResource[]>([
    [1, [session(3, { workspace_id: 1, parent_id: 1 }), session(4, { workspace_id: 1, parent_id: 1 })]],
    [2, [session(8, { workspace_id: 2 })]],
  ])

  const deps: ChatSessionLifecycleDeps = {
    applySessionEvent: (state) => ({ state, shouldRefresh: false }),
    fetchRuntimeStatus: async () => {
      calls.push('fetchRuntimeStatus')
      return runtimeStatus()
    },
    getSessionState: async (sessionId) => {
      calls.push(`getSessionState:${sessionId}`)
      return sessionState({
        session: session(sessionId, { workspace_id: sessionId === 8 ? 2 : 1, parent_id: sessionId === 8 ? null : 1 }),
      })
    },
    getSessionTree: async (rootId) => {
      calls.push(`getSessionTree:${rootId}`)
      return [session(rootId), session(3, { parent_id: rootId }), session(4, { parent_id: rootId })]
    },
    listMessages: async (sessionId) => {
      calls.push(`listMessages:${sessionId}`)
      return [
        {
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
        },
      ]
    },
    listProviderModels: async (providerId) => {
      calls.push(`listProviderModels:${providerId}`)
      return [{ provider_id: providerId, id: 'claude-opus-4-7' }] satisfies ProviderModel[]
    },
    listProviders: async () => {
      calls.push('listProviders')
      return [
        { provider_id: 'anthropic', default_model: 'claude-opus-4-7', default_model_ref: 'anthropic/claude-opus-4-7' },
      ]
    },
    listRewindCheckpoints: async (sessionId) => {
      calls.push(`listRewindCheckpoints:${sessionId}`)
      return [{ schema: 1, at_ms: 1715299200000, target_message_id: 21, dropped: [] }]
    },
    listSessionTimeline: async (sessionId) => {
      calls.push(`listSessionTimeline:${sessionId}`)
      return [
        {
          session_id: sessionId,
          seq_global: 1,
          kind: 'run_started',
          created_at: '2026-05-10T00:00:00Z',
          payload: {},
        },
      ]
    },
    listSessions: async (workspaceId, options) => {
      calls.push(`listSessions:${workspaceId}:${options?.search || ''}`)
      return sessionsByWorkspace.get(workspaceId) || []
    },
    listWorkspaces: async () => {
      calls.push('listWorkspaces')
      return [
        { id: 1, path: '/repo-a', created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' },
        { id: 2, path: '/repo-b', created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' },
      ]
    },
    streamSessionEvents: (_sessionId) => {
      calls.push('streamSessionEvents')
      return {
        close() {
          calls.push('closeStream')
        },
      }
    },
  }

  return { deps, calls }
}

describe('useChatSessionLifecycle', () => {
  test('loadSidebar seeds runtime, providers, route slash, and route session selection', async () => {
    const input = createInput({ session: '3', slash: '/continue' })
    const { deps, calls } = createDeps()

    const lifecycle = useChatSessionLifecycle(input, deps, { registerComponentLifecycle: false })
    await lifecycle.loadSidebar()

    expect(calls).toEqual([
      'fetchRuntimeStatus',
      'listProviders',
      'listWorkspaces',
      'listProviderModels:anthropic',
      'listSessions:1:',
      'getSessionState:3',
      'listMessages:3',
      'listSessionTimeline:3',
      'getSessionTree:1',
      'listRewindCheckpoints:3',
      'streamSessionEvents',
    ])
    expect(input.runtime.value?.provider_ids).toEqual(['anthropic'])
    expect(input.providers.value.map((item) => item.provider_id)).toEqual(['anthropic'])
    expect(input.providerModels.anthropic?.map((item) => item.id)).toEqual(['claude-opus-4-7'])
    expect(input.selectedProviderId.value).toBe('anthropic')
    expect(input.selectedModelId.value).toBe('claude-opus-4-7')
    expect(input.composer.value).toBe('/continue')
    expect(input.localCommandNotice.value).toBe('Prepared /continue from runtime inspector.')
    expect(input.selectedWorkspaceId.value).toBe(1)
    expect(input.selectedSessionId.value).toBe(3)
    expect(input.messages.value.map((item) => item.id)).toEqual([21])
    expect(input.timelineEvents.value.map((item) => item.seq_global)).toEqual([1])
    expect(input.sessionTree.value.map((item) => item.id)).toEqual([1, 3, 4])
    expect(input.rewindCheckpoints.value.map((item) => item.target_message_id)).toEqual([21])
  })

  test('loadSessionsForWorkspace clears conversation state when workspace has no sessions', async () => {
    const input = createInput({})
    input.selectedSessionId.value = 99
    input.messages.value = [
      {
        id: 1,
        session_id: 99,
        role: 'assistant',
        state: 'complete',
        created_at: '2026-05-10T00:00:00Z',
        updated_at: '2026-05-10T00:00:00Z',
        metadata: {},
        usage: null,
        finish: null,
        part_count: 0,
        parts: [],
      },
    ]
    input.timelineEvents.value = [
      { session_id: 99, seq_global: 1, kind: 'run_started', created_at: '2026-05-10T00:00:00Z', payload: {} },
    ]
    input.sessionState.value = sessionState({ session: session(99) })
    const { deps, calls } = createDeps()

    deps.listSessions = async (workspaceId, options) => {
      calls.push(`listSessions:${workspaceId}:${options?.search || ''}`)
      return []
    }

    const lifecycle = useChatSessionLifecycle(input, deps, { registerComponentLifecycle: false })
    await lifecycle.loadSessionsForWorkspace(2, false)

    expect(input.selectedWorkspaceId.value).toBe(2)
    expect(input.selectedSessionId.value === null).toBe(true)
    expect(input.messages.value).toEqual([])
    expect(input.timelineEvents.value).toEqual([])
    expect(input.sessionState.value === null).toBe(true)
  })

  test('openSessionById searches other workspaces when session is not in the current list', async () => {
    const input = createInput({})
    input.workspaces.value = [
      { id: 1, path: '/repo-a', created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' },
      { id: 2, path: '/repo-b', created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' },
    ]
    input.sessions.value = [session(3, { workspace_id: 1 })]
    const { deps } = createDeps()

    const lifecycle = useChatSessionLifecycle(input, deps, { registerComponentLifecycle: false })
    const matched = await lifecycle.openSessionById(8)

    expect(matched).toBe(true)
    expect(input.selectedWorkspaceId.value).toBe(2)
    expect(input.selectedSessionId.value).toBe(8)
    expect(input.sessionState.value?.session.id).toBe(8)
  })

  test('refreshConversation loads state, messages, timeline, tree, and checkpoints', async () => {
    const input = createInput({})
    input.selectedSessionId.value = 3
    const { deps, calls } = createDeps()

    const lifecycle = useChatSessionLifecycle(input, deps, { registerComponentLifecycle: false })
    await lifecycle.refreshConversation(true)

    expect(calls).toEqual([
      'getSessionState:3',
      'listMessages:3',
      'listSessionTimeline:3',
      'getSessionTree:1',
      'listRewindCheckpoints:3',
      'streamSessionEvents',
    ])
    expect(input.loading.value).toBe(false)
    expect(input.sessionState.value?.session.id).toBe(3)
    expect(input.messages.value.map((item) => item.id)).toEqual([21])
    expect(input.timelineEvents.value.map((item) => item.seq_global)).toEqual([1])
    expect(input.sessionTree.value.map((item) => item.id)).toEqual([1, 3, 4])
    expect(input.rewindCheckpoints.value.map((item) => item.target_message_id)).toEqual([21])
  })
})
