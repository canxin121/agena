import type { Ref } from 'vue'

import {
  type DomainEventRecord,
  getSessionState,
  listGlobalEvents,
  listSessions,
  listSessionTimeline,
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
  globalEvents: Ref<DomainEventRecord[]>
  sessionTimeline: Ref<DomainEventRecord[]>
  sessions: Ref<SessionResource[]>
  workflowLoading: Ref<boolean>
}

export type RuntimeSessionWorkflowActionsDeps = {
  getSessionState: typeof getSessionState
  listGlobalEvents: typeof listGlobalEvents
  listSessions: typeof listSessions
  listSessionTimeline: typeof listSessionTimeline
  pickSessionId: typeof pickSessionId
  reloadRuntime: typeof reloadRuntime
}

const defaultDeps: RuntimeSessionWorkflowActionsDeps = {
  getSessionState,
  listGlobalEvents,
  listSessions,
  listSessionTimeline,
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
      const [execution, timeline] = await Promise.all([
        deps.getSessionState(sessionId),
        deps.listSessionTimeline(sessionId, { limit: 25 }),
      ])
      if (input.selectedSessionId.value !== sessionId) return
      input.sessionExecution.value = execution
      input.sessionTimeline.value = timeline
      input.globalEvents.value = await deps.listGlobalEvents({ limit: 25 })
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.workflowLoading.value = false
    }
  }

  async function triggerReload() {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const result = await deps.reloadRuntime()
      input.actionMessage.value = `Runtime reloaded to generation ${result.generation}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
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
    input.sessionTimeline.value = []
    input.globalEvents.value = []
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
