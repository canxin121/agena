import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'
import { useRuntimeSkillsState } from './useRuntimeSkillsState'

export type RuntimeSkillsPageStateSource = {
  discoveredSkills: Parameters<typeof useRuntimeSkillsState>[0]['discoveredSkills']
  filteredDiscoveredSkills: Parameters<typeof useRuntimeSkillsState>[0]['filteredDiscoveredSkills']
  filteredSkillCommands: Parameters<typeof useRuntimeSkillsState>[0]['filteredSkillCommands']
  openPluginLogsWorkspacePath: Parameters<typeof useRuntimeSkillsState>[0]['openPluginLogsWorkspacePath']
  openRuntimeConfigRoot: Parameters<typeof useRuntimeSkillsState>[0]['openRuntimeConfigRoot']
  openRuntimeEntryInChat: Parameters<typeof useRuntimeSkillsState>[0]['openRuntimeEntryInChat']
  openRuntimeEntrySource: Parameters<typeof useRuntimeSkillsState>[0]['openRuntimeEntrySource']
  runtimeSkillQuery: Parameters<typeof useRuntimeSkillsState>[0]['runtimeSkillQuery']
  skillCommands: Parameters<typeof useRuntimeSkillsState>[0]['skillCommands']
}

export type RuntimeSkillsPageStateDeps = {
  useRuntimeSectionState: (input: { route: RouteLocationNormalizedLoaded; router: Router; section: 'runtime' }) => {
    shared: RuntimeSectionSharedState
    state: RuntimeSkillsPageStateSource
  }
}

const defaultDeps: RuntimeSkillsPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & RuntimeSkillsPageStateSource>(
      input,
    ) as {
      shared: RuntimeSectionSharedState
      state: RuntimeSkillsPageStateSource
    },
}

export function createRuntimeSkillsPanelState(state: RuntimeSkillsPageStateSource) {
  return useRuntimeSkillsState({
    discoveredSkills: state.discoveredSkills,
    filteredDiscoveredSkills: state.filteredDiscoveredSkills,
    filteredSkillCommands: state.filteredSkillCommands,
    openPluginLogsWorkspacePath: state.openPluginLogsWorkspacePath,
    openRuntimeConfigRoot: state.openRuntimeConfigRoot,
    openRuntimeEntryInChat: state.openRuntimeEntryInChat,
    openRuntimeEntrySource: state.openRuntimeEntrySource,
    runtimeSkillQuery: state.runtimeSkillQuery,
    skillCommands: state.skillCommands,
  })
}

export function useRuntimeSkillsPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: RuntimeSkillsPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'runtime' })
  const skills = createRuntimeSkillsPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    skills,
  }
}
