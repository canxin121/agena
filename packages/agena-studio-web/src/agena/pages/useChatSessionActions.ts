import type { Ref } from 'vue'

import {
  cancelSessionRun,
  cancelUserInput,
  continueSession,
  clearSessionGoal,
  completeSessionGoal,
  createSession,
  deleteSession,
  createWorkspace,
  exportSessionJsonl,
  forkSession,
  getSessionGoal,
  getMessage,
  getMessagePart,
  importSessionJsonl,
  listMessageParts,
  replyPermission,
  replyUserInput,
  resolveWorkspace,
  rewindSession,
  setSessionGoal,
  submitTurn,
  updateSession,
  type MessagePart,
  type MessageResource,
  type SessionExecutionResource,
  type SessionGoalResource,
  type SessionResource,
} from '../lib/agenaApi'

export type ChatSessionActionsInput = {
  confirm: (message: string) => boolean
  composer: Ref<string>
  continuing: Ref<boolean>
  errorMessage: Ref<string>
  interactiveRequestInFlight: Record<string, boolean>
  loading: Ref<boolean>
  localCommandNotice: Ref<string>
  inspectedMessage: Ref<MessageResource | null>
  inspectedMessageParts: Ref<MessagePart[]>
  inspectedPart: Ref<MessagePart | null>
  messages: Ref<MessageResource[]>
  newSessionTitle: Ref<string>
  refreshConversation: (foreground: boolean) => Promise<void>
  runSlashCommand: (inputText: string) => Promise<{
    matched: boolean
    command?: {
      title: string
      source?: 'navigation' | 'runtime-skill' | 'runtime-command' | 'plugin-studio' | 'chat-action' | 'workspace-action'
    }
    result?: { submitText?: string; notice?: string }
  }>
  selectedAdapterId: Ref<string>
  selectedModelId: Ref<string>
  selectedProviderId: Ref<string>
  selectedThinkingMode: Ref<string>
  selectedSpeedMode: Ref<string>
  selectedVerbosity: Ref<string>
  selectedParallelToolCalls: Ref<string>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sending: Ref<boolean>
  sessionImportJsonl: Ref<string>
  sessionState: Ref<SessionExecutionResource | null>
  sessions: Ref<SessionResource[]>
  syncEventStream: () => void
  prompt: (message: string, defaultValue?: string) => string | null
  userInputDrafts: Record<string, Record<string, string>>
  workspacePath: Ref<string>
  loadSidebar: () => Promise<void>
  loadSessionsForWorkspace: (workspaceId: number, preserveSelection?: boolean) => Promise<void>
  selectSession: (sessionId: number) => Promise<void>
  selectWorkspace: (workspaceId: number) => Promise<void>
}

export type ChatSessionActionsDeps = {
  cancelSessionRun: typeof cancelSessionRun
  cancelUserInput: typeof cancelUserInput
  clearSessionGoal: typeof clearSessionGoal
  completeSessionGoal: typeof completeSessionGoal
  continueSession: typeof continueSession
  createSession: typeof createSession
  createWorkspace: typeof createWorkspace
  deleteSession: typeof deleteSession
  exportSessionJsonl: typeof exportSessionJsonl
  forkSession: typeof forkSession
  getSessionGoal: typeof getSessionGoal
  getMessage: typeof getMessage
  getMessagePart: typeof getMessagePart
  importSessionJsonl: typeof importSessionJsonl
  listMessageParts: typeof listMessageParts
  replyPermission: typeof replyPermission
  replyUserInput: typeof replyUserInput
  resolveWorkspace: typeof resolveWorkspace
  rewindSession: typeof rewindSession
  setSessionGoal: typeof setSessionGoal
  submitTurn: typeof submitTurn
  updateSession: typeof updateSession
}

const defaultDeps: ChatSessionActionsDeps = {
  cancelSessionRun,
  cancelUserInput,
  clearSessionGoal,
  completeSessionGoal,
  continueSession,
  createSession,
  createWorkspace,
  deleteSession,
  exportSessionJsonl,
  forkSession,
  getSessionGoal,
  getMessage,
  getMessagePart,
  importSessionJsonl,
  listMessageParts,
  replyPermission,
  replyUserInput,
  resolveWorkspace,
  rewindSession,
  setSessionGoal,
  submitTurn,
  updateSession,
}

function formatGoalNotice(goal: SessionGoalResource): string {
  return `Goal #${goal.id} ${goal.status}: ${goal.objective}`
}

export function useChatSessionActions(input: ChatSessionActionsInput, deps: ChatSessionActionsDeps = defaultDeps) {
  function interactiveRequestKey(sessionId: number, requestId: string): string {
    return `${sessionId}:${requestId}`
  }

  function beginInteractiveRequest(sessionId: number, requestId: string): boolean {
    const key = interactiveRequestKey(sessionId, requestId)
    if (input.interactiveRequestInFlight[key]) return false
    input.interactiveRequestInFlight[key] = true
    return true
  }

  function finishInteractiveRequest(sessionId: number, requestId: string) {
    delete input.interactiveRequestInFlight[interactiveRequestKey(sessionId, requestId)]
  }

  function isInteractiveRequestBusy(requestId: string): boolean {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return false
    return !!input.interactiveRequestInFlight[interactiveRequestKey(sessionId, requestId)]
  }

  function patchCurrentGoal(goal: SessionGoalResource | null) {
    if (!input.sessionState.value) return
    input.sessionState.value = {
      ...input.sessionState.value,
      goal,
      session: {
        ...input.sessionState.value.session,
        goal,
      },
    }
  }

  function deriveSessionTitle(text: string): string {
    const title = text.replace(/\s+/g, ' ').trim()
    if (!title) return 'New session'
    return title.length > 72 ? `${title.slice(0, 69)}...` : title
  }

  function upsertSession(session: SessionResource) {
    input.sessions.value = [session, ...input.sessions.value.filter((item) => item.id !== session.id)]
  }

  async function ensureSessionForPrompt(text: string): Promise<number | null> {
    const existingSessionId = input.selectedSessionId.value
    if (existingSessionId) return existingSessionId

    const workspaceId = input.selectedWorkspaceId.value
    if (!workspaceId) {
      input.errorMessage.value = 'Select or create a workspace before sending a prompt.'
      return null
    }

    const session = await deps.createSession({
      workspaceId,
      title: input.newSessionTitle.value.trim() || deriveSessionTitle(text),
    })
    input.newSessionTitle.value = ''
    upsertSession(session)
    input.selectedSessionId.value = session.id
    await input.selectSession(session.id)
    return session.id
  }

  async function inspectMessage(messageId: number, partId?: number) {
    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const existingMessage = input.messages.value.find((message) => message.id === messageId) || null
      const messagePromise = existingMessage ? Promise.resolve(existingMessage) : deps.getMessage(messageId, 'summary')
      const partsPromise = deps.listMessageParts(messageId, 'summary')
      const [message, parts] = await Promise.all([messagePromise, partsPromise])
      input.inspectedMessage.value = message
      input.inspectedMessageParts.value = parts
      input.inspectedPart.value = partId == null ? null : await deps.getMessagePart(partId)
      input.localCommandNotice.value = `Loaded message #${messageId} inspector.`
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

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
      input.localCommandNotice.value = `Created session #${session.id}.`
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function showSessionGoalAction() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const goal =
        input.sessionState.value?.goal ??
        input.sessionState.value?.session.goal ??
        (await deps.getSessionGoal(sessionId))
      patchCurrentGoal(goal ?? null)
      input.localCommandNotice.value = goal ? formatGoalNotice(goal) : `Session #${sessionId} has no active goal.`
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function setSessionGoalAction(objective: string) {
    const sessionId = input.selectedSessionId.value
    const trimmedObjective = objective.trim()
    if (!sessionId || !trimmedObjective) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const goal = await deps.setSessionGoal({ sessionId, objective: trimmedObjective })
      patchCurrentGoal(goal)
      input.localCommandNotice.value = `Set ${formatGoalNotice(goal)}`
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function completeSessionGoalAction() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const goal = await deps.completeSessionGoal(sessionId)
      patchCurrentGoal(goal)
      input.localCommandNotice.value = `Completed ${formatGoalNotice(goal)}`
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function clearSessionGoalAction() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      await deps.clearSessionGoal(sessionId)
      patchCurrentGoal(null)
      input.localCommandNotice.value = `Cleared goal for session #${sessionId}.`
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function renameCurrentSession() {
    const session = input.sessionState.value?.session
    if (!session) return

    const nextTitle = input.prompt('Rename session', session.title)?.trim() ?? ''
    if (!nextTitle || nextTitle === session.title) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const updated = await deps.updateSession({
        sessionId: session.id,
        title: nextTitle,
        parentId: session.parent_id ?? null,
        version: session.version,
      })
      if (input.sessionState.value) {
        input.sessionState.value = {
          ...input.sessionState.value,
          session: {
            ...input.sessionState.value.session,
            ...updated,
          },
        }
      }
      await input.loadSessionsForWorkspace(updated.workspace_id, false)
      await input.selectSession(updated.id)
      input.localCommandNotice.value = `Renamed session #${updated.id}.`
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

  async function deleteCurrentSession() {
    const session = input.sessionState.value?.session
    const workspaceId = input.selectedWorkspaceId.value
    if (!session || !workspaceId) return
    if (!input.confirm(`Delete session #${session.id} (${session.title})?`)) {
      return
    }

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      await deps.deleteSession({
        sessionId: session.id,
        version: session.version,
      })
      input.localCommandNotice.value = `Deleted session #${session.id}.`
      await input.loadSessionsForWorkspace(workspaceId, false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  async function submitPromptText(text: string) {
    input.sending.value = true
    input.errorMessage.value = ''
    try {
      const sessionId = await ensureSessionForPrompt(text)
      if (!sessionId) return
      const state = await deps.submitTurn({
        sessionId,
        text,
        providerId: input.selectedProviderId.value || undefined,
        adapterId:
          input.selectedProviderId.value && input.selectedAdapterId.value ? input.selectedAdapterId.value : undefined,
        modelId:
          input.selectedProviderId.value && input.selectedModelId.value ? input.selectedModelId.value : undefined,
        thinkingMode:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedThinkingMode.value
            ? input.selectedThinkingMode.value
            : undefined,
        speedMode:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedSpeedMode.value
            ? input.selectedSpeedMode.value
            : undefined,
        verbosity:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedVerbosity.value
            ? input.selectedVerbosity.value
            : undefined,
        parallelToolCalls:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedParallelToolCalls.value
            ? input.selectedParallelToolCalls.value === 'true'
            : undefined,
      })
      input.sessionState.value = state
      upsertSession(state.session)
      input.composer.value = ''
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.sending.value = false
    }
  }

  async function sendPrompt() {
    const text = input.composer.value.trim()
    if (!text) return

    const noticeBeforeSlash = input.localCommandNotice.value
    const slashResult = await input.runSlashCommand(text)
    if (slashResult.matched) {
      input.composer.value = ''
      if (slashResult.result?.submitText) {
        await submitPromptText(slashResult.result.submitText)
        return
      }
      if (slashResult.result?.notice) {
        input.localCommandNotice.value = slashResult.result.notice
        return
      }
      const commandNoticeChanged =
        input.localCommandNotice.value && input.localCommandNotice.value !== noticeBeforeSlash
      if (!commandNoticeChanged) {
        input.localCommandNotice.value = `Executed ${slashResult.command?.title || text}`
      }
      return
    }

    await submitPromptText(text)
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
        adapterId:
          input.selectedProviderId.value && input.selectedAdapterId.value ? input.selectedAdapterId.value : undefined,
        modelId:
          input.selectedProviderId.value && input.selectedModelId.value ? input.selectedModelId.value : undefined,
        thinkingMode:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedThinkingMode.value
            ? input.selectedThinkingMode.value
            : undefined,
        speedMode:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedSpeedMode.value
            ? input.selectedSpeedMode.value
            : undefined,
        verbosity:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedVerbosity.value
            ? input.selectedVerbosity.value
            : undefined,
        parallelToolCalls:
          input.selectedProviderId.value && input.selectedModelId.value && input.selectedParallelToolCalls.value
            ? input.selectedParallelToolCalls.value === 'true'
            : undefined,
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.continuing.value = false
    }
  }

  async function cancelCurrentSessionRun() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.continuing.value = true
    input.errorMessage.value = ''
    try {
      await deps.cancelSessionRun(sessionId)
      input.localCommandNotice.value = `Cancellation requested for session #${sessionId}.`
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
    if (!beginInteractiveRequest(sessionId, requestId)) return
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
    } finally {
      finishInteractiveRequest(sessionId, requestId)
    }
  }

  async function submitUserAnswers(requestId: string) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    const request = input.sessionState.value?.pending_user_input_requests.find((item) => item.request_id === requestId)
    if (!request) return
    if (!beginInteractiveRequest(sessionId, requestId)) return

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
    } finally {
      finishInteractiveRequest(sessionId, requestId)
    }
  }

  async function cancelUserAnswers(requestId: string) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return
    if (!beginInteractiveRequest(sessionId, requestId)) return

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
    } finally {
      finishInteractiveRequest(sessionId, requestId)
    }
  }

  async function rewindToMessage(messageId: number) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return
    if (!input.confirm(`Rewind session #${sessionId} to message #${messageId}?`)) {
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
    cancelCurrentSessionRun,
    approvePermission,
    cancelUserAnswers,
    clearSessionGoalAction,
    completeSessionGoalAction,
    continueCurrentSession,
    createSessionAction,
    deleteCurrentSession,
    exportCurrentSession,
    forkCurrentSession,
    importSessionFromJsonl: importSessionFromJsonlAction,
    inspectMessage,
    isInteractiveRequestBusy,
    renameCurrentSession,
    resolveWorkspaceAction,
    rewindToMessage,
    sendPrompt,
    setSessionGoalAction,
    showSessionGoalAction,
    submitUserAnswers,
  }
}
