import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import type { ChatUsageSummary } from './chatUsageModel'
import { renderVueSsr } from './test/renderVueSsr'
import { useChatSidebarState } from './useChatSidebarState'

function createChatPageContentState() {
  const loading = ref(false)
  const workspacePath = ref('/repo')
  const workspaces = ref([
    {
      id: 7,
      path: '/repo',
      created_at: '2026-05-11T00:00:00Z',
      updated_at: '2026-05-11T00:00:00Z',
      session_count: 1,
    },
  ])
  const selectedWorkspaceId = ref<number | null>(7)
  const selectedWorkspace = computed(() => workspaces.value[0] || null)
  const sessionSearch = ref('')
  const newSessionTitle = ref('New session')
  const sessions = ref([
    {
      id: 11,
      workspace_id: 7,
      title: 'Review bug',
      version: 1,
      created_at: '2026-05-11T00:00:00Z',
      updated_at: '2026-05-11T00:00:00Z',
      message_count: 2,
      child_session_count: 0,
    },
  ])
  const selectedSessionId = ref<number | null>(11)
  const selectedSession = computed(() => sessions.value[0] || null)

  const sidebar = useChatSidebarState({
    createSessionAction: () => {},
    loadSessionsForWorkspace: () => {},
    loading,
    newSessionTitle,
    resolveWorkspaceAction: () => {},
    selectedSession,
    selectedSessionId,
    selectedWorkspace,
    selectedWorkspaceId,
    selectSession: () => {},
    selectWorkspace: () => {},
    sessionSearch,
    sessions,
    workspacePath,
    workspaces,
  })

  const message = {
    id: 101,
    session_id: 11,
    role: 'assistant' as const,
    state: 'done',
    created_at: '2026-05-11T01:00:00Z',
    updated_at: '2026-05-11T01:00:00Z',
    metadata: {
      model_provider_id: 'openai',
      model_id: 'gpt-5',
    },
    usage: {
      input_tokens: 120,
      output_tokens: 40,
      total_cost: 0.12,
    },
    finish: 'stop',
    part_count: 1,
    parts: [],
  }

  const inspectedPart = {
    id: 301,
    message_id: 101,
    part_index: 0,
    status: 'done',
    kind: 'tool_call',
    summary: 'inspect repo',
    has_detail: true,
    operation_id: 'op-1',
    created_at: '2026-05-11T01:00:00Z',
    content: { command: 'git status' },
  }

  const sessionUsageSummary: ChatUsageSummary = {
    turns: 1,
    inputTokens: 120,
    outputTokens: 40,
    reasoningTokens: 0,
    cacheWriteTokens: 0,
    cacheReadTokens: 0,
    totalCostUsd: 0.12,
    byModel: [],
  }

  return {
    sidebar,
    formatMessageTime: (value: string) => value,
    formatEventTime: (value: number) => String(value),
    selectedWorkspace,
    selectedSession,
    loading,
    continuing: ref(false),
    sessionState: ref({
      session: sessions.value[0],
      blocked: true,
      run_state: 'awaiting_model',
      latest_event_seq: 3,
      execution: {
        agent_profile: 'default',
        active_skill_name: 'review',
        system_prompt_override: 'be strict',
        allowed_tools: ['bash'],
        model_provider_id: 'openai',
        model_id: 'gpt-5',
        effective_workspace_root: '/repo',
        task_id: 'task-1',
      },
      automation: null,
      pending_permission_requests: [
        {
          request_id: 'perm-1',
          action: { kind: 'builtin_tool', tool_name: 'bash', qualifier: 'git status *' },
          reason: 'Need to inspect git status',
          explanation: 'Needed to verify workspace changes',
          source: 'permission_reply',
          scope: 'workspace' as const,
          operator: 'assistant',
          created_at: '2026-05-11T01:01:00Z',
        },
      ],
      pending_user_input_requests: [
        {
          request_id: 'input-1',
          questions: [
            {
              id: 'branch',
              header: 'Branch',
              question: 'Which branch should be used?',
            },
          ],
          created_at: '2026-05-11T01:02:00Z',
        },
      ],
    }),
    sessionLineageLabel: ref('root #11'),
    ancestorSessions: ref([]),
    executionFacts: ref(['run_state=awaiting_model', 'blocked=true']),
    sessionUsageSummaryFacts: ref(['turns 1', 'cost $0.1200']),
    siblingSessions: ref([]),
    childSessions: ref([]),
    parentSession: ref(null),
    renameCurrentSession: () => {},
    forkCurrentSession: () => {},
    deleteCurrentSession: () => {},
    exportCurrentSession: () => {},
    continueCurrentSession: () => {},
    cancelCurrentSessionTurn: () => {},
    sessionTreeRows: ref([
      {
        session: {
          id: 11,
          workspace_id: 7,
          title: 'Review bug',
          version: 1,
          created_at: '2026-05-11T00:00:00Z',
          updated_at: '2026-05-11T00:00:00Z',
          message_count: 2,
          child_session_count: 0,
        },
        depth: 0,
      },
    ]),
    loadSessionTree: () => {},
    rewindCheckpointFacts: ref([
      {
        key: 'rewind-1',
        label: 'checkpoint',
        summary: 'Before patch',
        messageId: 88,
      },
    ]),
    loadRewindCheckpoints: () => {},
    unrewindToMessage: () => {},
    sessionUsageHeadline: ref('1 assistant turn'),
    sessionUsageSummary: ref(sessionUsageSummary),
    sessionUsageModelLines: ref([
      {
        key: 'openai:gpt-5',
        label: 'openai / gpt-5',
        facts: ['turns 1', 'cost $0.1200'],
      },
    ]),
    formatUsageCount: (value: number) => String(value),
    formatUsageUsd: (value: number) => `$${value.toFixed(4)}`,
    copySessionUsageSummary: () => {},
    selectedProviderId: ref('openai'),
    selectedModelId: ref('gpt-5'),
    providers: ref([
      {
        provider_id: 'openai',
        default_model: 'gpt-5',
        default_model_ref: 'openai/gpt-5',
      },
    ]),
    providerDefaultModel: () => 'gpt-5',
    providerModelOptions: () => [
      {
        provider_id: 'openai',
        id: 'gpt-5',
        display_name: 'GPT-5',
      },
    ],
    providerModelLabel: (model: { display_name?: string; id: string }) => model.display_name || model.id,
    sessionImportJsonl: ref('{"schema":1}'),
    importSessionFromJsonl: () => {},
    messages: ref([message]),
    inspectedMessage: ref(message),
    inspectedMessageParts: ref([inspectedPart]),
    inspectedPart: ref(inspectedPart),
    refreshConversation: () => {},
    inspectMessage: () => {},
    rewindToMessage: () => {},
    messageTags: () => ['inspect-message', 'inspected-message'],
    messageUsageFacts: () => ['in 120', 'out 40'],
    messageBlocks: () => [{ kind: 'text' as const, body: 'Patch summary' }],
    timelineEvents: ref([
      {
        seq_global: 1,
        session_id: 11,
        created_at: '2026-05-11T01:03:00Z',
        kind: 'tool',
        payload: { summary: 'Ran git status', message_id: 101 },
      },
    ]),
    readPayloadMessageId: (payload: Record<string, unknown>) =>
      typeof payload.message_id === 'number' ? payload.message_id : null,
    scrollToMessage: () => {},
    permissionActionView: () => ({ title: 'bash · git status *', details: ['kind=builtin_tool'] }),
    permissionRiskLabel: () => 'mutable tool execution',
    permissionExplainability: () => ({
      summary: 'Matched a remembered permission reply · scope=workspace · operator=assistant',
      details: ['source=permission_reply', 'scope=workspace', 'operator=assistant'],
    }),
    permissionReplyPreview: (scope?: 'session' | 'workspace' | 'global') =>
      scope ? `remembered:${scope}` : 'once',
    approvePermission: () => {},
    readUserAnswer: () => 'main',
    updateUserAnswer: () => {},
    submitUserAnswers: () => {},
    cancelUserAnswers: () => {},
    composer: ref('/runtime review'),
    slashSuggestions: ref([
      {
        id: 'runtime',
        title: 'Open Runtime',
        description: 'Jump to runtime',
        category: 'Navigation',
        source: 'navigation' as const,
        slash: '/runtime',
      },
    ]),
    sending: ref(false),
    openGlobalCommandPalette: () => {},
    sendPrompt: () => {},
  }
}

describe('ChatPageContent', () => {
  test('renders the assembled chat workspace panels from reactive state', async () => {
    const html = await renderVueSsr('/src/agena/pages/ChatPageContent.vue', {
      state: createChatPageContentState(),
    })

    expect(html.includes('Workspace')).toBe(true)
    expect(html.includes('Active Session')).toBe(true)
    expect(html.includes('Messages')).toBe(true)
    expect(html.includes('Message Inspector')).toBe(true)
    expect(html.includes('Timeline')).toBe(true)
    expect(html.includes('Pending Permissions')).toBe(true)
    expect(html.includes('Pending User Input')).toBe(true)
    expect(html.includes('Composer')).toBe(true)
    expect(html.includes('inspect-message')).toBe(true)
    expect(html.includes('inspected-message')).toBe(true)
    expect(html.includes('Review bug')).toBe(true)
    expect(html.includes('/runtime review')).toBe(true)
    expect(html.includes('Run Options')).toBe(true)
    expect(html.includes('Usage')).toBe(true)
    expect(html.includes('Session Transfer')).toBe(true)
    expect(html.includes('Session Tree')).toBe(true)
    expect(html.includes('Rewind Checkpoints')).toBe(true)
  })
})
