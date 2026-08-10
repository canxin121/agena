import { userErrorMessage } from '@/lib/api'
import type { Ref } from 'vue'

import {
  fetchSessionExecution,
  listSessions,
  reloadRuntime,
  type SessionExecutionResource,
  type SessionResource,
} from '../lib/agenaApi'
import { pickSessionId } from './runtimePageStateModel'

export type RuntimeSessionWorkflowActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sessionExecution: Ref<SessionExecutionResource | null>
  sessions: Ref<SessionResource[]>
  workflowLoading: Ref<boolean>
}

export type RuntimeSessionWorkflowActionsDeps = {
  fetchSessionExecution: typeof fetchSessionExecution
  listSessions: typeof listSessions
  pickSessionId: typeof pickSessionId
  reloadRuntime: typeof reloadRuntime
}

const defaultDeps: RuntimeSessionWorkflowActionsDeps = {
  fetchSessionExecution,
  listSessions,
  pickSessionId,
  reloadRuntime,
}

export function useRuntimeSessionWorkflowActions(
  input: RuntimeSessionWorkflowActionsInput,
  deps: RuntimeSessionWorkflowActionsDeps = defaultDeps,
) {
  async function loadSessionExecution(sessionId: number) {
    input.workflowLoading.value = true
    input.actionError.value = ''
    try {
      const execution = await deps.fetchSessionExecution(sessionId)
      if (input.selectedSessionId.value !== sessionId) return
      input.sessionExecution.value = execution
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    } finally {
      input.workflowLoading.value = false
    }
  }

  async function triggerReload() {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const result = await deps.reloadRuntime()
      input.actionMessage.value = result.started ? 'Started runtime reload.' : 'Runtime reload is already running.'
      await input.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  async function selectWorkspace(workspaceId: number) {
    input.selectedWorkspaceId.value = workspaceId
    input.sessions.value = await deps.listSessions(workspaceId)
    const nextSessionId = deps.pickSessionId(input.selectedSessionId.value, input.sessions.value)
    input.selectedSessionId.value = nextSessionId
    if (nextSessionId) {
      await loadSessionExecution(nextSessionId)
      return
    }
    input.sessionExecution.value = null
  }

  async function selectSession(sessionId: number) {
    input.selectedSessionId.value = sessionId
    await loadSessionExecution(sessionId)
  }

  return {
    loadSessionExecution,
    selectSession,
    selectWorkspace,
    triggerReload,
  }
}
