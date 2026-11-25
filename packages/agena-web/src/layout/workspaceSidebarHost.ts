import { inject, provide, shallowRef, type InjectionKey, type ShallowRef } from 'vue'

export const WORKSPACE_SIDEBAR_HOST_ID = 'workspace-sidebar-host'
export const WORKSPACE_SIDEBAR_PANEL_HOST_ID = 'workspace-sidebar-panel-host'

const hostKey: InjectionKey<ShallowRef<HTMLElement | null>> = Symbol('workspace-sidebar-host')

export function provideWorkspaceSidebarHost() {
  provide(hostKey, shallowRef(null))
}

export function useWorkspaceSidebarHost() {
  return inject(hostKey, null)
}
