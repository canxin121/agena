import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useRuntimeWorkflowState } from './useRuntimeWorkflowState'

export type RuntimeWorkflowPageStateSource = {
  approvePermission: Parameters<typeof useRuntimeWorkflowState>[0]['approvePermission']
  executionFacts: Parameters<typeof useRuntimeWorkflowState>[0]['executionFacts']
  openSelectedSessionInChat: Parameters<typeof useRuntimeWorkflowState>[0]['openSelectedSessionInChat']
  selectSession: Parameters<typeof useRuntimeWorkflowState>[0]['selectSession']
  selectWorkspace: Parameters<typeof useRuntimeWorkflowState>[0]['selectWorkspace']
  selectedSessionId: Parameters<typeof useRuntimeWorkflowState>[0]['selectedSessionId']
  selectedWorkspaceId: Parameters<typeof useRuntimeWorkflowState>[0]['selectedWorkspaceId']
  sessionExecution: Parameters<typeof useRuntimeWorkflowState>[0]['sessionExecution']
  sessions: Parameters<typeof useRuntimeWorkflowState>[0]['sessions']
  globalEventSummaries: Parameters<typeof useRuntimeWorkflowState>[0]['globalEventSummaries']
  timelineSummaries: Parameters<typeof useRuntimeWorkflowState>[0]['timelineSummaries']
  workflowLoading: Parameters<typeof useRuntimeWorkflowState>[0]['workflowLoading']
  workspaces: Parameters<typeof useRuntimeWorkflowState>[0]['workspaces']
}

export type RuntimeWorkflowPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'runtime'
  }) => {
    shared: RuntimeSectionSharedState
    state: RuntimeWorkflowPageStateSource
  }
}

const defaultDeps: RuntimeWorkflowPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & RuntimeWorkflowPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: RuntimeWorkflowPageStateSource
    },
}

export function createRuntimeWorkflowPanelState(state: RuntimeWorkflowPageStateSource) {
  return useRuntimeWorkflowState({
    approvePermission: state.approvePermission,
    executionFacts: state.executionFacts,
    openSelectedSessionInChat: state.openSelectedSessionInChat,
    selectSession: state.selectSession,
    selectWorkspace: state.selectWorkspace,
    selectedSessionId: state.selectedSessionId,
    selectedWorkspaceId: state.selectedWorkspaceId,
    sessionExecution: state.sessionExecution,
    sessions: state.sessions,
    globalEventSummaries: state.globalEventSummaries,
    timelineSummaries: state.timelineSummaries,
    workflowLoading: state.workflowLoading,
    workspaces: state.workspaces,
  })
}

export function useRuntimeWorkflowPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: RuntimeWorkflowPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'runtime' })
  const workflow = createRuntimeWorkflowPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    workflow,
  }
}
