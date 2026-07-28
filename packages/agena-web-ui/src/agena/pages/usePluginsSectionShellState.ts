import type { ComputedRef, Ref } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import type { PluginsTab, SectionTabOption } from './runtimePageStateModel'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type PluginsSectionShellStateSource = {
  activePluginsTab: Ref<PluginsTab>
  visibleTabs: ComputedRef<SectionTabOption[]>
}

export type PluginsSectionShellStateDeps = {
  useRuntimeSectionState: (input: { route: RouteLocationNormalizedLoaded; router: Router; section: 'plugins' }) => {
    shared: RuntimeSectionSharedState
    state: PluginsSectionShellStateSource
  }
}

const defaultDeps: PluginsSectionShellStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & PluginsSectionShellStateSource>(
      input,
    ) as {
      shared: RuntimeSectionSharedState
      state: PluginsSectionShellStateSource
    },
}

export function createPluginsSectionShellState(state: PluginsSectionShellStateSource) {
  return {
    activeTab: state.activePluginsTab,
    tabs: state.visibleTabs,
  }
}

export function usePluginsSectionShellState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: PluginsSectionShellStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'plugins' })
  const shell = createPluginsSectionShellState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    shell,
  }
}
