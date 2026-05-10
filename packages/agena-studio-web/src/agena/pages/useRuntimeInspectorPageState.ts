import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeInspectorsState } from './useRuntimeInspectorsState'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type RuntimeInspectorPageStateSource = {
  filteredLspServers: Parameters<typeof useRuntimeInspectorsState>[0]['filteredLspServers']
  filteredMcpServers: Parameters<typeof useRuntimeInspectorsState>[0]['filteredMcpServers']
  lspQuery: Parameters<typeof useRuntimeInspectorsState>[0]['lspQuery']
  mcpQuery: Parameters<typeof useRuntimeInspectorsState>[0]['mcpQuery']
  openRuntimeConfigRoot: Parameters<typeof useRuntimeInspectorsState>[0]['openRuntimeConfigRoot']
  openWorkspacePath: Parameters<typeof useRuntimeInspectorsState>[0]['openWorkspacePath']
  openWorkspaceShortcut: Parameters<typeof useRuntimeInspectorsState>[0]['openWorkspaceShortcut']
  runtime: Parameters<typeof useRuntimeInspectorsState>[0]['runtime']
}

export type RuntimeInspectorPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'runtime'
  }) => {
    shared: RuntimeSectionSharedState
    state: RuntimeInspectorPageStateSource
  }
}

const defaultDeps: RuntimeInspectorPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & RuntimeInspectorPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: RuntimeInspectorPageStateSource
    },
}

export function createRuntimeInspectorPanelState(state: RuntimeInspectorPageStateSource) {
  return useRuntimeInspectorsState({
    filteredLspServers: state.filteredLspServers,
    filteredMcpServers: state.filteredMcpServers,
    lspQuery: state.lspQuery,
    mcpQuery: state.mcpQuery,
    openRuntimeConfigRoot: state.openRuntimeConfigRoot,
    openWorkspacePath: state.openWorkspacePath,
    openWorkspaceShortcut: state.openWorkspaceShortcut,
    runtime: state.runtime,
  })
}

export function useRuntimeInspectorPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: RuntimeInspectorPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'runtime' })
  const inspectors = createRuntimeInspectorPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    inspectors,
  }
}
