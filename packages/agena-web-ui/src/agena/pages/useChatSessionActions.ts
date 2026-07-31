import { userErrorMessage } from '@/lib/api'
import type { Ref } from 'vue'

import {
  cancelSessionRun,
  cancelUserInput,
  compactSession,
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
  pendingPermissionRequests,
  pendingUserInputRequests,
  type MessagePart,
  type MessageResource,
  type SessionExecutionResource,
  type SessionGoalResource,
  type SessionResource,
} from '../lib/agenaApi'
import type { ComposerAttachmentDraft } from './chatAttachmentModel'
import { composerQueuePreview, createComposerQueueItem, type ComposerQueueItem } from './chatQueueModel'
import { rewindMessageComposerText } from './chatRenderModel'
import type { ComposerSkillDraft } from './chatSkillModel'

export type ChatSessionActionsInput = {
  attachments: Ref<ComposerAttachmentDraft[]>
  skillReferences: Ref<ComposerSkillDraft[]>
  composerQueue: Ref<ComposerQueueItem[]>
  queueDraining: Ref<boolean>
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
  selectedTemperature: Ref<string>
  selectedMaxOutput: Ref<string>
  selectedSystemPrompt: Ref<string>
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
  compactSession: typeof compactSession
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
  compactSession,
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

export function parseChatRunOptionValues(input: { temperature: string; maxOutput: string; system: string }) {
  const temperatureText = input.temperature.trim()
  const maxOutputText = input.maxOutput.trim()
  const temperature = temperatureText ? Number(temperatureText) : undefined
  const maxOutputTokens = maxOutputText ? Number(maxOutputText) : undefined

  if (temperature !== undefined && !Number.isFinite(temperature)) {
    throw new Error('Temperature must be a finite number.')
  }
  if (maxOutputTokens !== undefined && (!Number.isSafeInteger(maxOutputTokens) || maxOutputTokens <= 0)) {
    throw new Error('Max output tokens must be a positive whole number.')
  }

  return {
    temperature,
    maxOutputTokens,
    system: input.system.trim() || undefined,
  }
}

export function useChatSessionActions(input: ChatSessionActionsInput, deps: ChatSessionActionsDeps = defaultDeps) {
  function selectedRunOptions() {
    const providerId = input.selectedProviderId.value.trim()
    const modelId = input.selectedModelId.value.trim()
    const sampling = parseChatRunOptionValues({
      temperature: input.selectedTemperature.value,
      maxOutput: input.selectedMaxOutput.value,
      system: input.selectedSystemPrompt.value,
    })

    return {
      providerId: providerId || undefined,
      adapterId: providerId && input.selectedAdapterId.value.trim() ? input.selectedAdapterId.value.trim() : undefined,
      modelId: providerId && modelId ? modelId : undefined,
      thinkingMode:
        providerId && modelId && input.selectedThinkingMode.value.trim()
          ? input.selectedThinkingMode.value.trim()
          : undefined,
      speedMode:
        providerId && modelId && input.selectedSpeedMode.value.trim()
          ? input.selectedSpeedMode.value.trim()
          : undefined,
      verbosity:
        providerId && modelId && input.selectedVerbosity.value.trim()
          ? input.selectedVerbosity.value.trim()
          : undefined,
      parallelToolCalls:
        providerId && modelId && input.selectedParallelToolCalls.value
          ? input.selectedParallelToolCalls.value === 'true'
          : undefined,
      ...sampling,
    }
  }
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
    const request = input.sessionState.value?.pending_interactive_requests.find((item) => item.request_id === requestId)
    const sessionId = request?.session_id || input.selectedSessionId.value
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
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      input.loading.value = false
    }
  }

  async function renameCurrentSession(title?: string) {
    const session = input.sessionState.value?.session
    if (!session) return

    const nextTitle = title?.trim() || input.prompt('Rename session', session.title)?.trim() || ''
    if (!nextTitle || nextTitle === session.title) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const updated = await deps.updateSession({
        sessionId: session.id,
        title: nextTitle,
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
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      input.loading.value = false
    }
  }

  async function askAside(question: string) {
    const parent = input.sessionState.value?.session
    const workspaceId = input.selectedWorkspaceId.value
    const normalizedQuestion = question.trim()
    if (!parent || !workspaceId || !normalizedQuestion) return

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const titleText = normalizedQuestion.replace(/\s+/g, ' ').slice(0, 72)
      const child = await deps.createSession({
        workspaceId,
        parentId: parent.id,
        title: `btw: ${titleText}`,
      })
      await deps.submitTurn({
        sessionId: child.id,
        text: normalizedQuestion,
        ...selectedRunOptions(),
      })
      await input.loadSessionsForWorkspace(workspaceId, true)
      input.localCommandNotice.value = `Started aside session #${child.id}; the current session remains selected.`
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      input.loading.value = false
    }
  }

  async function submitPromptText(
    text: string,
    attachments = input.attachments.value,
    skills = input.skillReferences.value,
    clearComposerOnSuccess = true,
  ): Promise<boolean> {
    input.sending.value = true
    input.errorMessage.value = ''
    try {
      const sessionId = await ensureSessionForPrompt(text)
      if (!sessionId) return false
      const state = await deps.submitTurn({
        sessionId,
        text,
        attachments: attachments.map((attachment) => attachment.item),
        skills: skills.map((skill) => skill.item),
        ...selectedRunOptions(),
      })
      input.sessionState.value = state
      upsertSession(state.session)
      if (clearComposerOnSuccess) {
        input.composer.value = ''
        input.attachments.value = []
        input.skillReferences.value = []
      }
      input.syncEventStream()
      await input.refreshConversation(false)
      return true
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
      return false
    } finally {
      input.sending.value = false
    }
  }

  function queueCurrentPrompt(text: string) {
    const item = createComposerQueueItem(text, input.attachments.value, input.skillReferences.value)
    input.composerQueue.value = [...input.composerQueue.value, item]
    input.composer.value = ''
    input.attachments.value = []
    input.skillReferences.value = []
    input.localCommandNotice.value = `Queued message ${input.composerQueue.value.length}: ${composerQueuePreview(item)}`
  }

  function clearComposerQueue() {
    const count = input.composerQueue.value.length
    input.composerQueue.value = []
    input.localCommandNotice.value = count ? `Cleared ${count} queued message(s).` : 'The message queue is empty.'
  }

  function popComposerQueue() {
    const [item, ...rest] = input.composerQueue.value
    if (!item) {
      input.localCommandNotice.value = 'The message queue is empty.'
      return
    }
    input.composerQueue.value = rest
    input.composer.value = item.text
    input.attachments.value = item.attachments
    input.skillReferences.value = item.skills
    input.localCommandNotice.value = `Moved queued message back to the composer: ${composerQueuePreview(item)}`
  }

  async function drainComposerQueue() {
    if (input.queueDraining.value || input.sending.value || !input.composerQueue.value.length) return
    if (
      !input.sessionState.value ||
      input.sessionState.value.active_execution ||
      input.sessionState.value.workflow_state === 'blocked'
    )
      return
    const [item, ...rest] = input.composerQueue.value
    if (!item) return
    input.queueDraining.value = true
    input.composerQueue.value = rest
    try {
      const submitted = await submitPromptText(item.text, item.attachments, item.skills, false)
      if (!submitted) input.composerQueue.value = [item, ...input.composerQueue.value]
    } finally {
      input.queueDraining.value = false
    }
  }

  async function sendPrompt() {
    const text = input.composer.value.trim()
    if (!text && !input.attachments.value.length && !input.skillReferences.value.length) return

    const noticeBeforeSlash = input.localCommandNotice.value
    const slashResult =
      input.attachments.value.length || input.skillReferences.value.length
        ? { matched: false as const, command: undefined, result: undefined }
        : await input.runSlashCommand(text)
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

    if (
      input.sessionState.value &&
      (input.sessionState.value.active_execution || input.sessionState.value.workflow_state === 'blocked')
    ) {
      queueCurrentPrompt(text)
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
        ...selectedRunOptions(),
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      input.continuing.value = false
    }
  }

  async function compactCurrentSession() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.continuing.value = true
    input.errorMessage.value = ''
    try {
      input.sessionState.value = await deps.compactSession({
        sessionId,
        expectedVersion: input.sessionState.value?.session.version,
        ...selectedRunOptions(),
      })
      input.localCommandNotice.value = `Compaction started for session #${sessionId}.`
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      input.continuing.value = false
    }
  }

  async function approvePermission(
    requestId: string,
    kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
    scope?: 'session' | 'workspace' | 'global',
  ) {
    const request = pendingPermissionRequests(input.sessionState.value).find((item) => item.request_id === requestId)
    const sessionId = request?.session_id || input.selectedSessionId.value
    if (!sessionId) return
    if (!beginInteractiveRequest(sessionId, requestId)) return
    input.errorMessage.value = ''
    try {
      await deps.replyPermission({
        sessionId,
        requestId,
        kind,
        scope,
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      finishInteractiveRequest(sessionId, requestId)
    }
  }

  async function submitUserAnswers(requestId: string) {
    const request = pendingUserInputRequests(input.sessionState.value).find((item) => item.request_id === requestId)
    if (!request) return
    const sessionId = request.session_id
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
      await deps.replyUserInput({
        sessionId,
        requestId,
        answers,
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      finishInteractiveRequest(sessionId, requestId)
    }
  }

  async function cancelUserAnswers(requestId: string) {
    const request = pendingUserInputRequests(input.sessionState.value).find((item) => item.request_id === requestId)
    if (!request) return
    const sessionId = request.session_id
    if (!beginInteractiveRequest(sessionId, requestId)) return

    try {
      await deps.cancelUserInput({
        sessionId,
        requestId,
        reason: 'Cancelled from Agena',
      })
      input.syncEventStream()
      await input.refreshConversation(false)
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      finishInteractiveRequest(sessionId, requestId)
    }
  }

  async function rewindToMessage(messageId: number) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return
    if (
      !input.confirm(
        `Rewind session #${sessionId} to message #${messageId}? The retracted message will replace the current composer draft.`,
      )
    ) {
      return
    }

    input.loading.value = true
    input.errorMessage.value = ''
    try {
      const rewoundMessage = await deps.getMessage(messageId, 'full')
      const messageText = rewindMessageComposerText(rewoundMessage)
      input.sessionState.value = await deps.rewindSession({
        sessionId,
        messageId,
      })
      await input.refreshConversation(true)
      input.composer.value = messageText
      input.attachments.value = []
      input.skillReferences.value = []
      input.localCommandNotice.value = `Restored message #${messageId} to the composer.`
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      input.loading.value = false
    }
  }

  async function exportCurrentSession(requestedPath?: string) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    input.errorMessage.value = ''
    try {
      const jsonl = await deps.exportSessionJsonl(sessionId)
      input.sessionImportJsonl.value = jsonl
      const requestedFilename = requestedPath?.trim().replaceAll('\\', '/').split('/').filter(Boolean).at(-1)
      if (requestedFilename && typeof document !== 'undefined' && typeof URL !== 'undefined' && URL.createObjectURL) {
        const blob = new Blob([jsonl], { type: 'application/x-ndjson;charset=utf-8' })
        const objectUrl = URL.createObjectURL(blob)
        const link = document.createElement('a')
        link.href = objectUrl
        link.download = requestedFilename
        link.style.display = 'none'
        document.body.appendChild(link)
        link.click()
        link.remove()
        window.setTimeout(() => URL.revokeObjectURL(objectUrl), 0)
      }
      input.localCommandNotice.value = requestedFilename
        ? `Exported session #${sessionId} as ${requestedFilename}.`
        : `Exported session #${sessionId} into the Session Transfer panel.`
    } catch (err) {
      input.errorMessage.value = userErrorMessage(err)
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
      input.errorMessage.value = userErrorMessage(err)
    } finally {
      input.loading.value = false
    }
  }

  return {
    cancelCurrentSessionRun,
    approvePermission,
    askAside,
    cancelUserAnswers,
    clearSessionGoalAction,
    clearComposerQueue,
    compactCurrentSession,
    completeSessionGoalAction,
    continueCurrentSession,
    createSessionAction,
    deleteCurrentSession,
    drainComposerQueue,
    exportCurrentSession,
    forkCurrentSession,
    importSessionFromJsonl: importSessionFromJsonlAction,
    inspectMessage,
    isInteractiveRequestBusy,
    popComposerQueue,
    renameCurrentSession,
    resolveWorkspaceAction,
    rewindToMessage,
    sendPrompt,
    setSessionGoalAction,
    showSessionGoalAction,
    submitUserAnswers,
  }
}
