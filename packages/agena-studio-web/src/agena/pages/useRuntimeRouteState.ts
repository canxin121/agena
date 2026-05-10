import type { Ref } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import {
  buildRuntimeSectionPath,
  isPluginsTab,
  isRuntimeTab,
  isSettingsTab,
  resolveRuntimeTabFromRoute,
  sanitizeRuntimeSectionQuery,
  type PluginsTab,
  type RuntimeRouteSection,
  type RuntimeTab,
  type SettingsTab,
} from './runtimePageStateModel'

export type RuntimeRouteStateInput = {
  activePluginsTab: Ref<PluginsTab>
  activeSettingsTab: Ref<SettingsTab>
  activeTab: Ref<RuntimeTab>
  routePath: Ref<string>
  routeQuery: RouteLocationNormalizedLoaded['query']
  routeSection: Ref<RuntimeRouteSection>
}

export type RuntimeRouteStateDeps = {
  router: Pick<Router, 'replace'>
}

export function useRuntimeRouteState(input: RuntimeRouteStateInput, deps: RuntimeRouteStateDeps) {
  function syncTabsFromRoute() {
    const normalized = resolveRuntimeTabFromRoute(input.routePath.value, input.routeQuery, input.routeSection.value)
    if (input.routeSection.value === 'runtime' && isRuntimeTab(normalized)) {
      input.activeTab.value = normalized
    }
    if (input.routeSection.value === 'settings' && isSettingsTab(normalized)) {
      input.activeSettingsTab.value = normalized
    }
    if (input.routeSection.value === 'plugins' && isPluginsTab(normalized)) {
      input.activePluginsTab.value = normalized
    }
  }

  async function updateRoutePath(tab: string) {
    const nextPath = buildRuntimeSectionPath(input.routeSection.value, tab)
    await deps.router.replace({ path: nextPath, query: sanitizeRuntimeSectionQuery(input.routeQuery) })
  }

  return {
    syncTabsFromRoute,
    updateRoutePath,
  }
}
