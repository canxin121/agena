import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeOverviewState } from './useRuntimeOverviewState'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type RuntimeOverviewPageStateSource = {
  catalogEntries: Parameters<typeof useRuntimeOverviewState>[0]['catalogEntries']
  operatorCards: Parameters<typeof useRuntimeOverviewState>[0]['operatorCards']
  providerModels: Parameters<typeof useRuntimeOverviewState>[0]['providerModels']
  providers: Parameters<typeof useRuntimeOverviewState>[0]['providers']
  runtime: Parameters<typeof useRuntimeOverviewState>[0]['runtime']
  runtimeSnapshotFacts: Parameters<typeof useRuntimeOverviewState>[0]['runtimeSnapshotFacts']
  sessionCacheFacts: Parameters<typeof useRuntimeOverviewState>[0]['sessionCacheFacts']
}

export type RuntimeOverviewPageStateDeps = {
  useRuntimeSectionState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: 'runtime'
  }) => {
    shared: RuntimeSectionSharedState
    state: RuntimeOverviewPageStateSource
  }
}

const defaultDeps: RuntimeOverviewPageStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & RuntimeOverviewPageStateSource>(input) as {
      shared: RuntimeSectionSharedState
      state: RuntimeOverviewPageStateSource
    },
}

export function createRuntimeOverviewPanelState(state: RuntimeOverviewPageStateSource) {
  return useRuntimeOverviewState({
    catalogEntries: state.catalogEntries,
    operatorCards: state.operatorCards,
    providerModels: state.providerModels,
    providers: state.providers,
    runtime: state.runtime,
    runtimeSnapshotFacts: state.runtimeSnapshotFacts,
    sessionCacheFacts: state.sessionCacheFacts,
  })
}

export function useRuntimeOverviewPageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: RuntimeOverviewPageStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'runtime' })
  const overview = createRuntimeOverviewPanelState(state)

  return {
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    loading: shared.loading,
    overview,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
  }
}
