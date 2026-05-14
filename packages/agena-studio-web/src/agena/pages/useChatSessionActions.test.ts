import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { MessagePart, MessageResource, SessionExecutionResource } from '@/agena/lib/agenaApi'

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

function message(id: number, overrides?: Partial<MessageResource>): MessageResource {
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
    ...overrides,
  }
}

function createDeps(): ChatSessionActionsDeps & { calls: string[] } {
  const calls: string[] = []
  return {
    calls,
    cancelSessionTurn: async (sessionId) => {
      calls.push(`cancelSessionTurn:${sessionId}`)
      return { ok: true }
    },
    cancelUserInput: async ({ sessionId, requestId }) => {
      calls.push(`cancelUserInput:${sessionId}:${requestId}`)
      return sessionState()
    },
    continueSession: async ({ sessionId, providerId, modelId, variant }) => {
      calls.push(`continueSession:${sessionId}:${providerId || ''}:${modelId || ''}:${variant || ''}`)
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
    deleteSession: async ({ sessionId, version }) => {
      calls.push(`deleteSession:${sessionId}:${version || ''}`)
      return {
        id: sessionId,
        workspace_id: 1,
        title: 'session',
        version: version ?? 1,
        created_at: '2026-05-10T00:00:00Z',
        updated_at: '2026-05-10T00:00:00Z',
        message_count: 0,
        child_session_count: 0,
      }
    },
    exportSessionJsonl: async (sessionId) => {
      calls.push(`exportSessionJsonl:${sessionId}`)
      return '{"session":3}'
    },
    forkSession: async ({ sessionId, atMessageId, title }) => {
      calls.push(`forkSession:${sessionId}:${atMessageId || ''}:${title}`)
      return sessionState({ session: { ...sessionState().session, id: 13, title: title || 'forked' } })
    },
    getMessage: async (messageId, parts) => {
      calls.push(`getMessage:${messageId}:${parts || 'summary'}`)
      return {
        ...message(messageId),
        part_count: 1,
        parts: [
          {
            id: 200 + messageId,
            message_id: messageId,
            part_index: 0,
            status: 'complete',
            kind: 'text',
            summary: 'hello',
            has_detail: true,
            created_at: '2026-05-10T00:00:00Z',
            ...(parts === 'full' ? { content: { type: 'text', text: 'hello' } } : {}),
          },
        ],
      }
    },
    getMessagePart: async (partId) => {
      calls.push(`getMessagePart:${partId}`)
      return { id: partId, message_id: 21, part_index: 0, status: 'complete', kind: 'text', created_at: '2026-05-10T00:00:00Z', content: { type: 'text', text: 'hello' } }
    },
    importSessionJsonl: async (jsonl) => {
      calls.push(`importSessionJsonl:${jsonl}`)
      return sessionState({ session: { ...sessionState().session, id: 15, workspace_id: 4 } })
    },
    listMessageParts: async (messageId, mode) => {
      calls.push(`listMessageParts:${messageId}:${mode || 'summary'}`)
      return [
        {
          id: 200 + messageId,
          message_id: messageId,
          part_index: 0,
          status: 'complete',
          kind: 'text',
          summary: 'hello',
          has_detail: true,
          created_at: '2026-05-10T00:00:00Z',
          ...(mode === 'full' ? { content: { type: 'text', text: 'hello' } } : {}),
        },
      ]
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
    submitTurn: async ({ sessionId, text, providerId, modelId, variant }) => {
      calls.push(`submitTurn:${sessionId}:${text}:${providerId || ''}:${modelId || ''}:${variant || ''}`)
      return sessionState({ run_state: 'awaiting_model' })
    },
    unrewindSession: async ({ sessionId, messageId }) => {
      calls.push(`unrewindSession:${sessionId}:${messageId}`)
      return sessionState()
    },
    updateSession: async ({ sessionId, title, parentId, version }) => {
      calls.push(`updateSession:${sessionId}:${title}:${parentId || ''}:${version || ''}`)
      return {
        id: sessionId,
        workspace_id: 1,
        title,
        version: (version ?? 1) + 1,
        created_at: '2026-05-10T00:00:00Z',
        updated_at: '2026-05-10T00:00:00Z',
        message_count: 0,
        child_session_count: 0,
        parent_id: parentId ?? null,
      }
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
    confirm: () => true,
    composer: ref(''),
    continuing: ref(false),
    errorMessage: ref(''),
    inspectedMessage: ref<MessageResource | null>(null),
    inspectedMessageParts: ref<MessagePart[]>([]),
    inspectedPart: ref<MessagePart | null>(null),
    loading: ref(false),
    localCommandNotice: ref(''),
    messages: ref<MessageResource[]>([
      message(21, {
        part_count: 1,
        parts: [
          {
            id: 221,
            message_id: 21,
            part_index: 0,
            status: 'complete',
            kind: 'permission_request',
            summary: 'Awaiting permission: Need to inspect git status',
            has_detail: true,
            created_at: '2026-05-10T00:00:00Z',
          },
        ],
      }),
    ]),
    newSessionTitle: ref(''),
    refreshConversation: async (foreground: boolean) => {
      refreshCalls.push(foreground)
    },
    runSlashCommand: async (_inputText: string) => ({ matched: false, command: undefined }),
    selectedModelId: ref('claude-opus-4-7'),
    selectedProviderId: ref('anthropic'),
    selectedVariant: ref(''),
    selectedSessionId: ref<number | null>(3),
    selectedWorkspaceId: ref<number | null>(1),
    sending: ref(false),
    sessionImportJsonl: ref(''),
    sessionState: ref<SessionExecutionResource | null>(sessionState()),
    syncEventStream: () => {
      syncCalls.push('sync')
    },
    prompt: () => null,
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

    expect(deps.calls.includes('submitTurn:3:hello world:anthropic:claude-opus-4-7:')).toBe(true)
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

  test('inspectMessage reuses summary message data and only loads part detail on demand', async () => {
    const deps = createDeps()
    const { input } = createInput()

    const actions = useChatSessionActions(input, deps)
    await actions.inspectMessage(21)

    expect(deps.calls).toEqual(['listMessageParts:21:summary'])
    expect(input.inspectedMessage.value?.id).toBe(21)
    expect(input.inspectedMessage.value?.parts?.[0]?.content).toBe(undefined)
    expect(input.inspectedMessageParts.value[0]?.summary).toBe('hello')
    expect(input.inspectedPart.value).toBe(null)

    await actions.inspectMessage(21, 221)

    expect(deps.calls).toEqual(['listMessageParts:21:summary', 'listMessageParts:21:summary', 'getMessagePart:221'])
    expect(input.inspectedPart.value?.id).toBe(221)
    expect(input.inspectedPart.value?.content).toEqual({ type: 'text', text: 'hello' })
  })

  test('inspectMessage falls back to summary message fetch when the message is not in the conversation list', async () => {
    const deps = createDeps()
    const { input } = createInput()
    input.messages.value = []

    const actions = useChatSessionActions(input, deps)
    await actions.inspectMessage(77)

    expect(deps.calls).toEqual(['getMessage:77:summary', 'listMessageParts:77:summary'])
    expect(input.inspectedMessage.value?.id).toBe(77)
    expect(input.inspectedPart.value).toBe(null)
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

  test('renameCurrentSession updates session title and reloads workspace sessions', async () => {
    const deps = createDeps()
    const { input, loadSessionsCalls, selectSessionCalls } = createInput()
    input.sessionState.value = sessionState()
    const promptCalls: string[] = []
    input.prompt = (message, value) => {
      promptCalls.push(`${message}:${value || ''}`)
      return 'Renamed Session'
    }

    const actions = useChatSessionActions(input, deps)
    await actions.renameCurrentSession()

    expect(promptCalls).toEqual(['Rename session:session'])
    expect(deps.calls.includes('updateSession:3:Renamed Session::1')).toBe(true)
    expect(loadSessionsCalls).toEqual([[1, false]])
    expect(selectSessionCalls).toEqual([3])
    expect(input.localCommandNotice.value).toBe('Renamed session #3.')
  })

  test('deleteCurrentSession deletes the current session after confirmation', async () => {
    const deps = createDeps()
    const { input, loadSessionsCalls } = createInput()

    const actions = useChatSessionActions(input, deps)
    await actions.deleteCurrentSession()

    expect(deps.calls.includes('deleteSession:3:1')).toBe(true)
    expect(loadSessionsCalls).toEqual([[1, false]])
    expect(input.localCommandNotice.value).toBe('Deleted session #3.')
  })

  test('cancelCurrentSessionTurn requests cancellation and refreshes conversation', async () => {
    const deps = createDeps()
    const { input, refreshCalls } = createInput()

    const actions = useChatSessionActions(input, deps)
    await actions.cancelCurrentSessionTurn()

    expect(deps.calls.includes('cancelSessionTurn:3')).toBe(true)
    expect(refreshCalls).toEqual([false])
    expect(input.localCommandNotice.value).toBe('Cancellation requested for session #3.')
  })

  test('unrewindToMessage undoes a rewind and refreshes conversation', async () => {
    const deps = createDeps()
    const { input, refreshCalls } = createInput()

    const actions = useChatSessionActions(input, deps)
    await actions.unrewindToMessage(21)

    expect(deps.calls.includes('unrewindSession:3:21')).toBe(true)
    expect(refreshCalls).toEqual([true])
  })
})
