import { onBeforeUnmount, onMounted, watch, type Ref } from 'vue'

import type { DesktopUpdateProgress } from '../../lib/desktopConfig'
import type { PluginsTab, RuntimeTab, SettingsTab } from './runtimePageStateModel'

export type RuntimeRouteLifecycleInput = {
  activePluginsTab: Ref<PluginsTab>
  activeSettingsTab: Ref<SettingsTab>
  activeTab: Ref<RuntimeTab>
  desktopEnabled: Ref<boolean>
  desktopUpdate: Ref<DesktopUpdateProgress | null>
  desktopUpdateRunning: Ref<boolean>
  load: () => Promise<void>
  loadDesktopPanel: () => Promise<void>
  loadMarketplacePanel: () => Promise<void>
  routePath: Ref<string>
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
    if (input.routeSection.value === 'settings' && tab === 'desktop' && input.desktopEnabled.value) {
      void input.loadDesktopPanel()
    }
  })

  watch(input.desktopUpdate, (update) => {
    if (update?.running) {
      input.desktopUpdateRunning.value = true
      return
    }
    input.desktopUpdateRunning.value = false
  })

  watch(
    () => [input.routePath.value, input.routeSection.value],
    () => {
      input.syncTabsFromRoute()
    },
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
    if (section === 'settings' && input.desktopEnabled.value && input.activeSettingsTab.value === 'desktop') {
      void input.loadDesktopPanel()
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
