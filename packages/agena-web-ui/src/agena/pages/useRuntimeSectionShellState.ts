import type { ComputedRef, Ref } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import type { RuntimeTab, SectionTabOption } from './runtimePageStateModel'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type RuntimeSectionShellStateSource = {
  activeTab: Ref<RuntimeTab>
  triggerReload: () => void | Promise<void>
  visibleTabs: ComputedRef<SectionTabOption[]>
}

export type RuntimeSectionShellStateDeps = {
  useRuntimeSectionState: (input: { route: RouteLocationNormalizedLoaded; router: Router; section: 'runtime' }) => {
    shared: RuntimeSectionSharedState
    state: RuntimeSectionShellStateSource
  }
}

const defaultDeps: RuntimeSectionShellStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & RuntimeSectionShellStateSource>(
      input,
    ) as {
      shared: RuntimeSectionSharedState
      state: RuntimeSectionShellStateSource
    },
}

export function createRuntimeSectionShellState(state: RuntimeSectionShellStateSource) {
  return {
    activeTab: state.activeTab,
    triggerReload: state.triggerReload,
    visibleTabs: state.visibleTabs,
  }
}

export function useRuntimeSectionShellState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: RuntimeSectionShellStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'runtime' })
  const shell = createRuntimeSectionShellState(state)

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
