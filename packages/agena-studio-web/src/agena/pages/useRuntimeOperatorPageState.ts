import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeOperatorState } from './useRuntimeOperatorState'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type RuntimeOperatorPageStateSource = {
  runtime: Parameters<typeof useRuntimeOperatorState>[0]['runtime']
}

export type RuntimeOperatorPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'runtime'
  }) => {
    shared: RuntimeSectionSharedState
    state: RuntimeOperatorPageStateSource
  }
}

const defaultDeps: RuntimeOperatorPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & RuntimeOperatorPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: RuntimeOperatorPageStateSource
    },
}

export function createRuntimeOperatorPanelState(state: RuntimeOperatorPageStateSource) {
  return useRuntimeOperatorState({
    runtime: state.runtime,
  })
}

export function useRuntimeOperatorPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: RuntimeOperatorPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'runtime' })
  const operator = createRuntimeOperatorPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    operator,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
  }
}
