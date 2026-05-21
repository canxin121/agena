import type { ComputedRef, Ref } from 'vue'

import type {
  PermissionRequest,
  SessionExecutionResource,
  SessionResource,
  TimelineEventRecord,
  WorkspaceResource,
} from '../lib/agenaApi'
import type { SessionExecutionFact, TimelineSummaryItem } from './runtimePageModel'

export type RuntimeWorkflowStateInput = {
  approvePermission: (
    requestId: string,
    kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
    scope?: 'session' | 'workspace' | 'global',
  ) => void | Promise<void>
  editPermissionRequest: (request: PermissionRequest) => void
  executionFacts: ComputedRef<SessionExecutionFact[]>
  openSelectedSessionInChat: () => void
  selectSession: (sessionId: number) => void | Promise<void>
  selectWorkspace: (workspaceId: number) => void | Promise<void>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sessionExecution: Ref<SessionExecutionResource | null>
  sessions: Ref<SessionResource[]>
  globalEventSummaries: ComputedRef<TimelineSummaryItem[]>
  timelineSummaries: ComputedRef<TimelineSummaryItem[]>
  workflowLoading: Ref<boolean>
  workspaces: Ref<WorkspaceResource[]>
  sessionTimeline?: Ref<TimelineEventRecord[]>
}

export function useRuntimeWorkflowState(input: RuntimeWorkflowStateInput) {
  return {
    approvePermission: input.approvePermission,
    editPermissionRequest: input.editPermissionRequest,
    executionFacts: input.executionFacts,
    openSelectedSessionInChat: input.openSelectedSessionInChat,
    selectSession: input.selectSession,
    selectWorkspace: input.selectWorkspace,
    selectedSessionId: input.selectedSessionId,
    selectedWorkspaceId: input.selectedWorkspaceId,
    sessionExecution: input.sessionExecution,
    sessions: input.sessions,
    globalEventSummaries: input.globalEventSummaries,
    timelineSummaries: input.timelineSummaries,
    workflowLoading: input.workflowLoading,
    workspaces: input.workspaces,
  }
}
