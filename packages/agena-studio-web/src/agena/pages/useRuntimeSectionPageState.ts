import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { createRuntimeInspectorPanelState } from './useRuntimeInspectorPageState'
import { createRuntimeOperatorPanelState } from './useRuntimeOperatorPageState'
import { createRuntimeOverviewPanelState } from './useRuntimeOverviewPageState'
import { createRuntimeSectionShellState } from './useRuntimeSectionShellState'
import { useRuntimeSectionState } from './useRuntimeSectionState'
import { createRuntimeSkillsPanelState } from './useRuntimeSkillsPageState'
import { createRuntimeWorkflowPanelState } from './useRuntimeWorkflowPageState'
import { useSectionPanelRegistry } from './useSectionPanelRegistry'

export function useRuntimeSectionPageState(input: {
  route: RouteLocationNormalizedLoaded
  router: Router
}) {
  const { shared, state } = useRuntimeSectionState({ ...input, section: 'runtime' })

  const shell = createRuntimeSectionShellState({
    activeTab: state.activeTab,
    triggerReload: state.triggerReload,
    visibleTabs: state.visibleTabs,
  })

  const overview = createRuntimeOverviewPanelState({
    operatorCards: state.operatorCards,
    providerModels: state.providerModels,
    providers: state.providers,
    runtime: state.runtime,
    runtimeSnapshotFacts: state.runtimeSnapshotFacts,
    sessionCacheFacts: state.sessionCacheFacts,
  })

  const workflow = createRuntimeWorkflowPanelState({
    approvePermission: state.approvePermission,
    executionFacts: state.executionFacts,
    openSelectedSessionInChat: state.openSelectedSessionInChat,
    selectSession: state.selectSession,
    selectWorkspace: state.selectWorkspace,
    selectedSessionId: state.selectedSessionId,
    selectedWorkspaceId: state.selectedWorkspaceId,
    sessionExecution: state.sessionExecution,
    sessions: state.sessions,
    timelineSummaries: state.timelineSummaries,
    workflowLoading: state.workflowLoading,
    workspaces: state.workspaces,
  })

  const inspectors = createRuntimeInspectorPanelState({
    filteredLspServers: state.filteredLspServers,
    filteredMcpServers: state.filteredMcpServers,
    lspQuery: state.lspQuery,
    mcpQuery: state.mcpQuery,
    openRuntimeConfigRoot: state.openRuntimeConfigRoot,
    openWorkspacePath: state.openWorkspacePath,
    openWorkspaceShortcut: state.openWorkspaceShortcut,
    runtime: state.runtime,
  })

  const skills = createRuntimeSkillsPanelState({
    discoveredSkills: state.discoveredSkills,
    filteredDiscoveredSkills: state.filteredDiscoveredSkills,
    filteredSkillCommands: state.filteredSkillCommands,
    openPluginLogsWorkspacePath: state.openPluginLogsWorkspacePath,
    openRuntimeConfigRoot: state.openRuntimeConfigRoot,
    openRuntimeEntryInChat: state.openRuntimeEntryInChat,
    openRuntimeEntrySource: state.openRuntimeEntrySource,
    openWorkspaceShortcut: state.openWorkspaceShortcut,
    runtimeSkillQuery: state.runtimeSkillQuery,
    skillCommands: state.skillCommands,
  })

  const operator = createRuntimeOperatorPanelState({
    runtime: state.runtime,
  })

  const panelRegistry = useSectionPanelRegistry({
    activeTab: shell.activeTab,
    panels: {
      overview,
      workflow,
      mcp: inspectors,
      lsp: inspectors,
      skills,
      operator,
    },
  })

  return {
    activeTab: shell.activeTab,
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    inspectors,
    load: shared.load,
    loading: shared.loading,
    operator,
    overview,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    panels: panelRegistry.panels,
    currentPanel: panelRegistry.currentPanel,
    skills,
    triggerReload: shell.triggerReload,
    visibleTabs: shell.visibleTabs,
    workflow,
  }
}
