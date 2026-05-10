import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { MessageResource, SessionExecutionResource } from '@/agena/lib/agenaApi'

import { useChatSessionActions, type ChatSessionActionsDeps, type ChatSessionActionsInput } from './useChatSessionActions'

function sessionState(overrides?: Partial<SessionExecutionResource>): SessionExecutionResource {
  return {
    session: {
      id: 3,
      workspace_id: 1,
      title: 'session',
      version: 1,
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      message_count: 0,
      child_session_count: 0,
    },
    blocked: false,
    run_state: 'idle',
    latest_event_seq: 1,
    execution: { allowed_tools: [] },
    pending_permission_requests: [],
    pending_user_input_requests: [],
    ...overrides,
  }
}

function message(id: number): MessageResource {
  return {
    id,
    session_id: 3,
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

function createDeps(): ChatSessionActionsDeps & { calls: string[] } {
  const calls: string[] = []
  return {
    calls,
    cancelUserInput: async ({ sessionId, requestId }) => {
      calls.push(`cancelUserInput:${sessionId}:${requestId}`)
      return sessionState()
    },
    continueSession: async ({ sessionId, providerId, modelId }) => {
      calls.push(`continueSession:${sessionId}:${providerId || ''}:${modelId || ''}`)
      return sessionState({ run_state: 'awaiting_model' })
    },
    createSession: async ({ workspaceId, title, parentId }) => {
      calls.push(`createSession:${workspaceId}:${title}:${parentId || ''}`)
      return {
        id: 9,
        workspace_id: workspaceId,
        title,
        version: 1,
        created_at: '2026-05-10T00:00:00Z',
        updated_at: '2026-05-10T00:00:00Z',
        message_count: 0,
        child_session_count: 0,
        parent_id: parentId,
      }
    },
    createWorkspace: async (path) => {
      calls.push(`createWorkspace:${path}`)
      return { id: 7, path, created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' }
    },
    exportSessionJsonl: async (sessionId) => {
      calls.push(`exportSessionJsonl:${sessionId}`)
      return '{"session":3}'
    },
    forkSession: async ({ sessionId, atMessageId, title }) => {
      calls.push(`forkSession:${sessionId}:${atMessageId || ''}:${title}`)
      return sessionState({ session: { ...sessionState().session, id: 13, title: title || 'forked' } })
    },
    importSessionJsonl: async (jsonl) => {
      calls.push(`importSessionJsonl:${jsonl}`)
      return sessionState({ session: { ...sessionState().session, id: 15, workspace_id: 4 } })
    },
    replyPermission: async ({ sessionId, requestId, kind, scope }) => {
      calls.push(`replyPermission:${sessionId}:${requestId}:${kind}:${scope || ''}`)
      return sessionState()
    },
    replyUserInput: async ({ sessionId, requestId, answers }) => {
      calls.push(`replyUserInput:${sessionId}:${requestId}:${JSON.stringify(answers)}`)
      return sessionState()
    },
    resolveWorkspace: async (path, createIfMissing) => {
      calls.push(`resolveWorkspace:${path}:${createIfMissing}`)
      return { id: 8, path, created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' }
    },
    rewindSession: async ({ sessionId, messageId }) => {
      calls.push(`rewindSession:${sessionId}:${messageId}`)
      return sessionState()
    },
    submitTurn: async ({ sessionId, text, providerId, modelId }) => {
      calls.push(`submitTurn:${sessionId}:${text}:${providerId || ''}:${modelId || ''}`)
      return sessionState({ run_state: 'awaiting_model' })
    },
  }
}

function createInput() {
  const syncCalls: string[] = []
  const refreshCalls: boolean[] = []
  const loadSidebarCalls: string[] = []
  const loadSessionsCalls: Array<[number, boolean | undefined]> = []
  const selectSessionCalls: number[] = []
  const selectWorkspaceCalls: number[] = []

  const input: ChatSessionActionsInput = {
    composer: ref(''),
    continuing: ref(false),
    errorMessage: ref(''),
    loading: ref(false),
    localCommandNotice: ref(''),
    messages: ref<MessageResource[]>([message(21)]),
    newSessionTitle: ref(''),
    refreshConversation: async (foreground: boolean) => {
      refreshCalls.push(foreground)
    },
    runSlashCommand: async (_inputText: string) => ({ matched: false, command: undefined }),
    selectedModelId: ref('claude-opus-4-7'),
    selectedProviderId: ref('anthropic'),
    selectedSessionId: ref<number | null>(3),
    selectedWorkspaceId: ref<number | null>(1),
    sending: ref(false),
    sessionImportJsonl: ref(''),
    sessionState: ref<SessionExecutionResource | null>(sessionState()),
    syncEventStream: () => {
      syncCalls.push('sync')
    },
    userInputDrafts: { req1: { q1: 'alpha', q2: 'one, two' } },
    workspacePath: ref('/repo'),
    loadSidebar: async () => {
      loadSidebarCalls.push('loadSidebar')
    },
    loadSessionsForWorkspace: async (workspaceId: number, preserveSelection?: boolean) => {
      loadSessionsCalls.push([workspaceId, preserveSelection])
    },
    selectSession: async (sessionId: number) => {
      selectSessionCalls.push(sessionId)
    },
    selectWorkspace: async (workspaceId: number) => {
      selectWorkspaceCalls.push(workspaceId)
    },
  }

  return { input, refreshCalls, syncCalls, loadSidebarCalls, loadSessionsCalls, selectSessionCalls, selectWorkspaceCalls }
}

describe('useChatSessionActions', () => {
  test('sendPrompt executes local slash commands without calling submitTurn', async () => {
    const deps = createDeps()
    const { input, refreshCalls, syncCalls } = createInput()
    input.composer.value = '/cost'
    input.runSlashCommand = async () => ({ matched: true, command: { title: 'Show Session Cost', source: 'chat-action' } })

    const actions = useChatSessionActions(input, deps)
    await actions.sendPrompt()

    expect(input.composer.value).toBe('')
    expect(input.localCommandNotice.value).toBe('Executed Show Session Cost')
    expect(refreshCalls).toEqual([])
    expect(syncCalls).toEqual([])
    expect(deps.calls.some((item) => item.startsWith('submitTurn:'))).toBe(false)
  })

  test('sendPrompt submits turn and refreshes conversation', async () => {
    const deps = createDeps()
    const { input, refreshCalls, syncCalls } = createInput()
    input.composer.value = 'hello world'

    const actions = useChatSessionActions(input, deps)
    await actions.sendPrompt()

    expect(deps.calls.includes('submitTurn:3:hello world:anthropic:claude-opus-4-7')).toBe(true)
    expect(input.sessionState.value?.run_state).toBe('awaiting_model')
    expect(input.composer.value).toBe('')
    expect(syncCalls).toEqual(['sync'])
    expect(refreshCalls).toEqual([false])
    expect(input.sending.value).toBe(false)
  })

  test('sendPrompt shows runtime command fallback notice without submitting a turn', async () => {
    const deps = createDeps()
    const { input, refreshCalls, syncCalls } = createInput()
    input.composer.value = '/review src/app.ts'
    input.runSlashCommand = async () => ({ matched: true, command: { title: 'review', source: 'runtime-skill' } })
    input.localCommandNotice.value = 'Runtime skill /review is available in the runtime catalog, but direct execution is not wired in Agena Web yet.'

    const actions = useChatSessionActions(input, deps)
    await actions.sendPrompt()

    expect(input.composer.value).toBe('')
    expect(input.localCommandNotice.value).toBe(
      'Runtime skill /review is available in the runtime catalog, but direct execution is not wired in Agena Web yet.',
    )
    expect(refreshCalls).toEqual([])
    expect(syncCalls).toEqual([])
    expect(deps.calls.some((item) => item.startsWith('submitTurn:'))).toBe(false)
  })

  test('createSessionAction uses title, reloads sessions, and selects new session', async () => {
    const deps = createDeps()
    const { input, loadSessionsCalls, selectSessionCalls } = createInput()
    input.newSessionTitle.value = 'branch me'

    const actions = useChatSessionActions(input, deps)
    await actions.createSessionAction()

    expect(deps.calls.includes('createSession:1:branch me:')).toBe(true)
    expect(input.newSessionTitle.value).toBe('')
    expect(loadSessionsCalls).toEqual([[1, false]])
    expect(selectSessionCalls).toEqual([9])
  })

  test('forkCurrentSession uses latest message id and selects forked session', async () => {
    const deps = createDeps()
    const { input, loadSessionsCalls, selectSessionCalls } = createInput()
    input.newSessionTitle.value = 'fork title'

    const actions = useChatSessionActions(input, deps)
    await actions.forkCurrentSession()

    expect(deps.calls.includes('forkSession:3:21:fork title')).toBe(true)
    expect(input.newSessionTitle.value).toBe('')
    expect(loadSessionsCalls).toEqual([[1, false]])
    expect(selectSessionCalls).toEqual([13])
  })

  test('importSessionFromJsonl reloads sidebar, workspace sessions, and selection', async () => {
    const deps = createDeps()
    const { input, loadSidebarCalls, loadSessionsCalls, selectSessionCalls } = createInput()
    input.sessionImportJsonl.value = '{"session":15}'

    const actions = useChatSessionActions(input, deps)
    await actions.importSessionFromJsonl()

    expect(deps.calls.includes('importSessionJsonl:{"session":15}')).toBe(true)
    expect(input.sessionImportJsonl.value).toBe('')
    expect(loadSidebarCalls).toEqual(['loadSidebar'])
    expect(loadSessionsCalls).toEqual([[4, false]])
    expect(selectSessionCalls).toEqual([15])
    expect(input.localCommandNotice.value).toBe('Imported session #15.')
  })

  test('submitUserAnswers normalizes single and multiple answers', async () => {
    const deps = createDeps()
    const { input, refreshCalls, syncCalls } = createInput()
    input.sessionState.value = sessionState({
      pending_user_input_requests: [
        {
          request_id: 'req1',
          created_at: '2026-05-10T00:00:00Z',
          questions: [
            { id: 'q1', question: 'one', multiple: false },
            { id: 'q2', question: 'many', multiple: true },
          ],
        },
      ],
    })

    const actions = useChatSessionActions(input, deps)
    await actions.submitUserAnswers('req1')

    expect(deps.calls.includes('replyUserInput:3:req1:{"q1":["alpha"],"q2":["one","two"]}')).toBe(true)
    expect(syncCalls).toEqual(['sync'])
    expect(refreshCalls).toEqual([false])
  })
})
