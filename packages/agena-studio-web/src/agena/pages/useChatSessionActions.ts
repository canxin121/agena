import type { Ref } from 'vue'

import {
  cancelUserInput,
  continueSession,
  createSession,
  createWorkspace,
  exportSessionJsonl,
  forkSession,
  importSessionJsonl,
  replyPermission,
  replyUserInput,
  resolveWorkspace,
  rewindSession,
  submitTurn,
  type MessageResource,
  type SessionExecutionResource,
} from '../lib/agenaApi'

export type ChatSessionActionsInput = {
  composer: Ref<string>
  continuing: Ref<boolean>
  errorMessage: Ref<string>
  loading: Ref<boolean>
  localCommandNotice: Ref<string>
  messages: Ref<MessageResource[]>
  newSessionTitle: Ref<string>
  refreshConversation: (foreground: boolean) => Promise<void>
  runSlashCommand: (inputText: string) => Promise<{
    matched: boolean
    command?: { title: string; source?: 'navigation' | 'runtime-skill' | 'runtime-command' | 'chat-action' | 'workspace-action' }
  }>
  selectedModelId: Ref<string>
  selectedProviderId: Ref<string>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sending: Ref<boolean>
  sessionImportJsonl: Ref<string>
  sessionState: Ref<SessionExecutionResource | null>
  syncEventStream: () => void
  userInputDrafts: Record<string, Record<string, string>>
  workspacePath: Ref<string>
  loadSidebar: () => Promise<void>
  loadSessionsForWorkspace: (workspaceId: number, preserveSelection?: boolean) => Promise<void>
  selectSession: (sessionId: number) => Promise<void>
  selectWorkspace: (workspaceId: number) => Promise<void>
}

export type ChatSessionActionsDeps = {
  cancelUserInput: typeof cancelUserInput
  continueSession: typeof continueSession
  createSession: typeof createSession
  createWorkspace: typeof createWorkspace
  exportSessionJsonl: typeof exportSessionJsonl
  forkSession: typeof forkSession
  importSessionJsonl: typeof importSessionJsonl
  replyPermission: typeof replyPermission
  replyUserInput: typeof replyUserInput
  resolveWorkspace: typeof resolveWorkspace
  rewindSession: typeof rewindSession
  submitTurn: typeof submitTurn
}

const defaultDeps: ChatSessionActionsDeps = {
  cancelUserInput,
  continueSession,
  createSession,
  createWorkspace,
  exportSessionJsonl,
  forkSession,
  importSessionJsonl,
  replyPermission,
  replyUserInput,
  resolveWorkspace,
  rewindSession,
  submitTurn,
}

export function useChatSessionActions(input: ChatSessionActionsInput, deps: ChatSessionActionsDeps = defaultDeps) {
  async function resolveWorkspaceAction(createIfMissing: boolean) {
    const path = input.workspacePath.value.trim()
    if (!path) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const workspace = createIfMissing ? await deps.resolveWorkspace(path, true) : await deps.createWorkspace(path)
      input.workspacePath.value = workspace.path
      await input.loadSidebar()
      await input.selectWorkspace(workspace.id)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function createSessionAction(parentId?: number | null) {
    const workspaceId = input.selectedWorkspaceId.value
    if (!workspaceId) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const fallbackTitle = parentId ? `Child of #${parentId}` : 'New session'
      const title = input.newSessionTitle.value.trim() || fallbackTitle
      const session = await deps.createSession({
        workspaceId,
        title,
        parentId: parentId ?? undefined,
      })
      input.newSessionTitle.value = ''
      await input.loadSessionsForWorkspace(workspaceId, false)
      await input.selectSession(session.id)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function forkCurrentSession() {
    const sessionId = input.selectedSessionId.value
    const workspaceId = input.selectedWorkspaceId.value
    const latestMessageId = input.messages.value.at(-1)?.id
    if (!sessionId || !workspaceId) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const fallbackTitle = `Fork of #${sessionId}`
      const execution = await deps.forkSession({
        sessionId,
        ...(latestMessageId != null ? { atMessageId: latestMessageId } : {}),
        title: input.newSessionTitle.value.trim() || fallbackTitle,
      })
      input.newSessionTitle.value = ''
      await input.loadSessionsForWorkspace(workspaceId, false)
      await input.selectSession(execution.session.id)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function sendPrompt() {
    const sessionId = input.selectedSessionId.value
    const text = input.composer.value.trim()
    if (!sessionId || !text) return

    const slashResult = await input.runSlashCommand(text)
    if (slashResult.matched) {
      input.composer.value = ''
      if (slashResult.command?.source === 'runtime-command' || slashResult.command?.source === 'runtime-skill') {
        input.localCommandNotice.value =
          input.localCommandNotice.value ||
          `Direct execution for ${slashResult.command.title} is not available in Agena Web yet.`
      } else {
        input.localCommandNotice.value = `Executed ${slashResult.command?.title || text}`
      }
      return
    }

    input.sending.value = true
    input.errorMessage.value = ''
    try {
      const state = await deps.submitTurn({
        sessionId,
        text,
        providerId: input.selectedProviderId.value || undefined,
        modelId: input.selectedProviderId.value && input.selectedModelId.value ? input.selectedModelId.value : undefined,
      })
      input.sessionState.value = state
      input.composer.value = ''
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.sending.value = false
    }
  }

  async function continueCurrentSession() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.continuing.value = true
    input.errorMessage.value = ''
    try {
      input.sessionState.value = await deps.continueSession({
        sessionId,
        providerId: input.selectedProviderId.value || undefined,
        modelId: input.selectedProviderId.value && input.selectedModelId.value ? input.selectedModelId.value : undefined,
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.continuing.value = false
    }
  }

  async function approvePermission(
    requestId: string,
    kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
    scope?: 'session' | 'workspace' | 'global',
  ) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return
    input.errorMessage.value = ''
    try {
      input.sessionState.value = await deps.replyPermission({
        sessionId,
        requestId,
        kind,
        scope,
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function submitUserAnswers(requestId: string) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    const request = input.sessionState.value?.pending_user_input_requests.find((item) => item.request_id === requestId)
    if (!request) return

    const answers: Record<string, string[]> = {}
    const draft = input.userInputDrafts[requestId] || {}
    for (const question of request.questions) {
      const raw = String(draft[question.id] || '').trim()
      if (!raw) continue
      answers[question.id] = question.multiple
        ? raw
            .split(',')
            .map((item) => item.trim())
            .filter(Boolean)
        : [raw]
    }

    try {
      input.sessionState.value = await deps.replyUserInput({
        sessionId,
        requestId,
        answers,
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function cancelUserAnswers(requestId: string) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    try {
      input.sessionState.value = await deps.cancelUserInput({
        sessionId,
        requestId,
        reason: 'Cancelled from Agena Studio',
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function rewindToMessage(messageId: number) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return
    if (typeof window !== 'undefined' && !window.confirm(`Rewind session #${sessionId} to message #${messageId}?`)) {
      return
    }

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      input.sessionState.value = await deps.rewindSession({
        sessionId,
        messageId,
      })
      await input.refreshConversation(true)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function exportCurrentSession() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.errorMessage.value = ''
    try {
      const jsonl = await deps.exportSessionJsonl(sessionId)
      input.sessionImportJsonl.value = jsonl
      input.localCommandNotice.value = `Exported session #${sessionId}.`
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function importSessionFromJsonlAction() {
    const jsonl = input.sessionImportJsonl.value.trim()
    if (!jsonl) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const execution = await deps.importSessionJsonl(jsonl)
      input.sessionImportJsonl.value = ''
      await input.loadSidebar()
      await input.loadSessionsForWorkspace(execution.session.workspace_id, false)
      await input.selectSession(execution.session.id)
      input.localCommandNotice.value = `Imported session #${execution.session.id}.`
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  return {
    approvePermission,
    cancelUserAnswers,
    continueCurrentSession,
    createSessionAction,
    exportCurrentSession,
    forkCurrentSession,
    importSessionFromJsonl: importSessionFromJsonlAction,
    resolveWorkspaceAction,
    rewindToMessage,
    sendPrompt,
    submitUserAnswers,
  }
}
