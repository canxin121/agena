import { onBeforeUnmount, onMounted, watch, type Ref } from 'vue'
import type { RouteLocationNormalizedLoaded } from 'vue-router'

import type { PluginsTab, RuntimeTab, SettingsTab } from './runtimePageStateModel'

export type RuntimeRouteLifecycleInput = {
  activePluginsTab: Ref<PluginsTab>
  activeSettingsTab: Ref<SettingsTab>
  activeTab: Ref<RuntimeTab>
  load: () => Promise<void>
  loadMarketplacePanel: () => Promise<void>
  routePath: Ref<string>
  routeQuery: Ref<RouteLocationNormalizedLoaded['query']>
  routeSection: Ref<'runtime' | 'settings' | 'plugins'>
  stopPluginLogPolling: () => void
  syncPluginLogPolling: () => void
  syncTabsFromRoute: () => void
  updateRoutePath: (tab: string) => Promise<void>
}

export type RuntimeRouteLifecycleOptions = {
  registerComponentLifecycle?: boolean
}

export function useRuntimeRouteLifecycle(
  input: RuntimeRouteLifecycleInput,
  options: RuntimeRouteLifecycleOptions = {},
) {
  const registerComponentLifecycle = options.registerComponentLifecycle !== false
  watch(input.activeTab, (tab) => {
    input.stopPluginLogPolling()
    if (input.routeSection.value === 'runtime') {
      void input.updateRoutePath(tab)
    }
  })

  watch(input.activePluginsTab, (tab) => {
    if (input.routeSection.value !== 'plugins') return
    void input.updateRoutePath(tab)
    if (tab === 'installed') {
      input.syncPluginLogPolling()
      return
    }
    input.stopPluginLogPolling()
    void input.loadMarketplacePanel()
  })

  watch(input.activeSettingsTab, (tab) => {
    if (input.routeSection.value === 'settings') {
      void input.updateRoutePath(tab)
    }
  })

  watch(
    () => [input.routePath.value, input.routeSection.value, input.routeQuery.value],
    () => {
      input.syncTabsFromRoute()
    },
    { deep: true },
  )

  // Settings uses the session query as part of its data source. A query-only
  // navigation keeps the same component instance alive, so route path
  // watchers alone would leave the editor bound to the previous session.
  watch(
    input.routeQuery,
    () => {
      void input.load()
    },
    { deep: true },
  )

  watch(input.routeSection, (section) => {
    input.syncTabsFromRoute()
    if (section === 'plugins') {
      input.syncPluginLogPolling()
      if (input.activePluginsTab.value === 'marketplace') {
        void input.loadMarketplacePanel()
      }
    } else {
      input.stopPluginLogPolling()
    }
  })

  if (registerComponentLifecycle) {
    onMounted(() => {
      input.syncTabsFromRoute()
      void input.load()
    })

    onBeforeUnmount(() => {
      input.stopPluginLogPolling()
    })
  }
}
