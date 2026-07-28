import type { ComputedRef, Ref } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import type { SettingsTab, SectionTabOption } from './runtimePageStateModel'
import { useRuntimeSectionState, type RuntimeSectionSharedState } from './useRuntimeSectionState'

export type SettingsSectionShellStateSource = {
  activeSettingsTab: Ref<SettingsTab>
  visibleTabs: ComputedRef<SectionTabOption[]>
}

export type SettingsSectionShellStateDeps = {
  useRuntimeSectionState: (input: { route: RouteLocationNormalizedLoaded; router: Router; section: 'settings' }) => {
    shared: RuntimeSectionSharedState
    state: SettingsSectionShellStateSource
  }
}

const defaultDeps: SettingsSectionShellStateDeps = {
  useRuntimeSectionState: (input) =>
    useRuntimeSectionState<{ [key: string]: unknown } & RuntimeSectionSharedState & SettingsSectionShellStateSource>(
      input,
    ) as {
      shared: RuntimeSectionSharedState
      state: SettingsSectionShellStateSource
    },
}

export function createSettingsSectionShellState(state: SettingsSectionShellStateSource) {
  return {
    activeTab: state.activeSettingsTab,
    tabs: state.visibleTabs,
  }
}

export function useSettingsSectionShellState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: SettingsSectionShellStateDeps = defaultDeps,
) {
  const { shared, state } = deps.useRuntimeSectionState({ ...input, section: 'settings' })
  const shell = createSettingsSectionShellState(state)

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
