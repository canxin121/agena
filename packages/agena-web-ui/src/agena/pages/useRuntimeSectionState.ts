import type { ComputedRef, Ref } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimePageState } from './useRuntimePageState'
import type { RuntimeRouteSection } from './runtimePageStateModel'

export type RuntimeSectionSharedState = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  loading: Ref<boolean>
  pageDescription: ComputedRef<string>
  pageTitle: ComputedRef<string>
}

export type RuntimeSectionStateDeps<TState extends RuntimeSectionSharedState> = {
  useRuntimePageState: (input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: RuntimeRouteSection
  }) => TState
}

const defaultDeps: RuntimeSectionStateDeps<ReturnType<typeof useRuntimePageState>> = {
  useRuntimePageState,
}

export function useRuntimeSectionState<
  TState extends RuntimeSectionSharedState = ReturnType<typeof useRuntimePageState>,
>(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
    section: RuntimeRouteSection
  },
  deps: RuntimeSectionStateDeps<TState> = defaultDeps as unknown as RuntimeSectionStateDeps<TState>,
) {
  const state = deps.useRuntimePageState(input)

  return {
    state,
    shared: {
      actionError: state.actionError,
      actionMessage: state.actionMessage,
      load: state.load,
      loading: state.loading,
      pageDescription: state.pageDescription,
      pageTitle: state.pageTitle,
    },
  }
}
